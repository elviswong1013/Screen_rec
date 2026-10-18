//! wasapi 音频采集（REQ-14/15/16/17/18；S3 spike 实证流程的工程化）。
//!
//! 采集核（[`run_capture_core`]）在**专用 MTA 线程**内完成 COM 初始化、按
//! [`CaptureTarget`] 激活采集客户端（进程环回 `new_application_loopback_client`
//! 含 INCLUDE 树语义；设备环回=默认渲染端点 + LOOPBACK 流旗标；麦克风=采集端点）、
//! 分源格式协商（REQ-7：System/Mic 取设备原生采样率，Process 固定 48k；均
//! f32/立体声 + autoconvert）、事件驱动取包，并经
//! [`SampleSlotPacer`](crate::slots::SampleSlotPacer) 编排输出（静音补齐/迟到
//! 裁切/墙钟续流/时长钉齐）。内存 API（[`capture_process_loopback`] /
//! [`capture_device_loopback`] / [`capture_microphone`]）与流式 API
//! （[`LoopbackStream`]，供路由管道写）复用同一采集核，行为不分叉；实际捕获
//! 格式经 `StreamMsg::Ready` 上报（REQ-8）。
//!
//! 身份前置校验（REQ-14/D9，激活**之前**执行、快速失败）：
//! PID 存在性（E_TARGET_LOST）、系统宿主根硬性禁止（HostForbidden）、
//! 创建时间吻合（IdentityAmbiguous，PID 复用防护）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use wasapi::{AudioClient, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

use crate::slots::{PacketPlan, SampleSlotPacer};

/// 管线回退采样率（REQ-18：设备原生率查询失败时回退 48k）。
pub const PIPELINE_SAMPLE_RATE: u32 = 48_000;
/// 管线固定声道数（REQ-18：立体声；解码固定双声道四字节，原生声道数会越界）。
pub const PIPELINE_CHANNELS: u16 = 2;

/// 实际捕获格式（REQ-8：`StreamMsg::Ready` 上报；路由据此构造重采样器）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

/// 分源捕获格式策略（REQ-7；`queried_mix_rate` 为可注入的 `GetMixFormat` 结果）：
/// - `Process`：进程环回不可查询，固定 48000/f32/双声道 + autoconvert（忽略注入）；
/// - `DeviceLoopback`/`Microphone`：查询成功 → 按原生**采样率**请求（仍固定 f32/
///   双声道）；查询失败（`None`）→ 回退 48000 + autoconvert，不阻断录制。
pub fn capture_format_strategy(
    target: &CaptureTarget,
    queried_mix_rate: Option<u32>,
) -> CaptureFormat {
    let sample_rate = match target {
        CaptureTarget::Process { .. } => PIPELINE_SAMPLE_RATE,
        CaptureTarget::DeviceLoopback | CaptureTarget::Microphone { .. } => {
            queried_mix_rate.unwrap_or(PIPELINE_SAMPLE_RATE)
        }
    };
    CaptureFormat {
        sample_rate,
        channels: PIPELINE_CHANNELS,
    }
}

/// 同步捕获时长钉齐的目标帧数（REQ-8：以**实际捕获率**为域，不得固定 48k）。
pub fn sync_target_frames(duration: Duration, sample_rate: u32) -> u64 {
    (duration.as_secs_f64() * sample_rate as f64).round() as u64
}

/// §3.4 AudioSourceSpec（M4 用例仅构造 App；System/Mic 为契约记载、M6 实现）。
#[derive(Debug, Clone)]
pub enum AudioSourceSpec {
    App { pid: u32, created_at_unix_ms: u64 },
    System,
    Mic { device: String },
}

/// M6：采集目标（进程环回 / 默认渲染端点设备环回 / 麦克风；同一采集核）。
#[derive(Debug, Clone)]
pub enum CaptureTarget {
    /// REQ-14：进程环回（INCLUDE_TARGET_PROCESS_TREE 由激活参数承载）。
    Process { pid: u32 },
    /// REQ-15：默认渲染端点设备环回（整机混音；v1 仅默认端点）。
    DeviceLoopback,
    /// REQ-16：采集（麦克风）设备；`device` = [`AudioDeviceInfo::id`]。
    Microphone { device: String },
}

/// REQ-16：采集（麦克风）设备信息。
#[derive(Debug, Clone)]
pub struct AudioDeviceInfo {
    /// 请求捕获时回传（wasapi 端点 id，稳定）。
    pub id: String,
    /// 友好名（报告/UI）。
    pub name: String,
    /// 默认采集端点标识。
    pub is_default: bool,
}

