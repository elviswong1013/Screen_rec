//! WAV 写与崩溃恢复（REQ-19）。
//!
//! - **流式写**：44 字节头部占位 + s16 数据追加；正常停止 `finalize` 封口
//!   （RIFF/data 长度回填）。未封口文件即崩溃残留形态——[`repair_wav`] 的目标场景。
//! - **格式**：输出率（M15/REQ-1：44.1k/48k/88.2k/96k）s16/立体声（管线的 s16
//!   归档；帧 = 4 字节）。写入方负责把捕获样本重采样到该率。
//! - **[`repair_wav`]**：头部重写（RIFF/data 长度封口）+ 尾部不完整样本裁切；
//!   **幂等**（二次调用无操作，`trimmed_bytes=0`、文件不变）。
//! - **RIFF 4GiB 上限**：数据段超过 u32 编码上限时写入明确失败（超限分段策略属
//!   后续里程碑；不静默截断）。
//!
//! 契约（测试即契约，docs/milestones/m6-red-report.md §3.2）：
//! `WavRepair { frames: u64（修复后完整帧数，s16 立体声）, trimmed_bytes: u64（0..3） }`。

use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// 单样本字节数（s16）。
const SAMPLE_BYTES: u64 = 2;
/// PCM WAV 固定头部字节数：RIFF(12) + fmt(24) + data(8)。
const HEADER_BYTES: u64 = 44;

/// WAV 写/修复错误（变体实现自定；测试仅 Debug 打印）。
#[derive(Debug)]
pub enum WavError {
    /// 文件 I/O 失败。
    Io { detail: String },
    /// 不是合法的 RIFF/WAVE 结构。
    NotWav { detail: String },
    /// 数据段超过 RIFF u32 编码上限（4GiB 策略：明确失败，不静默截断）。
    TooLarge { detail: String },
}

/// 崩溃修复结果。
#[derive(Debug, Clone)]
pub struct WavRepair {
    /// 修复后完整帧数（s16 立体声；frames/头部采样率 ≈ 文件时长）。
    pub frames: u64,
    /// 尾部不完整样本裁切字节数（0..3）。
    pub trimmed_bytes: u64,
}

fn io_err(e: std::io::Error) -> WavError {
    WavError::Io {
        detail: e.to_string(),
    }
}

fn f32_to_s16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// PCM WAV 头部（44 字节；`data_bytes` 为数据段长度）。
fn wav_header(data_bytes: u32, sample_rate: u32, channels: u16) -> [u8; HEADER_BYTES as usize] {
    let block_align = channels * SAMPLE_BYTES as u16;
    let byte_rate = sample_rate * block_align as u32;
    let mut h = [0u8; HEADER_BYTES as usize];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36u32.saturating_add(data_bytes)).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    h[22..24].copy_from_slice(&channels.to_le_bytes());
    h[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    h[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    h[32..34].copy_from_slice(&block_align.to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes()); // bits per sample
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    h
}

/// 流式 WAV 写（输出率/s16/立体声；`finalize` 前文件为「未封口」崩溃残留形态）。
pub struct WavWriter {
    file: File,
    path: PathBuf,
    sample_rate: u32,
    channels: u16,
    data_bytes: u64,
}

impl WavWriter {
    /// 创建并写入头部占位（未封口；正常停止路径须调用 [`Self::finalize`]）。
    pub fn create(path: &Path, sample_rate: u32, channels: u16) -> Result<Self, WavError> {
        let mut file = File::create(path).map_err(io_err)?;
        let header = wav_header(0, sample_rate, channels);
        file.write_all(&header).map_err(io_err)?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
            sample_rate,
            channels,
            data_bytes: 0,
        })
    }

    /// 追加交错 f32 样本（→ s16 LE）。
    pub fn write_f32_interleaved(&mut self, samples: &[f32]) -> Result<(), WavError> {
        let mut buf = Vec::with_capacity(samples.len() * SAMPLE_BYTES as usize);
        for s in samples {
            buf.extend_from_slice(&f32_to_s16(*s).to_le_bytes());
        }
        self.append(&buf)
    }

    /// 追加 `frames` 帧静音（每帧 = channels 个零样本）。
    pub fn write_silence_frames(&mut self, frames: u64) -> Result<(), WavError> {
        let mut remaining = frames * self.channels as u64 * SAMPLE_BYTES;
        let zeros = [0u8; 8192];
        while remaining > 0 {
            let n = remaining.min(zeros.len() as u64) as usize;
            self.append(&zeros[..n])?;
            remaining -= n as u64;
        }
        Ok(())
    }

    /// 封口：回填 RIFF/data 长度（正常停止路径；「停止即成品」）。
    pub fn finalize(&mut self) -> Result<(), WavError> {
        if self.data_bytes > u32::MAX as u64 {
            return Err(WavError::TooLarge {
                detail: format!(
                    "WAV 数据段 {} 字节超过 RIFF u32 上限（{}）",
                    self.data_bytes,
                    self.path.display()
                ),
            });
        }
        let header = wav_header(self.data_bytes as u32, self.sample_rate, self.channels);
        self.file.seek(SeekFrom::Start(0)).map_err(io_err)?;
        self.file.write_all(&header).map_err(io_err)?;
        self.file.flush().map_err(io_err)?;
        Ok(())
    }

    fn append(&mut self, bytes: &[u8]) -> Result<(), WavError> {
        let new_total = self.data_bytes + bytes.len() as u64;
        if new_total >= u32::MAX as u64 {
            return Err(WavError::TooLarge {
                detail: format!(
                    "WAV 数据段将超过 RIFF u32 上限（{new_total} 字节；{}）",
                    self.path.display()
                ),
            });
        }
        self.file.write_all(bytes).map_err(io_err)?;
        self.data_bytes = new_total;
        Ok(())
    }
}

