//! 后台收尾通用化（REQ-21）：ffconcat 收尾清单构建 + 段边界时间轴校准。
//!
//! ## M10b P0：concat 段边界时间轴空洞
//!
//! 分段编码使用 `-reset_timestamps 1`：每个新段的时间零点由**该段最早包**决定。
//! 音频包通常领先视频关键帧 60–110ms 被写入新段（muxer 交织队列），因此每段都带一个
//! **段首前导**：音轨从 0 开始、视频首帧从 ~0.06–0.10s 开始；段容器时长 =
//! 视频首帧偏移 + 视频内容，比音轨内容长约 60–110ms。
//!
//! concat demuxer 按「上一段清单 `duration`」推进下一段的起点，并按每段**段内最早包
//! 的 PTS** 把该段对齐到 0。仅写 `file`（无 `duration`）时它会取段容器时长 ⇒ 每段前导被
//! 计入成片时间轴：每个段边界（名义 5s）插入 ~60–110ms 空洞（610s 录制 122 个边界累计
//! +8.47s / +1.39%，音轨侧同时出现同数量 PTS 间隙，A/V 内容漂移 ≈20–29ms/s）。
//!
//! ## 修复：以视频轨为时间轴锚点
//!
//! 逐段用 ffprobe 读取**段内最早包 PTS**、**视频首/末包 PTS** 与视频包数，在清单中为
//! 该段写 `duration = (视频末包 PTS − 最早包 PTS) + 帧间隔 − (下段视频首包 PTS − 下段
//! 最早包 PTS)`。即让下一段视频首帧恰好落在上一段末帧之后一个帧间隔：视频时间轴保持
//! 严格 CFR（段边界无空洞、无重叠、无 32ms 台阶），音频只承担 ≤2ms 的段边界残差
//! （实测 122 个边界累计 +9.6ms，单调不累积）。
//!
//! 为什么不用容器时长或音轨内容跨度：容器时长含段首前导（空洞来源）；音轨内容跨度虽
//! 消除空洞，但视频在边界出现 32ms 台阶，长录制下 `-f null` 的 1/30 时间基换算会把
//! 相邻帧压到同一刻度并报 DTS 非单调（610s 实测 241 条），违反「整文件可解码（无错误
//! 输出）」。视频锚定后 610s 成片：视频 PTS 增量仅 33/34ms、`-f null` 零告警、段边界
//! >30ms 空洞 0 个，成片 610.344s（录制段 610.159s），内容级 |offset| max 98.5ms、
//! 斜率 0.0780ms/s（docs/milestones/m10b-fix-report.md）。
//!
//! 纯视频路由（无音轨）不写 `duration`，保持原有容器时长语义（M10 soak 无累积缺陷）。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 单段视频/音频时间戳摘要（ffprobe 包扫描；秒）。
#[derive(Debug, Clone, Copy, PartialEq)]
struct SegmentTiming {
    /// 段内最早包 PTS（concat demuxer 的对齐零点）。
    origin: f64,
    /// 视频首包 PTS（min）。
    video_first: f64,
    /// 视频末包 PTS（max；B 帧仅影响包顺序，不影响 max）。
    video_last: f64,
    /// 视频包数（帧间隔估计用）。
    video_packets: usize,
}

impl SegmentTiming {
    /// 下一段应推进到的清单 `duration`（视频 CFR 连续；秒）。
    fn duration_to(self, next: SegmentTiming) -> Option<f64> {
        if self.video_packets < 2 {
            return None; // 单帧/无视频段：不出校准值（退回容器时长，仅影响该边界）
        }
        let frame = (self.video_last - self.video_first) / (self.video_packets - 1) as f64;
        let d = (self.video_last - self.origin) + frame - (next.video_first - next.origin);
        (d.is_finite() && d > 0.0).then_some(d)
    }
}

/// 写收尾 ffconcat 清单（相对条目；清单与段同目录 ⇒ concat demuxer 按清单位置解析）。
///
/// `has_audio == true` 时为每个非末段写校准后的 `duration`（见模块文档）；单段 ffprobe
/// 失败/无音轨/时间戳异常时该段退回「无 duration」（容器时长语义，不阻塞收尾）。
/// 末段不写 `duration`（不会推进任何后续段，且成片终点由末段实际包决定）。
pub fn write_segment_list(
    list_path: &Path,
    segments: &[PathBuf],
    has_audio: bool,
) -> Result<(), String> {
    let timings: Vec<Option<SegmentTiming>> = if has_audio {
        segments.iter().map(|s| segment_timing(s)).collect()
    } else {
        vec![None; segments.len()]
    };
    let mut list = String::from("ffconcat version 1.0\n");
    for (i, seg) in segments.iter().enumerate() {
        let name = seg
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        list.push_str(&format!("file '{}'\n", name.replace('\'', "'\\''")));
        if has_audio && i + 1 < segments.len() {
            let duration = match (timings[i], timings[i + 1]) {
                (Some(a), Some(b)) => a.duration_to(b),
                _ => None,
            };
            match duration {
                Some(d) => list.push_str(&format!("duration {d:.6}\n")),
                None => eprintln!(
                    "[sr-pipeline] 段时间戳不可用，退回容器时长（{}）",
                    seg.display()
                ),
            }
        }
    }
    std::fs::write(list_path, list)
        .map_err(|e| format!("写收尾清单失败（{}）: {e}", list_path.display()))
}