/// 采集错误。测试钉 5 变体（花括号模式容忍字段）；`Timeout` 为契约记载。
#[derive(Debug)]
pub enum CaptureError {
    /// E_TARGET_LOST：PID 不存在（激活前校验）。
    TargetLost {
        detail: String,
    },
    /// E_IDENTITY_AMBIGUOUS：创建时间不符/PID 复用。
    IdentityAmbiguous {
        detail: String,
    },
    /// REQ-14：系统宿主根硬性禁止。
    HostForbidden {
        detail: String,
    },
    /// E_LOOPBACK_UNAVAILABLE（默认端点/进程环回不可用 → L2 记 BLOCKED）。
    LoopbackUnavailable {
        detail: String,
    },
    /// M6/REQ-16：无采集设备/设备不存在或不可打开——明确不可用（不得静默零包/静默成功）。
    MicUnavailable {
        detail: String,
    },
    Timeout,
    Failed {
        detail: String,
    },
}

/// REQ-18/26 记账（绿阶段事件/UI 呈现可扩展字段；测试读三字段）。
#[derive(Debug, Clone, Default)]
pub struct CaptureStats {
    /// 收到的 WASAPI 包数（诊断：与补齐/裁切量对照可判「丢包」还是「内容静音」）。
    pub packets: u64,
    /// 静音补齐帧数。
    pub padded_frames: u64,
    /// 迟到/裁切丢弃帧数。
    pub dropped_frames: u64,
    /// DATA_DISCONTINUITY 包数。
    pub discontinuities: u64,
}

/// 诊断开关（`SR_DIAG_AV=1`）：采集/写线程停止时把统计打到 stderr。
///
/// 默认关闭且不改变任何行为（仅多做几次整数累加）；用于同步/漂移类间歇问题的
/// 归因（区分「采集侧丢包/补齐」与「源内容停滞」）。见
/// `docs/milestones/m12-h-report.md`。
pub fn diag_enabled() -> bool {
    matches!(std::env::var("SR_DIAG_AV").as_deref(), Ok("1") | Ok("true"))
}

/// 进程环回捕获结果（交错立体声 f32；`sample_rate` = 实际捕获率）。
#[derive(Debug, Clone)]
pub struct CapturedPcm {
    /// 交错立体声 L,R,L,R,…（帧数 = samples.len()/2）。
    pub samples: Vec<f32>,
    /// 实际捕获率（REQ-8；设备原生率或回退 48000）。
    pub sample_rate: u32,
    pub channels: u16,
    pub stats: CaptureStats,
}

/// REQ-14：系统宿主根（捕获根为这些映像名时硬性禁止——宿主泄漏=变相录系统混音）。
const FORBIDDEN_HOSTS: [&str; 5] = [
    "explorer.exe",
    "svchost.exe",
    "applicationframehost.exe",
    "runtimebroker.exe",
    "sihost.exe",
];

/// 创建时间比对容差（地面真值读取与系统时钟粒度余量；±5 分钟偏差必拒）。
const CREATED_TOLERANCE_MS: i64 = 1_500;

// ─────────────────────────── 身份前置校验（激活前，快速失败） ───────────────────────────

fn process_exists(pid: u32) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(h) => {
                let _ = CloseHandle(h);
                true
            }
            Err(_) => false,
        }
    }
}

fn process_image_name(pid: u32) -> Option<String> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok =
            QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
                .is_ok();
        let _ = CloseHandle(h);
        if ok {
            Some(String::from_utf16_lossy(&buf[..len as usize]))
        } else {
            None
        }
    }
}

/// 进程创建时间（UNIX ms；与测试侧地面真值同源 GetProcessTimes）。
pub fn process_created_unix_ms(pid: u32) -> Option<u64> {
    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut created = FILETIME::default();
        let mut exit_t = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let ok = GetProcessTimes(h, &mut created, &mut exit_t, &mut kernel, &mut user).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return None;
        }
        let raw = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
        // FILETIME（1601-01-01 起 100ns）→ UNIX ms：epoch 偏移 116_444_736_000_000_000
        // ×100ns（= 369 年），再除以 10⁴。
        Some(raw.saturating_sub(116_444_736_000_000_000) / 10_000)
    }
}

/// REQ-14/D9 身份前置校验（激活之前；全部路径快速返回，不得触及采集）。
pub fn check_app_identity(pid: u32, created_at_unix_ms: u64) -> Result<(), CaptureError> {
    if pid == 0 || !process_exists(pid) {
        return Err(CaptureError::TargetLost {
            detail: format!("pid={pid} 不存在（激活前校验）"),
        });
    }
    if let Some(image) = process_image_name(pid) {
        let name = image
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(&image)
            .to_lowercase();
        if FORBIDDEN_HOSTS.iter().any(|h| *h == name) {
            return Err(CaptureError::HostForbidden {
                detail: format!("捕获根为系统宿主 {name}（pid={pid}）：REQ-14 硬性禁止"),
            });
        }
    }
    if let Some(actual) = process_created_unix_ms(pid) {
        let diff = (actual as i64 - created_at_unix_ms as i64).abs();
        if diff > CREATED_TOLERANCE_MS {
            return Err(CaptureError::IdentityAmbiguous {
                detail: format!("pid={pid} 创建时间不符（期望 {created_at_unix_ms}，实际 {actual}，差 {diff}ms）：PID 复用防护"),
            });
        }
    }
    Ok(())
}

