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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use sr_core::redact::{redact_path, redact_text_known_paths, redact_tool_error};

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
                    crate::sidecar::render_path_for_stderr(seg)
                ),
            }
        }
    }
    std::fs::write(list_path, list)
        .map_err(|e| format!("写收尾清单失败（{}）: {e}", redact_path(list_path)))
}

/// 成品校验（收尾提交与恢复幂等复用同一口径）：存在 + 非零 + ffprobe 可解析且
/// 容器时长 > 0。
///
/// `safe_commit` 在提交前校验临时成品；恢复路径「清单已记成品」分支在复用/清理前
/// 必须用同一口径复核——否则零字节/损坏成品会被误判为成功，默认清理随即删掉原段。
pub(crate) fn verify_final_media(ffprobe: &Path, p: &Path) -> Result<(), String> {
    verify_final_media_with_expectation(ffprobe, p, None)
}

/// 统一基础校验 + 可选期望轨数（M17/REQ-17：正常收尾与恢复幂等共用同口径）。
pub(crate) fn verify_final_media_with_expectation(
    ffprobe: &Path,
    p: &Path,
    expected: Option<MediaExpectation>,
) -> Result<(), String> {
    let len = std::fs::metadata(p)
        .map_err(|e| format!("成品校验失败（{}：不存在或不可读：{e}）", redact_path(p)))?
        .len();
    if len == 0 {
        return Err(format!("成品校验失败（{}：0 字节）", redact_path(p)));
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
        .map_err(|e| format!("成品校验失败（{}：ffprobe 启动失败：{e}）", redact_path(p)))?;
    if !out.status.success() {
        let tail: String = String::from_utf8_lossy(&out.stderr)
            .lines()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .join(" | ");
        return Err(format!(
            "成品校验失败（{}：不可解析：{}）",
            redact_path(p),
            redact_text_known_paths(&tail, &[p])
        ));
    }
    let dur: f64 = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0.0);
    if !(dur > 0.0) {
        return Err(format!(
            "成品校验失败（{}：时长无效 {dur}）",
            redact_path(p)
        ));
    }
    if let Some(expected) = expected {
        let (video, audio) = probe_stream_counts(ffprobe, p)?;
        if video != expected.video || audio != expected.audio {
            return Err(format!(
                "成品校验失败（{}：轨数不符：期望 v={} a={}，实际 v={video} a={audio}）",
                redact_path(p),
                expected.video,
                expected.audio
            ));
        }
    }
    Ok(())
}

/// 逐流 codec_type 计数（ffprobe）。
fn probe_stream_counts(ffprobe: &Path, p: &Path) -> Result<(usize, usize), String> {
    let out = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type",
            "-of",
            "csv=p=0",
        ])
        .arg(p)
        .output()
        .map_err(|e| format!("轨数校验失败（{}：ffprobe 启动失败：{e}）", redact_path(p)))?;
    if !out.status.success() {
        return Err(format!(
            "轨数校验失败（{}：{}）",
            redact_path(p),
            redact_text_known_paths(&String::from_utf8_lossy(&out.stderr), &[p])
        ));
    }
    let (mut video, mut audio) = (0usize, 0usize);
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        match line.trim() {
            "video" => video += 1,
            "audio" => audio += 1,
            _ => {}
        }
    }
    Ok((video, audio))
}

// ─────────────────────── M17/REQ-17：容器收尾事务（可注入 runner） ───────────────────────

/// MP4 容器族（ffprobe `format_name` 的 token；REQ-19：不得断言 == "mp4"）。
pub(crate) const MP4_FORMAT_FAMILY: [&str; 6] = ["mov", "mp4", "m4a", "3gp", "3g2", "mj2"];

/// `format_name`（逗号分隔）是否属于 MP4 容器族。
pub(crate) fn format_family_ok(format_name: &str) -> bool {
    format_name
        .split(',')
        .map(str::trim)
        .any(|token| MP4_FORMAT_FAMILY.contains(&token))
}

/// 期望轨数（去重后 sources 的成品投影；`video` 通常 1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MediaExpectation {
    pub video: usize,
    pub audio: usize,
}

impl MediaExpectation {
    /// 清单 `streams`（`"v"` / `"a0"..`）→ 期望轨数（恢复幂等复核与收尾同口径）。
    pub(crate) fn from_streams(streams: &[String]) -> Self {
        Self {
            video: streams.iter().filter(|s| s.as_str() == "v").count(),
            audio: streams.iter().filter(|s| s.starts_with('a')).count(),
        }
    }
}

/// 收尾/取消控制（P1-2）：原子标志供 remux/concat/校验热路径轮询；提交门保证
/// 「取消登记」与「校验 + 原子提交」互斥。
pub(crate) struct FinalizeControl {
    cancel: AtomicBool,
    gate: crate::naming::CommitGate,
}