/// 成品校验（收尾提交与恢复幂等复用同一口径）：存在 + 非零 + ffprobe 可解析且
/// 容器时长 > 0。
///
/// `safe_commit` 在提交前校验临时成品；恢复路径「清单已记成品」分支在复用/清理前
/// 必须用同一口径复核——否则零字节/损坏成品会被误判为成功，默认清理随即删掉原段。
pub(crate) fn verify_final_media(ffprobe: &Path, p: &Path) -> Result<(), String> {
    let len = std::fs::metadata(p)
        .map_err(|e| format!("成品校验失败（{}：不存在或不可读：{e}）", p.display()))?
        .len();
    if len == 0 {
        return Err(format!("成品校验失败（{}：0 字节）", p.display()));
    }
    let out = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(p)
        .output()
        .map_err(|e| format!("成品校验失败（{}：ffprobe 启动失败：{e}）", p.display()))?;
    if !out.status.success() {
        let tail: String = String::from_utf8_lossy(&out.stderr)
            .lines()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .join(" | ");
        return Err(format!("成品校验失败（{}：不可解析：{tail}）", p.display()));
    }
    let dur: f64 = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0.0);
    if !(dur > 0.0) {
        return Err(format!("成品校验失败（{}：时长无效 {dur}）", p.display()));
    }
    Ok(())
}

/// 段的视频/音频包时间戳摘要（ffprobe `packet` 扫描；失败 → `None`）。
fn segment_timing(seg: &Path) -> Option<SegmentTiming> {
    let ffprobe = match crate::sidecar::ffprobe_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[sr-pipeline] ffprobe 不可用：{e}");
            return None;
        }
    };
    let out = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "packet=stream_index,pts_time",
            "-of",
            "csv=p=0",
        ])
        .arg(seg)
        .output()
        .ok()?;
    if !out.status.success() {
        let tail: String = String::from_utf8_lossy(&out.stderr)
            .lines()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .join(" | ");
        eprintln!("[sr-pipeline] ffprobe 失败（{}）: {tail}", seg.display());
        return None;
    }
    parse_segment_timing(&String::from_utf8_lossy(&out.stdout))
}

/// 解析 `packet=stream_index,pts_time` 的 CSV（文件顺序无关）→ 时间戳摘要。
///
/// `stream_index == 0` 视为视频（`build_encode_args` 固定 `-map 0:v` 在前）。
fn parse_segment_timing(csv: &str) -> Option<SegmentTiming> {
    let mut origin = f64::INFINITY;
    let (mut video_first, mut video_last) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut video_packets = 0usize;
    for line in csv.lines() {
        let mut fields = line.trim().split(',');
        let Some(stream) = fields.next().and_then(|s| s.trim().parse::<u32>().ok()) else {
            continue;
        };
        let Some(pts) = fields.next().and_then(|s| s.trim().parse::<f64>().ok()) else {
            continue;
        };
        origin = origin.min(pts);
        if stream == 0 {
            video_first = video_first.min(pts);
            video_last = video_last.max(pts);
            video_packets += 1;
        }
    }
    (video_packets > 0 && origin.is_finite()).then_some(SegmentTiming {
        origin,
        video_first,
        video_last,
        video_packets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timing(csv: &str) -> SegmentTiming {
        parse_segment_timing(csv).expect("应解析出段摘要")
    }

    #[test]
    fn video_anchor_keeps_cfr_after_audio_leading_gap() {
        // 真机段形态：段 0 视频 [0.100, 6.067]（180 帧）、音频领先（最早 0.079）；
        // 段 1 视频 [0.070, 4.037]（120 帧）、音频最早 -0.021。
        let a = SegmentTiming {
            origin: 0.079,
            video_first: 0.100,
            video_last: 6.067,
            video_packets: 180,
        };
        let b = SegmentTiming {
            origin: -0.021,
            video_first: 0.070,
            video_last: 4.037,
            video_packets: 120,
        };
        let d = a.duration_to(b).expect("应给出段 duration");
        let frame = (a.video_last - a.video_first) / 179.0;
        let next_video_first_aligned = d + (b.video_first - b.origin);
        assert!(
            (next_video_first_aligned - ((a.video_last - a.origin) + frame)).abs() < 1e-9,
            "下一段视频首帧应接续 CFR（得到 {next_video_first_aligned}）"
        );
        assert!(
            (d - 5.930335).abs() < 1e-5,
            "段 duration 应≈5.930335（得到 {d}）"
        );
    }

    #[test]
    fn parse_extracts_origin_and_video_span() {
        let t = timing("0,0.100000\n1,0.079000\n0,6.067000\n1,6.009000\n");
        assert_eq!(t.origin, 0.079);
        assert_eq!((t.video_first, t.video_last), (0.100, 6.067));
        assert_eq!(t.video_packets, 2);
    }

    #[test]
    fn malformed_or_single_frame_has_no_duration() {
        assert_eq!(parse_segment_timing(""), None);
        assert_eq!(parse_segment_timing("N/A,N/A\n"), None);
        let single = timing("0,0.100000\n1,0.079000\n");
        let next = timing("0,0.070000\n0,0.103000\n");
        assert_eq!(single.duration_to(next), None);
        // 无音轨包不影响视频摘要（是否写 duration 由路由层 has_audio 决定）。
        assert!(parse_segment_timing("0,0.100000\n0,0.133000\n").is_some());
    }
}