// ─────────────────────────── QPC 时基（WASAPI 包时间戳同域） ───────────────────────────

/// QPC → 100ns 单位（WASAPI 包时间戳同域；SampleSlotPacer 输入）。
fn qpc_100ns() -> i64 {
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
    unsafe {
        let mut freq = 0i64;
        let mut count = 0i64;
        let _ = QueryPerformanceFrequency(&mut freq);
        let _ = QueryPerformanceCounter(&mut count);
        if freq <= 0 {
            return 0;
        }
        ((count as i128) * 10_000_000i128 / freq as i128) as i64
    }
}

// ─────────────────────────── 采集核（专用 MTA 线程内执行） ───────────────────────────

/// 包时间戳连续化 + 样本槽编排（采集核内部记账）。
struct CaptureTimeline {
    pacer: SampleSlotPacer,
    /// 期望的下一包时间戳（100ns；= 上一包（连续化后）ts + frames×步长）。
    ts_cursor: Option<i64>,
    rate: u32,
}

/// 包时间戳量化抖动容忍窗（1.5ms）：引擎将包时间戳舍入到毫秒级网格时，相邻包
/// ts 与「上一包 ts + 帧数步长」的偏差在窗内 ⇒ 视为连续（取期望值，消除每包
/// 0–1ms 的幻像补齐/裁切咔哒声）；超出窗（真实断流/迟到）⇒ 用实际 ts，由样本槽
/// 按 REQ-18 补齐/裁切（S3 §3.3 的 ts 记账同思路）。
const TS_QUANTUM_100NS: i64 = 15_000;

impl CaptureTimeline {
    fn new(rate: u32) -> Self {
        Self {
            pacer: SampleSlotPacer::new(rate),
            ts_cursor: None,
            rate,
        }
    }

    fn on_packet(&mut self, raw_ts: i64, frames: usize, silent: bool) -> PacketPlan {
        let ts = match self.ts_cursor {
            Some(cursor) => {
                if (raw_ts - cursor).abs() <= TS_QUANTUM_100NS {
                    cursor
                } else {
                    raw_ts
                }
            }
            None => raw_ts,
        };
        self.ts_cursor = Some(ts + (frames as i64) * 10_000_000i64 / self.rate as i64);
        self.pacer.on_packet(ts, frames, silent)
    }

    /// 墙钟续流（REQ-18）：委托 pacer 的停滞判定——仅距上一真实包 ≥ 阈值才补。
    fn pad_to_wall_clock(&mut self, now_100ns: i64) -> u64 {
        self.pacer.pad_to_wall_clock(now_100ns)
    }

    fn pad_to(&mut self, total_frames: u64) -> u64 {
        self.pacer.pad_to(total_frames)
    }
}

/// 结束条件：内存 API=录满请求时长并把时长钉到精确帧数；流式=直到 stop。
#[derive(Clone, Copy)]
enum EndMode {
    Duration { wall: Duration },
    UntilStopped,
}

/// 按采集目标创建/激活 AudioClient（专用 MTA 线程内调用）。
/// 设备环回 = 默认渲染端点 + **LOOPBACK 流旗标**（wasapi 客户端方向=Render、
/// 流方向=Capture 时自动置位，S3 apploop `--device-loopback` 同构）；麦克风 =
/// 采集端点；错误变体按目标分流（设备环回 → LoopbackUnavailable，麦克风 →
/// MicUnavailable——「无设备时明确不可用」，不得静默）。
fn create_capture_client(target: &CaptureTarget) -> Result<AudioClient, CaptureError> {
    match target {
        CaptureTarget::Process { pid } => AudioClient::new_application_loopback_client(*pid, true)
            .map_err(|e| CaptureError::LoopbackUnavailable {
                detail: format!("进程环回激活失败（pid={pid}）: {e:?}"),
            }),
        CaptureTarget::DeviceLoopback => {
            let enumerator =
                DeviceEnumerator::new().map_err(|e| CaptureError::LoopbackUnavailable {
                    detail: format!("音频设备枚举器创建失败: {e:?}"),
                })?;
            let device = enumerator
                .get_default_device(&Direction::Render)
                .map_err(|e| CaptureError::LoopbackUnavailable {
                    detail: format!("默认渲染端点不可用: {e:?}"),
                })?;
            device
                .get_iaudioclient()
                .map_err(|e| CaptureError::LoopbackUnavailable {
                    detail: format!("默认渲染端点激活失败: {e:?}"),
                })
        }
        CaptureTarget::Microphone { device } => {
            let enumerator = DeviceEnumerator::new().map_err(|e| CaptureError::MicUnavailable {
                detail: format!("音频设备枚举器创建失败: {e:?}"),
            })?;
            let dev = enumerator
                .get_device(device)
                .map_err(|e| CaptureError::MicUnavailable {
                    detail: format!("采集设备不存在/不可打开（{device}）: {e:?}"),
                })?;
            dev.get_iaudioclient()
                .map_err(|e| CaptureError::MicUnavailable {
                    detail: format!("采集设备打开失败（{device}）: {e:?}"),
                })
        }
    }
}