/// 崩溃修复：头部重写（RIFF/data 长度封口）+ 尾部不完整样本裁切；**幂等**
/// （二次调用无操作，`trimmed_bytes=0`、文件不变）。
///
/// 崩溃残留 = 数据已追加而头部长度仍为占位的半成品：数据段长度以**实际文件
/// 长度**为准（数据段为末段），裁切 `payload % block_align` 帧对齐；帧数与
/// block_align 从 `fmt ` 块读取（缺省 s16 立体声 = 4 字节）。
pub fn repair_wav(path: &Path) -> Result<WavRepair, WavError> {
    let bytes = std::fs::read(path).map_err(io_err)?;
    if bytes.len() < HEADER_BYTES as usize || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(WavError::NotWav {
            detail: format!("{} 不是 RIFF/WAVE 文件", path.display()),
        });
    }

    // 扫描 chunk 链：定位 fmt（block_align）与 data（数据起点 + 长度字段偏移）。
    let mut pos = 12usize;
    let mut block_align: u64 = 4; // 缺省：s16 立体声帧
    let mut data_start: Option<usize> = None;
    let mut data_size_off: Option<usize> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]) as usize;
        if id == b"fmt " {
            if size >= 14 && pos + 8 + 14 <= bytes.len() {
                let ba = u16::from_le_bytes([bytes[pos + 20], bytes[pos + 21]]) as u64;
                if ba > 0 {
                    block_align = ba;
                }
            }
        } else if id == b"data" {
            data_start = Some(pos + 8);
            data_size_off = Some(pos + 4);
            break;
        }
        let Some(next) = pos.checked_add(8 + size) else {
            break;
        };
        if next <= pos {
            break;
        }
        pos = next;
    }
    let (Some(data_start), Some(data_size_off)) = (data_start, data_size_off) else {
        return Err(WavError::NotWav {
            detail: format!("{} 缺少 data 块（非标准 WAV）", path.display()),
        });
    };

    // 数据段长度以实际文件长度为准（崩溃残留：头部长度为占位）。
    let payload_len = (bytes.len() - data_start) as u64;
    let trimmed_bytes = payload_len % block_align;
    let sealed_len = bytes.len() as u64 - trimmed_bytes;
    let frames = (payload_len - trimmed_bytes) / block_align;
    if sealed_len > u32::MAX as u64 {
        return Err(WavError::TooLarge {
            detail: format!(
                "{} 超过 RIFF u32 编码上限（{sealed_len} 字节）",
                path.display()
            ),
        });
    }
    let sealed_riff = (sealed_len - 8) as u32;
    let sealed_data = (payload_len - trimmed_bytes) as u32;

    // 幂等：已封口且帧对齐 ⇒ 无操作（不改字节、不触碰文件）。
    let cur_riff = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let cur_data = u32::from_le_bytes([
        bytes[data_size_off],
        bytes[data_size_off + 1],
        bytes[data_size_off + 2],
        bytes[data_size_off + 3],
    ]);
    if trimmed_bytes == 0 && cur_riff == sealed_riff && cur_data == sealed_data {
        return Ok(WavRepair {
            frames,
            trimmed_bytes: 0,
        });
    }

    let mut f = OpenOptions::new().write(true).open(path).map_err(io_err)?;
    if trimmed_bytes > 0 {
        f.set_len(sealed_len).map_err(io_err)?;
    }
    f.seek(SeekFrom::Start(4)).map_err(io_err)?;
    f.write_all(&sealed_riff.to_le_bytes()).map_err(io_err)?;
    f.seek(SeekFrom::Start(data_size_off as u64))
        .map_err(io_err)?;
    f.write_all(&sealed_data.to_le_bytes()).map_err(io_err)?;
    f.flush().map_err(io_err)?;
    Ok(WavRepair {
        frames,
        trimmed_bytes,
    })
}