impl FinalizeControl {
    pub(crate) fn new() -> Self {
        Self {
            cancel: AtomicBool::new(false),
            gate: crate::naming::CommitGate::new(),
        }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    /// 登记取消：`true` = 提交必被拒绝（调用方不得删除预留路径）；`false` = 提交已
    /// 完成（取消太晚，成品归属本事务）。
    pub(crate) fn request_cancel(&self) -> bool {
        if self.gate.cancel() {
            self.cancel.store(true, Ordering::Release);
            true
        } else {
            false
        }
    }

    /// 提交边界：与取消互斥的 `safe_commit`（失败返回 [`crate::naming::CommitError`]）。
    fn safe_commit<F>(
        &self,
        tmp_path: &Path,
        final_path: &Path,
        verify: F,
    ) -> Result<(), crate::naming::CommitError>
    where
        F: FnOnce(&Path) -> Result<(), String>,
    {
        if self.is_cancelled() {
            return Err(crate::naming::CommitError::Cancelled);
        }
        self.gate.safe_commit(tmp_path, final_path, verify)
    }
}

/// L2 阶段屏障（P1-2）：让测试确定性停在 remux 已启动 / 提交边界。
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinalizeTestPhase {
    /// remux 子进程已 spawn（可取消窗口内）。
    RemuxStarted,
    /// remux 校验完成、safe_commit 之前。
    CommitBoundary,
}

#[doc(hidden)]
#[derive(Debug)]
pub struct FinalizePhaseBarrier {
    state: Mutex<PhaseBarrierState>,
    cond: Condvar,
}

#[derive(Debug)]
struct PhaseBarrierState {
    entered: bool,
    released: bool,
}

impl FinalizePhaseBarrier {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(PhaseBarrierState {
                entered: false,
                released: false,
            }),
            cond: Condvar::new(),
        }
    }

    /// 等待收尾线程到达屏障（超时返回 false；测试失败而非静默继续）。
    pub fn wait_entered(&self, timeout: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        let mut state = self.state.lock().expect("阶段屏障锁");
        while !state.entered {
            let now = std::time::Instant::now();
            if now >= deadline {
                return false;
            }
            let (guard, _) = self
                .cond
                .wait_timeout(state, deadline - now)
                .expect("阶段屏障锁");
            state = guard;
        }
        true
    }

    /// 放行被屏障停住的收尾线程（可重复调用）。
    pub fn release(&self) {
        let mut state = self.state.lock().expect("阶段屏障锁");
        state.released = true;
        self.cond.notify_all();
    }

    fn arrive_and_wait(&self) {
        let mut state = self.state.lock().expect("阶段屏障锁");
        state.entered = true;
        self.cond.notify_all();
        while !state.released {
            state = self.cond.wait(state).expect("阶段屏障锁");
        }
    }
}

impl Default for FinalizePhaseBarrier {
    fn default() -> Self {
        Self::new()
    }
}

fn test_phase_barrier_slot(
) -> &'static Mutex<Option<(FinalizeTestPhase, Arc<FinalizePhaseBarrier>)>> {
    static SLOT: std::sync::OnceLock<
        Mutex<Option<(FinalizeTestPhase, Arc<FinalizePhaseBarrier>)>>,
    > = std::sync::OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// L2 注入：设置阶段屏障（`None` 清除）。生产路径永不设置。
#[doc(hidden)]
pub fn set_finalize_phase_barrier_for_tests(
    phase: FinalizeTestPhase,
    barrier: Option<Arc<FinalizePhaseBarrier>>,
) {
    *test_phase_barrier_slot().lock().expect("阶段屏障槽锁") =
        barrier.map(|barrier| (phase, barrier));
}

/// 收尾线程到达阶段点时调用（未配置屏障 → 立即返回）。
pub(crate) fn hit_test_phase(phase: FinalizeTestPhase) {
    let entry = test_phase_barrier_slot()
        .lock()
        .ok()
        .and_then(|slot| slot.clone());
    if let Some((configured, barrier)) = entry {
        if configured == phase {
            barrier.arrive_and_wait();
        }
    }
}

/// 收尾事务输入（正常收尾与恢复共用）。
pub(crate) struct FinalizeJob<'a> {
    /// 会话目录（concat/remux 临时物落此；与成品同卷）。
    pub session_dir: &'a Path,
    /// 输出根（磁盘预检锚点）。
    pub output_dir: PathBuf,
    /// 启动时预留的最终成品名（按容器扩展名）。
    pub final_path: &'a Path,
    /// ffconcat 清单路径。
    pub list_path: &'a Path,
    /// 参与收尾的有效段（>0 字节）。
    pub segments: &'a [PathBuf],
    /// 期望轨数（= 实际去重后 sources）。
    pub expected: MediaExpectation,
    /// 成品容器（启动冻结）。
    pub container: crate::naming::OutputContainer,
}

/// 媒体命令执行器（生产 = 锁定 ffmpeg sidecar；测试可注入失败/延迟）。
pub(crate) trait MediaRunner {
    /// concat 段清单 → 临时 MKV（`-map 0 -c copy -f matroska`）。
    fn concat(
        &self,
        list: &Path,
        out: &Path,
        cancel: Option<&FinalizeControl>,
    ) -> Result<(), String>;

    /// MKV → MP4 流拷贝（`-map 0 -c copy -movflags +faststart -f mp4`）。
    fn remux(
        &self,
        input: &Path,
        out: &Path,
        cancel: Option<&FinalizeControl>,
    ) -> Result<(), String>;
}

/// remux 命令 argv（REQ-17：`-map 0` 保全部流；显式 `-f mp4`；输出以 `.tmp` 结尾）。
pub(crate) fn build_remux_args(input: &Path, out: &Path) -> Vec<String> {
    vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-i".into(),
        input.to_string_lossy().to_string(),
        "-map".into(),
        "0".into(),
        "-c".into(),
        "copy".into(),
        "-movflags".into(),
        "+faststart".into(),
        "-f".into(),
        "mp4".into(),
        out.to_string_lossy().to_string(),
    ]
}

/// 收尾磁盘总峰值估算（原段 + concat + 临时 MP4；REQ-17 预算 ≈3 份）。
pub(crate) fn finalize_space_needed(
    input_bytes: u64,
    container: crate::naming::OutputContainer,
) -> u64 {
    let copies = match container {
        crate::naming::OutputContainer::Mkv => 2,
        crate::naming::OutputContainer::Mp4 => 3,
    };
    input_bytes.saturating_mul(copies)
}