/// 初始化/启动阶段错误按目标映射（进程环回维持既有 `Failed` 语义；设备环回 →
/// `LoopbackUnavailable`；麦克风 → `MicUnavailable`）。
fn stage_error(target: &CaptureTarget, stage: &str, detail: String) -> CaptureError {
    match target {
        CaptureTarget::Process { .. } => CaptureError::Failed {
            detail: format!("{stage}: {detail}"),
        },
        CaptureTarget::DeviceLoopback => CaptureError::LoopbackUnavailable {
            detail: format!("{stage}: {detail}"),
        },
        CaptureTarget::Microphone { .. } => CaptureError::MicUnavailable {
            detail: format!("{stage}: {detail}"),
        },
    }
}

/// 采集核：激活 → 事件驱动取包 → 样本槽编排 → sink。
/// sink 返回 false = 调用方请求提前终止（正常收尾）。任一 I/O 步骤失败即 Err。
/// `ready` 在 `start_stream` 成功后触发，携带**实际捕获格式**（REQ-8）。
fn run_capture_core<F>(
    target: &CaptureTarget,
    stop: &AtomicBool,
    mode: EndMode,
    ready: &mut dyn FnMut(CaptureFormat),
    sink: &mut F,
) -> Result<(CaptureStats, CaptureFormat), CaptureError>
where
    F: FnMut(PacketPlan, &[f32]) -> bool,
{
    // 专用线程内 COM MTA 初始化（S3 §4：UI 线程/STA 禁用——RPC_E_CHANGED_MODE）。
    let _ = wasapi::initialize_mta();

    let wall = match mode {
        EndMode::Duration { wall } => Some(wall),
        EndMode::UntilStopped => None,
    };
    let started = Instant::now();

    // 采集客户端激活（进程环回/设备环回/麦克风；失败 → 对应目标错误变体）。
    let mut client = create_capture_client(target)?;

    // REQ-7 分源捕获格式策略：Process 查询不可用（wasapi 0.26 get_mixformat →
    // Not implemented），固定 48k；System/Mic 查询设备 GetMixFormat 取原生采样率
    // （仅采样率原生，仍请求 f32/双声道），失败回退 48k + autoconvert。
    let queried = match target {
        CaptureTarget::Process { .. } => None,
        CaptureTarget::DeviceLoopback | CaptureTarget::Microphone { .. } => {
            match client.get_mixformat() {
                Ok(f) => Some(f.get_samplespersec()),
                Err(e) => {
                    eprintln!(
                        "[sr-audio] GetMixFormat 查询失败，回退 {}Hz + autoconvert（不阻断录制）: {e:?}",
                        PIPELINE_SAMPLE_RATE
                    );
                    None
                }
            }
        }
    };
    let capture_format = capture_format_strategy(target, queried);
    let format = WaveFormat::new(
        32,
        32,
        &SampleType::Float,
        capture_format.sample_rate as usize,
        capture_format.channels as usize,
        None,
    );
    client
        .initialize_client(
            &format,
            &Direction::Capture,
            &StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: 0,
            },
        )
        .map_err(|e| stage_error(target, "initialize_client 失败", format!("{e:?}")))?;
    let h_event = client
        .set_get_eventhandle()
        .map_err(|e| stage_error(target, "set_get_eventhandle 失败", format!("{e:?}")))?;
    let capture = client
        .get_audiocaptureclient()
        .map_err(|e| stage_error(target, "get_audiocaptureclient 失败", format!("{e:?}")))?;
    let blockalign = format.get_blockalign() as usize;

    client
        .start_stream()
        .map_err(|e| stage_error(target, "start_stream 失败", format!("{e:?}")))?;
    ready(capture_format);

    let target_frames = wall.map(|w| sync_target_frames(w, capture_format.sample_rate));
    let channels = capture_format.channels as usize;
    let mut timeline = CaptureTimeline::new(capture_format.sample_rate);
    let mut stats = CaptureStats::default();
    let mut buf: Vec<u8> = Vec::new();
    let mut had_t0 = false;
    let mut sink_closed = false;

    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        // 事件超时仅表示「暂无新数据」；墙钟续流与结束判定照常进行。
        let _ = h_event.wait_for_event(100);
        // 排空当前全部包。
        loop {
            let n = match capture.get_next_packet_size() {
                Ok(Some(n)) => n,
                Ok(None) | Err(_) => break,
            };
            if n == 0 {
                break;
            }
            let need = n as usize * blockalign;
            if buf.len() < need {
                buf.resize(need, 0);
            }
            let (frames, info) = match capture.read_from_device(&mut buf) {
                Ok(v) => v,
                Err(_) => break,
            };
            if frames == 0 {
                continue;
            }
            stats.packets += 1;
            if info.flags.data_discontinuity {
                stats.discontinuities += 1;
            }
            let plan =
                timeline.on_packet(info.timestamp as i64, frames as usize, info.flags.silent);
            had_t0 = true;
            stats.padded_frames += plan.pad_frames;
            stats.dropped_frames += plan.trimmed_frames as u64;
            // 写入载荷：迟到裁切跳过前缀；SILENT 包内容为数字零（缓冲内容未定义）。
            let skip = plan.trimmed_frames * channels;
            let out_frames = frames as usize - plan.trimmed_frames;
            let payload: Vec<f32> = if info.flags.silent || plan.trimmed_frames >= frames as usize {
                vec![0.0; out_frames * channels]
            } else {
                buf[skip * 4..(skip + out_frames * channels) * 4]
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect()
            };
            if !sink(plan, &payload) {
                sink_closed = true;
                break;
            }
        }
        if sink_closed {
            break;
        }
        // 墙钟静音续流（源停滞/音源消失：S3 §3.3「流不断」）。
        // 仅在距上一真实包超过 `DEFAULT_STALL_100NS` 时触发：正常端点延迟
        // （包时间戳落后 QPC 0.3–0.7ms）不得被补零替换（M13 修复）。
        if had_t0 {
            let pad = timeline.pad_to_wall_clock(qpc_100ns());
            if pad > 0 {
                stats.padded_frames += pad;
                let plan = PacketPlan {
                    pad_frames: pad,
                    trimmed_frames: 0,
                    start_sample: 0,
                };
                if !sink(plan, &[]) {
                    break;
                }
            }
        }
        if let Some(wall) = wall {
            if started.elapsed() >= wall {
                break;
            }
        }
    }
    // 时长钉齐（内存 API）：把已编排帧数补到请求值（S3 §3.3 实测精确到请求值）。
    if let (Some(target), false) = (target_frames, sink_closed) {
        let pad = timeline.pad_to(target);
        if pad > 0 {
            stats.padded_frames += pad;
            let plan = PacketPlan {
                pad_frames: pad,
                trimmed_frames: 0,
                start_sample: 0,
            };
            let _ = sink(plan, &[]);
        }
    }
    let _ = client.stop_stream();
    Ok((stats, capture_format))
}

// ─────────────────────────── 内存 API（红测试一等入口；S3 record 同构） ───────────────────────────

/// 同步「录 N 秒返回」共用实现：专用 MTA 线程采集 → 样本槽编排（静音补齐/迟到
/// 裁切/墙钟续流）→ 时长按请求值与**实际捕获率**钉齐（±48 帧内截断；REQ-8）。
fn capture_sync(target: CaptureTarget, duration: Duration) -> Result<CapturedPcm, CaptureError> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    let name = match &target {
        CaptureTarget::Process { pid } => format!("sr-audio-loopback-{pid}"),
        CaptureTarget::DeviceLoopback => "sr-audio-device-loopback".into(),
        CaptureTarget::Microphone { device } => format!("sr-audio-mic-{}", device.len()),
    };

    let handle = std::thread::Builder::new()
        .name(name)
        .spawn(move || {
            let mut samples: Vec<f32> = Vec::new();
            let channels = std::cell::Cell::new(PIPELINE_CHANNELS as usize);
            let mut ready = |fmt: CaptureFormat| channels.set(fmt.channels as usize);
            let mut sink = |plan: PacketPlan, payload: &[f32]| -> bool {
                samples.resize(
                    samples.len() + plan.pad_frames as usize * channels.get(),
                    0.0,
                );
                samples.extend_from_slice(payload);
                true
            };
            let (stats, fmt) = run_capture_core(
                &target,
                &stop_thread,
                EndMode::Duration { wall: duration },
                &mut ready,
                &mut sink,
            )?;
            // 排水位可因末批包越过请求时长而略超：截到恰请求帧数（容差 48 帧内）。
            let target_frames = sync_target_frames(duration, fmt.sample_rate);
            let exact = target_frames as usize * fmt.channels as usize;
            samples.truncate(exact);
            Ok(CapturedPcm {
                samples,
                sample_rate: fmt.sample_rate,
                channels: fmt.channels,
                stats,
            })
        })
        .map_err(|e| CaptureError::Failed {
            detail: format!("spawn 采集线程失败: {e}"),
        })?;

    match handle.join() {
        Ok(r) => r,
        Err(_) => Err(CaptureError::Failed {
            detail: "采集线程 panic".into(),
        }),
    }
}