/// 收尾期间**新增**空间预算（P1-3）：原段 S 已占盘，只有临时物是新占用。
///
/// - MKV：concat 临时 1S（提交为同卷 rename，不复制）；
/// - MP4：concat 临时 + remux 临时 = 2S。
///
/// 预检只比较新增预算（总峰值仍保留给日志/诊断），避免把已占盘的原段重复计入
/// 而误报磁盘不足。P2-B：MKV 仅记录预算、不做硬预检（保持 8847f0f 既有收尾行为）；
/// 硬拒绝只用于 MP4。
pub(crate) fn finalize_incremental_space_needed(
    input_bytes: u64,
    container: crate::naming::OutputContainer,
) -> u64 {
    let copies = match container {
        crate::naming::OutputContainer::Mkv => 1,
        crate::naming::OutputContainer::Mp4 => 2,
    };
    input_bytes.saturating_mul(copies)
}

/// 空间预检边界：可用空间**恰等于**新增预算时允许（严格小于才拒绝）。
pub(crate) fn check_finalize_space(need: u64, free: Option<u64>) -> Result<(), String> {
    match free {
        Some(free) if free < need => Err(format!(
            "磁盘空间不足（收尾需新增 {need} 字节，可用 {free} 字节；原段保留）"
        )),
        _ => Ok(()),
    }
}

/// 容器族校验（矩阵与收尾同口径）：MP4 = mov/mp4/m4a/3gp/3g2/mj2；MKV = matroska/webm。
fn container_family_ok(container: crate::naming::OutputContainer, format_name: &str) -> bool {
    match container {
        crate::naming::OutputContainer::Mp4 => format_family_ok(format_name),
        crate::naming::OutputContainer::Mkv => format_name
            .split(',')
            .map(str::trim)
            .any(|token| token == "matroska" || token == "webm"),
    }
}

/// 容器族显示名（错误信息用）。
fn container_family_label(container: crate::naming::OutputContainer) -> &'static str {
    match container {
        crate::naming::OutputContainer::Mp4 => "MP4（mov/mp4/m4a/3gp/3g2/mj2）",
        crate::naming::OutputContainer::Mkv => "MKV（matroska/webm）",
    }
}