/// 进程环回捕获（同步「录 N 秒返回」）：激活前身份校验 → 专用 MTA 线程采集 →
/// 样本槽编排（静音补齐/迟到裁切/墙钟续流）→ 时长按请求值精确补齐。
pub fn capture_process_loopback(
    pid: u32,
    created_at_unix_ms: u64,
    duration: Duration,
) -> Result<CapturedPcm, CaptureError> {
    check_app_identity(pid, created_at_unix_ms)?;
    capture_sync(CaptureTarget::Process { pid }, duration)
}

/// REQ-15：默认渲染端点设备环回，同步录 N 秒（S3 apploop `--device-loopback`
/// 同构）。默认端点缺失/激活失败 → `LoopbackUnavailable`（L2 → BLOCKED）。
pub fn capture_device_loopback(duration: Duration) -> Result<CapturedPcm, CaptureError> {
    capture_sync(CaptureTarget::DeviceLoopback, duration)
}

/// REQ-16：麦克风捕获（同步录 N 秒；`device` = [`AudioDeviceInfo::id`]）。
/// 设备不存在/不可打开 → [`CaptureError::MicUnavailable`]，**快速返回**，
/// 不得静默零包/静默成功（「无设备时明确不可用（SKIP 不得静默）」）。
pub fn capture_microphone(device: &str, duration: Duration) -> Result<CapturedPcm, CaptureError> {
    capture_sync(
        CaptureTarget::Microphone {
            device: device.to_string(),
        },
        duration,
    )
}

/// REQ-16：枚举采集（麦克风）设备。无设备 → `Ok(空列表)`（明确报告，不静默）；
/// 专用线程内 COM/MTA 生命周期（§3.1 线程规则，wasapi 类型 !Send）。
pub fn list_capture_devices() -> Result<Vec<AudioDeviceInfo>, CaptureError> {
    let handle = std::thread::Builder::new()
        .name("sr-audio-enum-devices".into())
        .spawn(|| -> Result<Vec<AudioDeviceInfo>, CaptureError> {
            let _ = wasapi::initialize_mta();
            let enumerator = DeviceEnumerator::new().map_err(|e| CaptureError::Failed {
                detail: format!("DeviceEnumerator::new: {e:?}"),
            })?;
            let default_id = enumerator
                .get_default_device(&Direction::Capture)
                .ok()
                .and_then(|d| d.get_id().ok());
            let collection = enumerator
                .get_device_collection(&Direction::Capture)
                .map_err(|e| CaptureError::Failed {
                    detail: format!("枚举采集设备失败: {e:?}"),
                })?;
            let mut out = Vec::new();
            for dev in &collection {
                let dev = dev.map_err(|e| CaptureError::Failed {
                    detail: format!("读取采集设备失败: {e:?}"),
                })?;
                let id = dev.get_id().map_err(|e| CaptureError::Failed {
                    detail: format!("读取设备 id 失败: {e:?}"),
                })?;
                let name = dev.get_friendlyname().unwrap_or_else(|_| id.clone());
                let is_default = default_id.as_deref() == Some(id.as_str());
                out.push(AudioDeviceInfo {
                    id,
                    name,
                    is_default,
                });
            }
            Ok(out)
        })
        .map_err(|e| CaptureError::Failed {
            detail: format!("spawn 设备枚举线程失败: {e}"),
        })?;
    match handle.join() {
        Ok(r) => r,
        Err(_) => Err(CaptureError::Failed {
            detail: "设备枚举线程 panic".into(),
        }),
    }
}

// ─────────────────────────── 端点混音率查询（M15/REQ-2，UI 只读展示） ───────────────────────────

/// 端点原生混音率（REQ-2；仅用于设置页如实展示，**不得用于推导重采样比率**）。
///
/// `kind` = `"render"`（默认渲染端点 / System 来源）或 `"capture"`（麦克风设备）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointMixRate {
    pub rate: u32,
    pub kind: &'static str,
    pub device_name: String,
}

/// 查询端点混音率（REQ-2）：`device_id=None` → 默认渲染端点；`Some(id)` → 采集
/// 设备。专用 MTA 线程执行；任何失败返回 `None`（UI 显示「未知」，与
/// `probe_capabilities` 隔离；不阻断录制/不参与重采样比率推导）。
pub fn query_endpoint_mix_rate(device_id: Option<String>) -> Option<EndpointMixRate> {
    let handle = std::thread::Builder::new()
        .name("sr-audio-mix-rate".into())
        .spawn(move || -> Option<EndpointMixRate> {
            let _ = wasapi::initialize_mta();
            let enumerator = DeviceEnumerator::new().ok()?;
            let (device, kind) = match device_id {
                None => (
                    enumerator.get_default_device(&Direction::Render).ok()?,
                    "render",
                ),
                Some(id) => (enumerator.get_device(&id).ok()?, "capture"),
            };
            let name = device.get_friendlyname().unwrap_or_default();
            let client = device.get_iaudioclient().ok()?;
            let rate = client.get_mixformat().ok()?.get_samplespersec();
            Some(EndpointMixRate {
                rate,
                kind,
                device_name: if name.is_empty() {
                    "未命名端点".to_string()
                } else {
                    name
                },
            })
        })
        .ok()?;
    handle.join().ok().flatten()
}

// ─────────────────────────── 流式 API（路由管道写；与内存 API 同核） ───────────────────────────

/// 流式采集事件（Pcm：先写 `plan.pad_frames` 帧零再写 samples）。
#[derive(Debug)]
pub enum StreamEvent {
    Pcm {
        plan: PacketPlan,
        samples: Arc<Vec<f32>>,
    },
    /// 运行期捕获错误（终止事件）。
    Error(CaptureError),
}

enum StreamMsg {
    /// 激活完成 + 实际捕获格式（REQ-8：路由不得假设捕获率）。
    Ready(CaptureFormat),
    Event(StreamEvent),
}

/// 流式采集（M6：进程环回/设备环回/麦克风同一采集核；每音轨一个；激活结果经
/// Ready 握手同步给启动方，携带实际捕获格式）。
pub struct LoopbackStream {
    rx: mpsc::Receiver<StreamMsg>,
    stop: Arc<AtomicBool>,
    join: Mutex<Option<JoinHandle<()>>>,
    /// Ready 握手收到的实际捕获格式（REQ-8）。
    format: CaptureFormat,
}

impl LoopbackStream {
    /// 启动进程环回流式采集（身份校验同步执行；激活结果同步等待 ≤5s——失败即
    /// `LoopbackUnavailable`/对应错误，路由启动期即可 BLOCKED 分流）。
    pub fn start(pid: u32, created_at_unix_ms: u64) -> Result<Self, CaptureError> {
        check_app_identity(pid, created_at_unix_ms)?;
        Self::start_target(CaptureTarget::Process { pid })
    }