/// 成品校验（P1-3）：存在 + 非零 + **容器族** + 期望轨数 + 时长 > 0。
///
/// 正常收尾（concat/remux 临时物与最终提交）与恢复幂等复用共用此口径，防止
/// 「缺轨/错容器的替代文件」被当作有效成品后清理原段。
pub(crate) fn verify_final_media_with_container(
    ffprobe: &Path,
    p: &Path,
    expected: MediaExpectation,
    container: crate::naming::OutputContainer,
) -> Result<(), String> {
    let label = match container {
        crate::naming::OutputContainer::Mp4 => "MP4",
        crate::naming::OutputContainer::Mkv => "MKV",
    };
    let len = std::fs::metadata(p)
        .map_err(|e| {
            format!(
                "成品校验失败（{label} {}：不存在或不可读：{e}）",
                redact_path(p)
            )
        })?
        .len();
    if len == 0 {
        return Err(format!(
            "成品校验失败（{label} {}：0 字节）",
            redact_path(p)
        ));
    }
    let out = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=format_name,duration",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(p)
        .output()
        .map_err(|e| {
            format!(
                "成品校验失败（{label} {}：ffprobe 启动失败：{e}）",
                redact_path(p)
            )
        })?;
    if !out.status.success() {
        let tail: String = String::from_utf8_lossy(&out.stderr)
            .lines()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .join(" | ");
        return Err(format!(
            "成品校验失败（{label} {}：不可解析：{}）",
            redact_path(p),
            redact_text_known_paths(&tail, &[p])
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut format_name = String::new();
    let mut duration = 0.0f64;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("format_name=") {
            format_name = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("duration=") {
            duration = v.trim().parse().unwrap_or(0.0);
        }
    }
    if !container_family_ok(container, &format_name) {
        return Err(format!(
            "成品校验失败（{label} {}：容器族不符：format_name={format_name:?}，期望 {}）",
            redact_path(p),
            container_family_label(container)
        ));
    }
    if !(duration > 0.0) {
        return Err(format!(
            "成品校验失败（{label} {}：时长无效 {duration}）",
            redact_path(p)
        ));
    }
    let (video, audio) = probe_stream_counts(ffprobe, p)?;
    if video != expected.video || audio != expected.audio {
        return Err(format!(
            "成品校验失败（{label} {}：轨数不符：期望 v={} a={}，实际 v={video} a={audio}）",
            redact_path(p),
            expected.video,
            expected.audio
        ));
    }
    Ok(())
}

/// MP4 成品校验（REQ-19）：存在+非零+容器族+期望轨数+时长>0。
pub(crate) fn verify_mp4_media(
    ffprobe: &Path,
    p: &Path,
    expected: MediaExpectation,
) -> Result<(), String> {
    verify_final_media_with_container(ffprobe, p, expected, crate::naming::OutputContainer::Mp4)
}

fn cancelled(cancel: Option<&FinalizeControl>) -> bool {
    cancel.is_some_and(FinalizeControl::is_cancelled)
}

#[cfg(test)]
thread_local! {
    /// 测试注入可用空间（0 = 必然不足）；仅当前线程，生产路径不读取。
    static FREE_SPACE_OVERRIDE: std::cell::Cell<Option<u64>> =
        const { std::cell::Cell::new(None) };
    /// 空间策略日志捕获（验证「MKV 空间不足仍继续」的预算日志）。
    static SPACE_LOGS: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn set_free_space_override_for_tests(free: Option<u64>) {
    FREE_SPACE_OVERRIDE.with(|slot| slot.set(free));
}

#[cfg(test)]
fn take_space_logs_for_tests() -> Vec<String> {
    SPACE_LOGS.with(|logs| std::mem::take(&mut *logs.borrow_mut()))
}

/// 空间预算日志（P2-B：MKV 只记录、不硬拒绝；测试捕获同一消息）。
fn log_space_budget(
    container: crate::naming::OutputContainer,
    need: u64,
    total_peak: u64,
    free: Option<u64>,
) {
    let name = match container {
        crate::naming::OutputContainer::Mkv => "MKV",
        crate::naming::OutputContainer::Mp4 => "MP4",
    };
    let msg = match free {
        Some(free) if free < need => format!(
            "[finalize] {name} 收尾空间不足（可用 {free} 字节 < 新增预算 {need} 字节；总峰值 {total_peak} 字节）：按既有行为继续（不硬拒绝）"
        ),
        Some(free) => format!(
            "[finalize] {name} 收尾空间预算：新增 {need} 字节，可用 {free} 字节，总峰值 {total_peak} 字节"
        ),
        None => format!(
            "[finalize] {name} 收尾空间预算：新增 {need} 字节，总峰值 {total_peak} 字节（可用空间未知，预检降级）"
        ),
    };
    eprintln!("{msg}");
    #[cfg(test)]
    SPACE_LOGS.with(|logs| logs.borrow_mut().push(msg));
}

/// P2-B：空间预检策略。
///
/// - MP4：硬预检，新增预算不足直接拒绝（失败保留原段）；
/// - MKV：历史收尾（`8847f0f`）无硬预检，仅记录预算日志并继续；空间真不足时由
///   concat 自身写失败归因（不新增拒绝条件）。
///
/// `free == None`（查询失败）对两种容器都降级为日志，不拒绝。
fn finalize_space_policy(
    container: crate::naming::OutputContainer,
    need: u64,
    total_peak: u64,
    free: Option<u64>,
) -> Result<(), String> {
    match container {
        crate::naming::OutputContainer::Mp4 => match free {
            Some(free) => check_finalize_space(need, Some(free))
                .map_err(|e| format!("{e}（新增空间预算 {need} / 总峰值 {total_peak}）")),
            None => {
                log_space_budget(container, need, total_peak, None);
                Ok(())
            }
        },
        crate::naming::OutputContainer::Mkv => {
            log_space_budget(container, need, total_peak, free);
            Ok(())
        }
    }
}

/// 输出卷可用空间（字节；查询失败 → None，预检降级为记录日志）。
fn free_space_bytes(dir: &Path) -> Option<u64> {
    #[cfg(test)]
    if let Some(free) = FREE_SPACE_OVERRIDE.with(|slot| slot.get()) {
        return Some(free);
    }
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide: Vec<u16> = dir
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut free = 0u64;
    unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut free), None, None) }.ok()?;
    Some(free)
}

/// 收尾事务临时物清理守卫（成功提交后 remove 为无害 no-op）。
struct TempCleanup(Vec<PathBuf>);
impl Drop for TempCleanup {
    fn drop(&mut self) {
        for p in &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// 统一收尾事务：写清单 → concat → 基础校验 →（MP4）remux → MP4 校验 → safe_commit。
/// 失败路径清理临时物并保留原段；返回提交后的成品路径。
///
/// P2-B：空间预检仅 MP4 硬拒绝；MKV 记录预算后继续（既有收尾无硬预检）。
pub(crate) fn run_finalize_transaction(
    job: &FinalizeJob<'_>,
    runner: &dyn MediaRunner,
    cancel: Option<&FinalizeControl>,
) -> Result<PathBuf, String> {
    if cancelled(cancel) {
        return Err("已取消（批量回滚），未开始收尾".into());
    }
    let total_bytes: u64 = job
        .segments
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok().map(|m| m.len()))
        .sum();
    let total_peak = finalize_space_needed(total_bytes, job.container);
    let need = finalize_incremental_space_needed(total_bytes, job.container);
    finalize_space_policy(
        job.container,
        need,
        total_peak,
        free_space_bytes(&job.output_dir),
    )?;

    write_segment_list(job.list_path, job.segments, job.expected.audio > 0)?;

    let name = job
        .final_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "final".into());
    let concat_tmp = job.session_dir.join(format!(".{name}.concat.mkv.tmp"));
    let remux_tmp = job.session_dir.join(format!(".{name}.remux.mp4.tmp"));
    let _cleanup = TempCleanup(vec![concat_tmp.clone(), remux_tmp.clone()]);
    let _ = std::fs::remove_file(&concat_tmp);
    let _ = std::fs::remove_file(&remux_tmp);

    runner.concat(job.list_path, &concat_tmp, cancel)?;
    let ffprobe = crate::sidecar::ffprobe_exe()
        .map_err(|e| format!("收尾校验失败：{}", redact_tool_error(&e)))?;
    verify_final_media_with_container(
        &ffprobe,
        &concat_tmp,
        job.expected,
        crate::naming::OutputContainer::Mkv,
    )?;

    let commit_src = match job.container {
        crate::naming::OutputContainer::Mkv => concat_tmp.clone(),
        crate::naming::OutputContainer::Mp4 => {
            if cancelled(cancel) {
                return Err("已取消（批量回滚），remux 未开始".into());
            }
            if let Some(delay) = remux_test_delay() {
                wait_remux_delay(delay, cancel)?;
            }
            runner.remux(&concat_tmp, &remux_tmp, cancel)?;
            verify_mp4_media(&ffprobe, &remux_tmp, job.expected)?;
            remux_tmp.clone()
        }
    };
    if cancelled(cancel) {
        return Err("已取消（批量回滚），未提交成品".into());
    }
    hit_test_phase(FinalizeTestPhase::CommitBoundary);
    let expected = job.expected;
    let container = job.container;
    let verify = |p: &Path| verify_final_media_with_container(&ffprobe, p, expected, container);
    let committed = match cancel {
        Some(control) => control.safe_commit(&commit_src, job.final_path, verify),
        None => crate::naming::safe_commit(&commit_src, job.final_path, verify),
    };
    committed.map_err(|e| match e {
        crate::naming::CommitError::Cancelled => "已取消（批量回滚），未提交成品".to_string(),
        // CommitError::FinalExists 的 Debug 携带成品绝对路径：detail 在构造点脱敏。
        other => format!(
            "成品提交失败: {}",
            redact_text_known_paths(&format!("{other:?}"), &[&commit_src, job.final_path])
        ),
    })?;
    Ok(job.final_path.to_path_buf())
}

/// M17/REQ-17：L2 注入的 remux 暂停（生产默认 0；仅测试设置）。
fn remux_test_delay() -> Option<std::time::Duration> {
    crate::route::remux_test_delay()
}

fn wait_remux_delay(
    delay: std::time::Duration,
    cancel: Option<&FinalizeControl>,
) -> Result<(), String> {
    let deadline = std::time::Instant::now() + delay;
    while std::time::Instant::now() < deadline {
        if cancelled(cancel) {
            return Err("已取消（批量回滚），remux 延迟中止".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    Ok(())
}

/// 段的视频/音频包时间戳摘要（ffprobe `packet` 扫描；失败 → `None`）。
fn segment_timing(seg: &Path) -> Option<SegmentTiming> {
    let ffprobe = match crate::sidecar::ffprobe_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "[sr-pipeline] ffprobe 不可用：{}",
                crate::sidecar::render_tool_error_for_stderr(&e)
            );
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
        eprintln!(
            "[sr-pipeline] ffprobe 失败（{}）: {}",
            crate::sidecar::render_path_for_stderr(seg),
            crate::sidecar::render_text_for_stderr(&tail, &[seg])
        );
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

    // ─────────────────── M17/REQ-17：remux argv / 失败矩阵（可注入 runner） ───────────────────

    fn test_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("sr_m17_finalize_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".screenrec_tmp").join("sess-1")).expect("session");
        std::fs::create_dir_all(root.join("out")).expect("out");
        root
    }

    fn ffmpeg_make(path: &Path, ext: &str) {
        let ffmpeg = crate::sidecar::ffmpeg_exe().expect("锁定 ffmpeg");
        let st = Command::new(ffmpeg)
            .args([
                "-hide_banner",
                "-nostdin",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=64x64:r=10:d=0.4",
            ])
            .args(["-c:v", "mpeg4", "-f", ext])
            .arg(path)
            .status()
            .expect("spawn ffmpeg 生成夹具");
        assert!(st.success(), "生成夹具失败（{st}）");
    }

    #[test]
    fn unit_remux_args_map_all_streams_faststart_explicit_mp4() {
        let input = PathBuf::from(r"C:\sess\.final.x.concat.mkv.tmp");
        let out = PathBuf::from(r"C:\sess\x.mp4.tmp");
        let args = build_remux_args(&input, &out);
        let s: Vec<String> = args.iter().map(|a| a.replace('\\', "/")).collect();
        assert_eq!(
            s,
            vec![
                "-hide_banner",
                "-nostdin",
                "-i",
                "C:/sess/.final.x.concat.mkv.tmp",
                "-map",
                "0",
                "-c",
                "copy",
                "-movflags",
                "+faststart",
                "-f",
                "mp4",
                "C:/sess/x.mp4.tmp",
            ],
            "REQ-17：remux argv 必须 -map 0 / -c copy / +faststart / -f mp4 / .tmp"
        );
        assert!(
            out.to_string_lossy().ends_with(".mp4.tmp"),
            "REQ-17：remux 临时物应为 <out>.tmp"
        );
    }

    #[test]
    fn unit_finalize_space_estimate_is_three_copies_for_mp4() {
        use crate::naming::OutputContainer;
        assert_eq!(finalize_space_needed(1000, OutputContainer::Mp4), 3000);
        assert_eq!(finalize_space_needed(1000, OutputContainer::Mkv), 2000);
        assert_eq!(
            finalize_space_needed(u64::MAX, OutputContainer::Mp4),
            u64::MAX,
            "饱和乘法不得溢出"
        );
    }

    #[test]
    fn unit_finalize_incremental_space_budget_excludes_existing_segments() {
        use crate::naming::OutputContainer;
        // P1-3：原段 S 已占盘，新增预算 MKV = 1S、MP4 = 2S（总峰值仍为 2S/3S）。
        assert_eq!(
            finalize_incremental_space_needed(1000, OutputContainer::Mkv),
            1000
        );
        assert_eq!(
            finalize_incremental_space_needed(1000, OutputContainer::Mp4),
            2000
        );
        assert_eq!(
            finalize_incremental_space_needed(u64::MAX, OutputContainer::Mp4),
            u64::MAX,
            "饱和乘法不得溢出"
        );
        assert!(
            finalize_incremental_space_needed(1000, OutputContainer::Mkv)
                < finalize_space_needed(1000, OutputContainer::Mkv),
            "新增预算不得把已占盘原段重复计入"
        );
    }

    #[test]
    fn unit_finalize_space_boundary_equal_budget_passes_one_byte_short_fails() {
        assert_eq!(
            check_finalize_space(2000, Some(2000)),
            Ok(()),
            "可用空间恰等于新增预算必须允许"
        );
        let err = check_finalize_space(2000, Some(1999)).expect_err("少 1 字节应拒绝");
        assert!(err.contains("磁盘空间不足"), "错误应可归因（得到 {err}）");
        assert!(err.contains("2000"), "错误应含新增预算（得到 {err}）");
        assert!(check_finalize_space(0, Some(0)).is_ok(), "0 预算不得拒绝");
        assert!(
            check_finalize_space(2000, None).is_ok(),
            "查询失败应降级而不是拒绝"
        );
    }

    // ── M19c REQ-2/6：校验错误 detail 脱敏（basename 保留、父目录不出现） ──

    #[test]
    fn unit_verify_error_detail_redacts_parent_keeps_basename() {
        let root =
            std::env::temp_dir().join(format!("sr_m19c_verify_redact_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("private-user-dir");
        std::fs::create_dir_all(&dir).expect("创建私有目录");
        let ffprobe = crate::sidecar::ffprobe_exe().expect("定位锁定 ffprobe");

        // 缺失文件：归因 + basename 保留，父目录不出现。
        let missing = dir.join("missing_final.mp4");
        let err = verify_final_media(&ffprobe, &missing).expect_err("缺失文件必须显式失败");
        assert!(err.contains("成品校验失败"), "归因保留（{err}）");
        assert!(err.contains("missing_final.mp4"), "basename 保留（{err}）");
        assert!(!err.contains("private-user-dir"), "父目录不得出现（{err}）");

        // ffprobe 不可解析分支：原因保留、路径脱敏（stderr 尾部已知路径同步替换）。
        let corrupt = dir.join("corrupt_final.mkv");
        std::fs::write(&corrupt, b"not-an-mkv").expect("写损坏成品");
        let err = verify_final_media(&ffprobe, &corrupt).expect_err("损坏文件必须显式失败");
        assert!(err.contains("不可解析"), "ffprobe 原因保留（{err}）");
        assert!(err.contains("corrupt_final.mkv"), "basename 保留（{err}）");
        assert!(!err.contains("private-user-dir"), "父目录不得出现（{err}）");
        let _ = std::fs::remove_dir_all(&root);
    }

    struct FreeSpaceGuard;
    impl Drop for FreeSpaceGuard {
        fn drop(&mut self) {
            set_free_space_override_for_tests(None);
        }
    }

    /// P2-B 生产分支：MKV 空间不足不得硬拒绝（8847f0f 既有收尾无预检），
    /// 仅记录预算日志；收尾照常提交。
    #[test]
    fn unit_finalize_mkv_insufficient_space_still_commits_with_budget_log() {
        let root = test_root("mkv-nospace");
        let session = root.join(".screenrec_tmp").join("sess-1");
        let seg = session.join("out0_00000.mkv");
        std::fs::write(&seg, b"SEGMENT-SENTINEL").expect("写原段占位");
        let mkv = root.join("fixture.mkv");
        ffmpeg_make(&mkv, "matroska");
        let final_path = root.join("out").join("20261018_120000_Target.mkv");
        let list = session.join("segments.ffconcat");
        let segments = vec![seg.clone()];
        let runner = FakeRunner {
            mkv,
            mp4: root.join("unused.mp4"),
            mode: RemuxMode::Valid,
        };
        set_free_space_override_for_tests(Some(0));
        let _guard = FreeSpaceGuard;
        take_space_logs_for_tests();

        let committed = run_finalize_transaction(
            &job_with_container(
                root.join("out"),
                &session,
                &segments,
                &final_path,
                &list,
                crate::naming::OutputContainer::Mkv,
            ),
            &runner,
            None,
        )
        .expect("P2-B：MKV 空间不足必须维持既有行为（不得硬拒绝）");
        assert_eq!(committed, final_path);
        assert!(final_path.exists(), "MKV 成品应提交");
        assert_eq!(
            std::fs::read(&seg).expect("读原段"),
            b"SEGMENT-SENTINEL",
            "事务本身不删除原段"
        );
        assert!(tmp_left(&session).is_empty(), "成功提交后临时物不应残留");
        let logs = take_space_logs_for_tests().join("\n");
        assert!(logs.contains("MKV"), "预算日志应标明容器（得到 {logs}）");
        assert!(logs.contains("预算"), "预算日志应含预算字样（得到 {logs}）");
        assert!(
            logs.contains("16") && logs.contains("32"),
            "预算日志应含新增预算/总峰值实际值（得到 {logs}）"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// P2-B 生产分支：MP4 新增预算不足必须硬拒绝，且保留原段、不发布成品。
    #[test]
    fn unit_finalize_mp4_insufficient_space_rejects_and_keeps_segments() {
        let root = test_root("mp4-nospace");
        let session = root.join(".screenrec_tmp").join("sess-1");
        let seg = session.join("out0_00000.mkv");
        std::fs::write(&seg, b"SEGMENT-SENTINEL").expect("写原段占位");
        let mkv = root.join("fixture.mkv");
        let mp4 = root.join("fixture.mp4");
        ffmpeg_make(&mkv, "matroska");
        ffmpeg_make(&mp4, "mp4");
        let final_path = root.join("out").join("20261018_120000_Target.mp4");
        let list = session.join("segments.ffconcat");
        let segments = vec![seg.clone()];
        let runner = FakeRunner {
            mkv,
            mp4,
            mode: RemuxMode::Valid,
        };
        set_free_space_override_for_tests(Some(0));
        let _guard = FreeSpaceGuard;
        take_space_logs_for_tests();

        let err = run_finalize_transaction(
            &job(root.join("out"), &session, &segments, &final_path, &list),
            &runner,
            None,
        )
        .expect_err("P2-B：MP4 新增预算不足必须硬拒绝");
        assert!(err.contains("磁盘空间不足"), "错误应可归因（得到 {err}）");
        assert!(
            err.contains("新增空间预算 32"),
            "MP4 预算 = 2×16 字节（得到 {err}）"
        );
        assert!(!final_path.exists(), "拒绝时不得发布成品");
        assert_eq!(
            std::fs::read(&seg).expect("读原段"),
            b"SEGMENT-SENTINEL",
            "拒绝时必须保留原段且不改写"
        );
        assert!(tmp_left(&session).is_empty(), "拒绝时不得产生临时物");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unit_commit_gate_cancel_wins_before_commit() {
        let gate = crate::naming::CommitGate::new();
        let root =
            std::env::temp_dir().join(format!("sr_m17c_commit_gate_cancel_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("gate 临时目录");
        let tmp = root.join("tmp.part");
        let final_path = root.join("final.mp4");
        std::fs::write(&tmp, b"MEDIA").expect("写临时物");

        assert!(gate.cancel(), "取消应先于提交登记成功");
        let err = gate
            .safe_commit(&tmp, &final_path, |_| Ok(()))
            .expect_err("已取消的提交必须被拒绝");
        assert!(matches!(err, crate::naming::CommitError::Cancelled));
        assert!(!final_path.exists(), "取消后不得出现成品");
        assert!(tmp.exists(), "拒绝提交不得触碰临时物");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unit_commit_gate_commit_wins_then_cancel_is_too_late() {
        let gate = crate::naming::CommitGate::new();
        let root =
            std::env::temp_dir().join(format!("sr_m17c_commit_gate_commit_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("gate 临时目录");
        let tmp = root.join("tmp.part");
        let final_path = root.join("final.mp4");
        std::fs::write(&tmp, b"MEDIA").expect("写临时物");

        gate.safe_commit(&tmp, &final_path, |_| Ok(()))
            .expect("未取消的提交应成功");
        assert!(
            !gate.cancel(),
            "提交完成后到达的取消必须返回「太晚」（调用方据此按已提交处理）"
        );
        assert_eq!(std::fs::read(&final_path).expect("成品"), b"MEDIA");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum RemuxMode {
        Valid,
        SpawnFail,
        NonZero,
        Timeout,
        VerifyFail,
    }

    struct FakeRunner {
        mkv: PathBuf,
        mp4: PathBuf,
        mode: RemuxMode,
    }

    impl MediaRunner for FakeRunner {
        fn concat(
            &self,
            _list: &Path,
            out: &Path,
            _cancel: Option<&FinalizeControl>,
        ) -> Result<(), String> {
            std::fs::copy(&self.mkv, out).map_err(|e| e.to_string())?;
            Ok(())
        }

        fn remux(
            &self,
            _input: &Path,
            out: &Path,
            _cancel: Option<&FinalizeControl>,
        ) -> Result<(), String> {
            match self.mode {
                RemuxMode::Valid => std::fs::copy(&self.mp4, out)
                    .map(|_| ())
                    .map_err(|e| e.to_string()),
                RemuxMode::SpawnFail => Err("spawn remux 失败: test-injected".into()),
                RemuxMode::NonZero => Err("remux 退出码 Some(1)；stderr 尾部: test".into()),
                RemuxMode::Timeout => Err("remux 超时（180s）".into()),
                RemuxMode::VerifyFail => {
                    std::fs::write(out, b"not-an-mp4").map_err(|e| e.to_string())
                }
            }
        }
    }

    fn job<'a>(
        output_dir: PathBuf,
        session: &'a Path,
        segments: &'a [PathBuf],
        final_path: &'a Path,
        list: &'a Path,
    ) -> FinalizeJob<'a> {
        job_with_container(
            output_dir,
            session,
            segments,
            final_path,
            list,
            crate::naming::OutputContainer::Mp4,
        )
    }

    fn job_with_container<'a>(
        output_dir: PathBuf,
        session: &'a Path,
        segments: &'a [PathBuf],
        final_path: &'a Path,
        list: &'a Path,
        container: crate::naming::OutputContainer,
    ) -> FinalizeJob<'a> {
        FinalizeJob {
            session_dir: session,
            output_dir,
            final_path,
            list_path: list,
            segments,
            expected: MediaExpectation { video: 1, audio: 0 },
            container,
        }
    }

    fn tmp_left(session: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(session)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().map_or(false, |x| x == "tmp"))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn failure_case(tag: &str, mode: RemuxMode) {
        let expected: &[&str] = match mode {
            RemuxMode::Valid => &[],
            RemuxMode::SpawnFail => &["spawn"],
            RemuxMode::NonZero => &["退出码"],
            RemuxMode::Timeout => &["超时"],
            RemuxMode::VerifyFail => &["MP4", "容器"],
        };
        let root = test_root(tag);
        let session = root.join(".screenrec_tmp").join("sess-1");
        let seg = session.join("out0_00000.mkv");
        std::fs::write(&seg, b"SEGMENT-SENTINEL").expect("写原段占位");
        let mkv = root.join("fixture.mkv");
        let mp4 = root.join("fixture.mp4");
        ffmpeg_make(&mkv, "matroska");
        ffmpeg_make(&mp4, "mp4");
        let final_path = root.join("out").join("20261018_120000_Target.mp4");
        let list = session.join("segments.ffconcat");
        let segments = vec![seg.clone()];
        let runner = FakeRunner { mkv, mp4, mode };
        let err = run_finalize_transaction(
            &job(root.join("out"), &session, &segments, &final_path, &list),
            &runner,
            None,
        )
        .expect_err("REQ-17：注入的 remux 失败必须显式失败");
        println!("[m17-finalize] {tag} → {err}");
        assert!(
            expected.iter().any(|m| err.contains(m)),
            "{tag}：错误应可归因（期望含 {expected:?}，得到 {err}）"
        );
        assert!(!final_path.exists(), "{tag}：失败不得发布成品");
        assert!(seg.exists(), "{tag}：失败必须保留原段");
        assert_eq!(
            std::fs::read(&seg).expect("读原段"),
            b"SEGMENT-SENTINEL",
            "{tag}：原段不得被改写"
        );
        assert!(
            tmp_left(&session).is_empty(),
            "{tag}：临时物应清理（得到 {:?}）",
            tmp_left(&session)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unit_finalize_container_failures_spawn() {
        failure_case("spawn", RemuxMode::SpawnFail);
    }

    #[test]
    fn unit_finalize_container_failures_nonzero_exit() {
        failure_case("nonzero", RemuxMode::NonZero);
    }

    #[test]
    fn unit_finalize_container_failures_timeout() {
        failure_case("timeout", RemuxMode::Timeout);
    }

    #[test]
    fn unit_finalize_container_failures_verify() {
        failure_case("verify", RemuxMode::VerifyFail);
    }

    #[test]
    fn unit_finalize_container_failures_commit_conflict() {
        let root = test_root("commit");
        let session = root.join(".screenrec_tmp").join("sess-1");
        let seg = session.join("out0_00000.mkv");
        std::fs::write(&seg, b"SEGMENT-SENTINEL").expect("写原段占位");
        let mkv = root.join("fixture.mkv");
        let mp4 = root.join("fixture.mp4");
        ffmpeg_make(&mkv, "matroska");
        ffmpeg_make(&mp4, "mp4");
        let final_path = root.join("out").join("20261018_120000_Target.mp4");
        std::fs::write(&final_path, b"EXTERNAL-SENTINEL").expect("占位最终名");
        let list = session.join("segments.ffconcat");
        let segments = vec![seg.clone()];
        let runner = FakeRunner {
            mkv,
            mp4,
            mode: RemuxMode::Valid,
        };
        let err = run_finalize_transaction(
            &job(root.join("out"), &session, &segments, &final_path, &list),
            &runner,
            None,
        )
        .expect_err("REQ-17：提交冲突必须显式失败");
        println!("[m17-finalize] commit → {err}");
        assert!(
            err.contains("成品"),
            "提交冲突错误应明确指向成品提交（得到 {err}）"
        );
        assert_eq!(
            std::fs::read(&final_path).expect("占位文件应存在"),
            b"EXTERNAL-SENTINEL",
            "REQ-17：既有成品不得被覆盖"
        );
        assert!(seg.exists(), "REQ-17：提交冲突必须保留原段");
        assert!(
            tmp_left(&session).is_empty(),
            "REQ-17：提交冲突后临时物应清理（得到 {:?}）",
            tmp_left(&session)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unit_finalize_container_success_commits_mp4() {
        let root = test_root("success");
        let session = root.join(".screenrec_tmp").join("sess-1");
        let seg = session.join("out0_00000.mkv");
        std::fs::write(&seg, b"SEGMENT-SENTINEL").expect("写原段占位");
        let mkv = root.join("fixture.mkv");
        let mp4 = root.join("fixture.mp4");
        ffmpeg_make(&mkv, "matroska");
        ffmpeg_make(&mp4, "mp4");
        let final_path = root.join("out").join("20261018_120000_Target.mp4");
        let list = session.join("segments.ffconcat");
        let segments = vec![seg.clone()];
        let runner = FakeRunner {
            mkv,
            mp4,
            mode: RemuxMode::Valid,
        };
        let committed = run_finalize_transaction(
            &job(root.join("out"), &session, &segments, &final_path, &list),
            &runner,
            None,
        )
        .expect("REQ-17：有效 MP4 应提交成功");
        assert_eq!(committed, final_path);
        assert!(final_path.exists(), "成品应存在");
        assert_eq!(
            std::fs::read(&seg).expect("读原段"),
            b"SEGMENT-SENTINEL",
            "事务本身不删除原段（清理策略由调用层执行）"
        );
        assert!(tmp_left(&session).is_empty(), "成功提交后临时物不应残留");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unit_finalize_cancel_before_commit_does_not_publish() {
        let root = test_root("cancel");
        let session = root.join(".screenrec_tmp").join("sess-1");
        let seg = session.join("out0_00000.mkv");
        std::fs::write(&seg, b"SEGMENT-SENTINEL").expect("写原段占位");
        let mkv = root.join("fixture.mkv");
        let mp4 = root.join("fixture.mp4");
        ffmpeg_make(&mkv, "matroska");
        ffmpeg_make(&mp4, "mp4");
        let final_path = root.join("out").join("20261018_120000_Target.mp4");
        let list = session.join("segments.ffconcat");
        let segments = vec![seg.clone()];
        let cancel = FinalizeControl::new();
        assert!(cancel.request_cancel(), "测试前置：取消应先于提交生效");
        let runner = FakeRunner {
            mkv,
            mp4,
            mode: RemuxMode::Valid,
        };
        let err = run_finalize_transaction(
            &job(root.join("out"), &session, &segments, &final_path, &list),
            &runner,
            Some(&cancel),
        )
        .expect_err("REQ-17：已取消的收尾不得提交");
        assert!(err.contains("取消"), "错误应说明取消（得到 {err}）");
        assert!(!final_path.exists(), "取消后不得有成品");
        assert!(seg.exists());
        assert!(tmp_left(&session).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