    /// M6：按采集目标启动流式采集（进程环回/设备环回/麦克风同一采集核）。
    /// 设备环回/麦克风无身份校验；激活失败快速返回对应目标错误变体。
    pub fn start_target(target: CaptureTarget) -> Result<Self, CaptureError> {
        let name = match &target {
            CaptureTarget::Process { pid } => format!("sr-audio-stream-{pid}"),
            CaptureTarget::DeviceLoopback => "sr-audio-stream-device".into(),
            CaptureTarget::Microphone { device } => format!("sr-audio-stream-mic-{}", device.len()),
        };
        let (tx, rx) = mpsc::channel::<StreamMsg>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let diag_label = name.clone();
        let join = std::thread::Builder::new()
            .name(name)
            .spawn(move || {
                let tx_err = tx.clone();
                let mut sink = |plan: PacketPlan, payload: &[f32]| -> bool {
                    tx.send(StreamMsg::Event(StreamEvent::Pcm {
                        plan,
                        samples: Arc::new(payload.to_vec()),
                    }))
                    .is_ok()
                };
                let mut ready = |fmt: CaptureFormat| {
                    let _ = tx.send(StreamMsg::Ready(fmt));
                };
                match run_capture_core(
                    &target,
                    &stop_thread,
                    EndMode::UntilStopped,
                    &mut ready,
                    &mut sink,
                ) {
                    Ok((stats, _fmt)) => {
                        if diag_enabled() {
                            eprintln!(
                                "[diag-audio] {diag_label}: packets={} padded={} dropped={} discontinuities={}",
                                stats.packets,
                                stats.padded_frames,
                                stats.dropped_frames,
                                stats.discontinuities
                            );
                        }
                    }
                    Err(e) => {
                        if diag_enabled() {
                            eprintln!("[diag-audio] {diag_label}: capture failed: {e:?}");
                        }
                        let _ = tx_err.send(StreamMsg::Event(StreamEvent::Error(e)));
                    }
                }
            })
            .map_err(|e| CaptureError::Failed {
                detail: format!("spawn 流式采集线程失败: {e}"),
            })?;

        // 激活握手（激活实测 1–6ms；5s 上限覆盖慢端点环境）。
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(StreamMsg::Ready(format)) => Ok(Self {
                rx,
                stop,
                join: Mutex::new(Some(join)),
                format,
            }),
            Ok(StreamMsg::Event(StreamEvent::Error(e))) => Err(e),
            Ok(StreamMsg::Event(StreamEvent::Pcm { .. })) => Err(CaptureError::Failed {
                detail: "采集在就绪前产出数据（内部错误）".into(),
            }),
            Err(_) => {
                stop.store(true, Ordering::Release);
                let _ = join.join();
                Err(CaptureError::LoopbackUnavailable {
                    detail: "进程环回激活超时（5s）".into(),
                })
            }
        }
    }

    /// 实际捕获格式（Ready 握手回传；REQ-8 路由据此构造重采样器）。
    pub fn capture_format(&self) -> CaptureFormat {
        self.format
    }

    /// 取下一事件（超时返回 None；调用方轮询 stop 标志）。
    pub fn recv_timeout(&self, timeout: Duration) -> Option<StreamEvent> {
        match self.rx.recv_timeout(timeout) {
            Ok(StreamMsg::Ready(_)) => self.recv_timeout(timeout),
            Ok(StreamMsg::Event(e)) => Some(e),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => None,
        }
    }

    /// 请求停止并等待采集线程结束（运行期错误已经由 [`StreamEvent::Error`] 送达）。
    pub fn stop(self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.join.lock().expect("流式采集 join 锁").take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_capture_format_strategy_system_uses_native_rate() {
        let fmt = capture_format_strategy(&CaptureTarget::DeviceLoopback, Some(96_000));
        assert_eq!(
            fmt.sample_rate, 96_000,
            "REQ-7：System 查询到原生率 → 按原生率请求（仅采样率原生，仍 f32/双声道）"
        );
        assert_eq!(fmt.channels, 2, "REQ-7：解码固定双声道");
    }

    #[test]
    fn unit_capture_format_strategy_mic_uses_native_rate() {
        let fmt = capture_format_strategy(
            &CaptureTarget::Microphone {
                device: "mic-1".into(),
            },
            Some(44_100),
        );
        assert_eq!(
            fmt.sample_rate, 44_100,
            "REQ-7：Mic 查询到原生率 → 按原生率请求"
        );
        assert_eq!(fmt.channels, 2);
    }

    #[test]
    fn unit_capture_format_strategy_query_failure_falls_back_48k() {
        for target in [
            CaptureTarget::DeviceLoopback,
            CaptureTarget::Microphone {
                device: "mic-1".into(),
            },
        ] {
            let fmt = capture_format_strategy(&target, None);
            assert_eq!(
                fmt.sample_rate, 48_000,
                "REQ-7：查询失败 → 回退 48000 + autoconvert（现状行为），不阻断录制"
            );
            assert_eq!(fmt.channels, 2);
        }
    }

    #[test]
    fn unit_capture_format_strategy_process_never_queries() {
        // 进程环回 GetMixFormat 不可用：即使调用方提供了“查询结果”也必须固定 48k。
        let fmt = capture_format_strategy(&CaptureTarget::Process { pid: 1 }, Some(96_000));
        assert_eq!(
            fmt.sample_rate, 48_000,
            "REQ-7：Process 固定请求 48000/f32/双声道 + autoconvert"
        );
        assert_eq!(fmt.channels, 2);
    }

    #[test]
    fn unit_capture_format_strategy_ready_carries_actual_rate_and_channels() {
        // REQ-8：Ready 握手必须携带实际捕获格式（路由据此构造 DriftResampler）。
        let msg = StreamMsg::Ready(CaptureFormat {
            sample_rate: 44_100,
            channels: 2,
        });
        match msg {
            StreamMsg::Ready(fmt) => {
                assert_eq!(fmt.sample_rate, 44_100, "Ready 应携带实际捕获率");
                assert_eq!(fmt.channels, 2, "Ready 应携带实际声道数");
            }
            _ => panic!("Ready 变体形状不符"),
        }
    }

    #[test]
    fn unit_capture_format_strategy_sync_frames_use_capture_rate() {
        // REQ-8：同步捕获路径的时长钉齐以实际捕获率为域（96k 6s = 576000 帧）。
        assert_eq!(
            sync_target_frames(Duration::from_secs(6), 96_000),
            576_000,
            "同步捕获目标帧数应使用实际捕获率（不得固定 48k）"
        );
        assert_eq!(
            sync_target_frames(Duration::from_secs(6), 44_100),
            264_600,
            "44.1k 捕获目标帧数"
        );
    }
}
