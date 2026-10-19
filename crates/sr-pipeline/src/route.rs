//! 单路视频路由（M3）：`start_route` → RouteHandle（REQ-9/11/13/20/21 的 M3 范围）；
//! M4 音轨扩展（`start_audio_route`）；M5 多路批量（`start_routes`：全成功或回滚，REQ-12）；
//! M6 音频范围（全音轨 System/Mic 源 + `start_audio_only` 纯录音，REQ-15/16/17/19）；
//! M8 恢复集成（REQ-22）：会话临时目录 + manifest 自动维护（段/清单在会话目录内，
//! 成品在输出目录根）；sidecar 进程级 Kill-on-close Job；写失败 → `DiskFull`。
//!
//! 启动协议（§3.1，消除互等）：建管（server 先监听）→ spawn sidecar → 外置超时连接
//! → 捕获会话启动 ⇒ **Ok = 「输出开始」**（编码器冷启动首写阻塞由 8MiB 管道缓冲吸收）。
//! 停止协议：`request_stop` 立即返回；worker 停捕获 → **EOF（唯一优雅停止）** →
//! 等 sidecar 退出（超时强杀）→ 后台 concat（显式映射、0 字节段排除）。
//! 错误路径（§3.5）：sidecar 非零退出/被终止 → `SidecarExit`（保留分段）；目标失效 →
//! `TargetInvalid`；编码器链全败 → `EncoderUnavailable`；WGC 授权被拒 → `ConsentDenied`；
//! 环回不可用 → `LoopbackUnavailable`（纯录音同）。
//! M14 保留策略：正常停止且成品验证成功后默认递归清理会话临时目录（原段）；设置
//! `keep_segments` 开启时保留。清理失败只记录日志、不改变收尾成功归因；失败/崩溃路径
//! 始终保留原段（恢复状态机扫描不变）。M14b：纯录音（`start_audio_only`）成功收尾
//! 纳入同一策略（默认清理会话目录；`keep_segments=true` 保留；失败/崩溃路径不删）。
//! M17/P3：成品容器启动时冻结（MKV 默认/MP4 收尾流拷贝 remux）；收尾事务全程持有
//! 会话级占用，批量回滚显式取消后台收尾（通用 `wait_finalized` 契约不变）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sr_audio::resample::{DriftAction, DriftCfg, DriftController, DriftResampler};
use sr_audio::wasapi::{
    check_app_identity, AudioSourceSpec, CaptureError, CaptureTarget, LoopbackStream, StreamEvent,
};
use sr_audio::wav::WavWriter;
use sr_capture::pipe::{
    PipeConnectError, PipeOptions, VideoPipe, AUDIO_PIPE_BUFFER_BYTES, PIPE_WRITE_TIMEOUT,
};
use sr_capture::region::{CaptureLossLatch, RegionCrop};
use sr_capture::wgc::{self, LatestFrame, StreamControl, StreamStartError};

use crate::naming::{format_timestamp, next_available, sanitize_target_name};
use crate::sidecar::{
    build_encode_args, detect_encoder, spawn_sidecar, spawn_stderr_drainer, AudioPipeSpec,
    VideoPipeSpec,
};

/// 段长（D5：5s MKV 分段）。
const SEGMENT_SECS: u32 = 5;
/// REQ-26 漂移控制锚点（40/20/500ppm；S6 定值前的 spec 锚点）。
const DRIFT_ENTER_MS: f64 = 40.0;
const DRIFT_EXIT_MS: f64 = 20.0;
const DRIFT_LIMIT_PPM: f64 = 500.0;
/// 漂移测量节流（音频写线程；200ms 足以滤除包级抖动）。
const DRIFT_MEASURE_INTERVAL: Duration = Duration::from_millis(200);
/// sidecar 连接外置超时（§3.3/S1b：≥5s）。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// EOF 后等待 sidecar 收尾的上限（超时强杀兜底）。
const SIDECAR_STOP_TIMEOUT: Duration = Duration::from_secs(5);
/// concat 收尾上限（wait_finalized 契约窗口 120s 的分量）。
const FINALIZE_TIMEOUT: Duration = Duration::from_secs(60);

/// M18/REQ-22：区域矩形（物理桌面坐标；负坐标合法）。
///
/// 输出尺寸在启动时按 `w & !1` / `h & !1` 向下取偶（NV12/编码约束）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// 向下取偶后的区域矩形（输出尺寸口径；w/h 保持 ≥2）。
pub fn snap_region_rect(rect: RegionRect) -> RegionRect {
    RegionRect {
        x: rect.x,
        y: rect.y,
        w: rect.w & !1,
        h: rect.h & !1,
    }
}

/// 桌面矩形 → 显示器帧内裁剪（纯函数；REQ-22 坐标与边界）。
///
/// 失败原因：矩形不含于显示器、起点/终点越界、尺寸不足 64×64（吸附后）。
pub fn region_crop_rect(
    rect: RegionRect,
    monitor_x: i32,
    monitor_y: i32,
    monitor_w: u32,
    monitor_h: u32,
) -> Result<RegionCrop, String> {
    let snapped = snap_region_rect(rect);
    if snapped.w < 64 || snapped.h < 64 {
        return Err(format!(
            "区域尺寸过小（吸附后 {}x{}，要求 ≥64x64）",
            snapped.w, snapped.h
        ));
    }
    let end_x_i64 = rect.x as i64 + snapped.w as i64;
    let end_y_i64 = rect.y as i64 + snapped.h as i64;
    let monitor_end_x = monitor_x as i64 + monitor_w as i64;
    let monitor_end_y = monitor_y as i64 + monitor_h as i64;
    if rect.x < monitor_x
        || rect.y < monitor_y
        || end_x_i64 > monitor_end_x
        || end_y_i64 > monitor_end_y
    {
        return Err(format!(
            "区域越界（rect=({},{}) {}x{}；显示器=({monitor_x},{monitor_y}) {monitor_w}x{monitor_h}）",
            rect.x, rect.y, snapped.w, snapped.h
        ));
    }
    let start_x = (rect.x - monitor_x) as u32;
    let start_y = (rect.y - monitor_y) as u32;
    RegionCrop::new(start_x, start_y, start_x + snapped.w, start_y + snapped.h)
}

/// 录制目标（§3.4）。`Region` = 单显示器内区域（M18）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteTarget {
    Window {
        hwnd: u64,
    },
    Monitor {
        index: u32,
    },
    /// 区域目标：`display_device` = 稳定 device_path，缺失时回退 GDI `szDevice`。
    Region {
        display_device: String,
        rect: RegionRect,
    },
}

/// 视频配置（固定画布 D6：启动时固定宽高/fps/像素格式）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoCfg {
    pub canvas_w: u32,
    pub canvas_h: u32,
    pub fps: u32,
    pub cursor: bool,
}

/// 路由配置（M3：视频单路，无音频）。
#[derive(Clone, Debug)]
pub struct RouteConfig {
    pub target: RouteTarget,
    pub video: VideoCfg,
    pub output_dir: PathBuf,
}

/// 路由错误。测试钉死的 4 变体见 m3-red-report.md §3.4（花括号模式，容忍带字段）；
/// 其余变体实现自定。
#[derive(Clone, Debug)]
pub enum RouteError {
    /// 编码器链全败 / 不支持实际区域尺寸（M18）。
    EncoderUnavailable,
    /// M18/REQ-21：编码器不支持实际区域尺寸（不缩放/补边）。
    RegionSizeUnsupported { width: u32, height: u32 },
    /// E_CONSENT_DENIED（WGC 授权被拒）。
    ConsentDenied,
    /// E_SIDECAR_EXIT（sidecar 非零退出/被终止；保留分段）。
    SidecarExit { detail: String },
    /// 收尾等待超时。
    WaitTimeout,
    /// E_LOOPBACK_UNAVAILABLE（§3.5：进程环回/默认端点不可用 → 路级明确报错，
    /// 不改录系统混音）。
    LoopbackUnavailable { detail: String },
    /// 目标不存在/无效（E_TARGET_LOST 语义）。
    TargetInvalid { detail: String },
    /// 捕获会话启动失败。
    CaptureFailed { detail: String },
    /// 启动协议失败（建管/连接/spawn）。
    StartFailed { detail: String },
    /// 收尾失败（concat 失败/无有效段）。
    FinalizeFailed { detail: String },
    /// E_DISK_FULL（§3.4/§3.5）：录制/收尾中输出写失败（磁盘满/写拒绝等 ENOSPC
    /// 类）——安全停止、保留原段、收尾可重试。
    DiskFull { detail: String },
    /// 实现内部约束失败（正常路径不应出现）。
    Internal { detail: String },
}

impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EncoderUnavailable => write!(f, "E_ENCODER_UNAVAILABLE: 无可用编码器"),
            Self::RegionSizeUnsupported { width, height } => write!(
                f,
                "E_ENCODER_UNAVAILABLE: 编码器不支持区域尺寸 {width}x{height}（不缩放/补边）"
            ),
            Self::ConsentDenied => write!(f, "E_CONSENT_DENIED: WGC 授权被拒"),
            Self::SidecarExit { detail } => write!(
                f,
                "E_SIDECAR_EXIT: sidecar 退出/被终止（保留分段）: {detail}"
            ),
            Self::WaitTimeout => write!(f, "E_WAIT_TIMEOUT: 收尾等待超时"),
            Self::LoopbackUnavailable { detail } => write!(f, "E_LOOPBACK_UNAVAILABLE: {detail}"),
            Self::TargetInvalid { detail } => write!(f, "E_TARGET_LOST: {detail}"),
            Self::CaptureFailed { detail } => write!(f, "E_CAPTURE_FAILED: {detail}"),
            Self::StartFailed { detail } => write!(f, "E_START_FAILED: {detail}"),
            Self::FinalizeFailed { detail } => write!(f, "E_FINALIZE_FAILED: {detail}"),
            Self::DiskFull { detail } => write!(f, "E_DISK_FULL: {detail}"),
            Self::Internal { detail } => write!(f, "E_INTERNAL: {detail}"),
        }
    }
}

impl std::error::Error for RouteError {}

/// 收尾产物。
#[derive(Clone, Debug)]
pub struct Finalized {
    /// 成品：`{yyyyMMdd_HHmmss}_{sanitize(目标名)}.{mkv|mp4}`（容器启动时冻结），
    /// 位于 output_dir 内。
    pub final_path: PathBuf,
    /// ffconcat 段清单（收尾 concat 的输入；已排除 0 字节段）。默认成功后随会话目录
    /// 清理（路径不再存在）；`keep_segments=true` 时保留。
    pub segment_list: PathBuf,
    /// 参与收尾的段（全部 >0 字节）。默认成功后随会话目录清理；`keep_segments=true`
    /// 时保留。
    pub segments: Vec<PathBuf>,
    /// M16/REQ-12：路级停止原因（端点失效；成功/失败分支都可读）。
    pub stop_notice: Option<StopNotice>,
}

/// 进程内路由序号（管道名/段 pattern/清单名唯一性）。
static ROUTE_SEQ: AtomicU32 = AtomicU32::new(0);

struct Shared {
    stop: AtomicBool,
    /// M16/REQ-12：端点失效等路级停止原因（首因生效；进入快照/终态事件）。
    stop_notice: Mutex<Option<StopNotice>>,
    /// M18/REQ-24：区域生产者失效闩锁（WGC on_closed/尺寸变化；worker 结束后重读）。
    region_loss: Option<CaptureLossLatch>,
    /// M18/REQ-24：L2 确定性交错注入——强制 capture_ended（生产永不设置）。
    capture_ended_for_tests: AtomicBool,
    /// L2 收尾失败分支注入（[`RouteHandle::force_finalize_failure_for_tests`]）。
    force_finalize_failure: AtomicBool,
    /// M17/REQ-17：批量回滚显式取消后台收尾（P1-2：提交门与取消互斥；热路径检查原子标志）。
    finalize_control: crate::finalize::FinalizeControl,
    /// M17/REQ-17：启动冻结的成品容器（收尾按此 remux）。
    container: crate::naming::OutputContainer,
    /// 段扫描根 = 会话临时目录（REQ-24/D12；段与 ffconcat 清单在会话目录内）。
    output_dir: PathBuf,
    /// 会话临时目录（`.screenrec_tmp/<session_id>`；manifest 维护锚点）。
    session_dir: PathBuf,
    /// 路由 id（manifest.routes[].route_id 锚点）。
    route_id: String,
    /// 启动时分配并预留给本路收尾的最终成品名（P1-4：收尾只提交到此名）。
    final_path: PathBuf,
    /// M14：成品验证成功后是否保留会话目录（原段）；false = 默认自动清理。
    keep_segments: bool,
    seg_prefix: String,
    result: Mutex<Option<Result<Finalized, RouteError>>>,
    cond: Condvar,
    stderr_tail: Arc<Mutex<String>>,
    /// 发送器写错误诊断（PeerGone 码等；错误归因以 child 退出码为准）。
    send_error: Mutex<Option<String>>,
    /// worker 线程句柄（P1-2：批量回滚在返回前 join，确定性确认收尾线程/sidecar 已回收）。
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Shared {
    fn set_result(&self, outcome: Result<Finalized, RouteError>) {
        let mut guard = self.result.lock().expect("route 结果锁");
        if guard.is_none() {
            *guard = Some(outcome);
            self.cond.notify_all();
        }
    }

    /// M16/REQ-12：设置路级停止原因（首因生效）并请求优雅停止。
    fn set_stop_notice(&self, notice: StopNotice) {
        let mut guard = self.stop_notice.lock().expect("route stop_notice 锁");
        if guard.is_none() {
            *guard = Some(notice);
        }
        self.stop.store(true, Ordering::Release);
    }

    fn stop_notice(&self) -> Option<StopNotice> {
        self.stop_notice
            .lock()
            .expect("route stop_notice 锁")
            .clone()
    }
}

/// 单路路由句柄（request_stop 立即返回；wait_finalized 等后台收尾；segments 清单视角）。
pub struct RouteHandle {
    shared: Arc<Shared>,
}

impl RouteHandle {
    /// 「已请求停止」：立即返回（§9 T5）；实际停止/收尾在 worker 线程后台完成。
    pub fn request_stop(&self) {
        self.shared.stop.store(true, Ordering::Release);
    }

    /// 等待后台收尾完成（concat）；超时返回 `WaitTimeout`（路由仍在后台继续）。
    pub fn wait_finalized(&self, timeout: Duration) -> Result<Finalized, RouteError> {
        let deadline = Instant::now() + timeout;
        let mut guard = self.shared.result.lock().expect("route 结果锁");
        loop {
            if let Some(outcome) = guard.as_ref() {
                return outcome.clone();
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(RouteError::WaitTimeout);
            }
            let (g, _) = self
                .shared
                .cond
                .wait_timeout(guard, deadline - now)
                .expect("route 结果锁");
            guard = g;
        }
    }

    /// 当前已写出段（清单视角；0 字节壳不列入；Err 后仍可查询，完成段永不消失）。
    pub fn segments(&self) -> Vec<PathBuf> {
        scan_segments(&self.shared.output_dir, &self.shared.seg_prefix)
    }

    /// 启动时为本路预留的最终成品路径（P1-4；收尾 safe_commit 的目标名）。
    pub fn reserved_final_path(&self) -> PathBuf {
        self.shared.final_path.clone()
    }

    /// M16/REQ-12：路级停止原因（端点失效；成功/失败分支都可读，早于收尾完成）。
    pub fn stop_notice(&self) -> Option<StopNotice> {
        self.shared.stop_notice()
    }

    /// M18/REQ-24：L2 确定性交错注入——强制 capture_ended（生产路径不调用）。
    #[doc(hidden)]
    pub fn force_capture_ended_for_tests(&self) {
        self.shared
            .capture_ended_for_tests
            .store(true, Ordering::Release);
    }

    /// M18/REQ-24：L2 注入——区域生产者闩锁首因（模拟 WGC on_closed 的同步写入）。
    #[doc(hidden)]
    pub fn latch_region_loss_for_tests(&self, reason: &str) {
        if let Some(latch) = &self.shared.region_loss {
            latch.latch(reason);
        }
    }

    /// L2 收尾失败分支注入（`it_route_endpoint_runtime_lost_keeps_others`）：
    /// 让本路 finalize 返回 `FinalizeFailed`，用于断言「失败仍保留原段 + notice 可读」。
    /// 生产路径不调用；无副作用。
    #[doc(hidden)]
    pub fn force_finalize_failure_for_tests(&self) {
        self.shared
            .force_finalize_failure
            .store(true, Ordering::Release);
    }

    /// M17/REQ-17：批量启动回滚显式取消/丢弃后台收尾（正常 `wait_finalized` 契约不变）。
    /// P1-2：登记取消与 safe_commit 的「校验 + 原子提交」互斥；返回 `true` = 取消先于
    /// 提交生效（本次收尾必不产出成品），`false` = 提交已完成（取消太晚）。
    pub(crate) fn cancel_finalize_for_rollback(&self) -> bool {
        self.shared.finalize_control.request_cancel()
    }

    /// P1-2 回归/L2 注入：在 remux 中/提交边界取消后台收尾（生产回滚走
    /// [`Self::cancel_finalize_for_rollback`]）。
    #[doc(hidden)]
    pub fn cancel_finalize_for_tests(&self) -> bool {
        self.shared.finalize_control.request_cancel()
    }

    /// P1-2：等待 worker 线程终止（结果就绪后 join；确认 sidecar/媒体子进程已回收）。
    pub(crate) fn join_worker(&self) {
        let handle = self
            .shared
            .worker
            .lock()
            .ok()
            .and_then(|mut slot| slot.take());
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }
}

impl Drop for RouteHandle {
    fn drop(&mut self) {
        // 兜底：句柄丢弃即请求停止（worker 自行收尾；不阻塞 drop）。
        self.shared.stop.store(true, Ordering::Release);
    }
}

/// CFR 槽发送器（每槽一帧：新鲜则新帧、否则最近帧重复；§3.3 连续 NV12 流）。
struct SlotSender {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
    /// 已发送槽数（P1-2：音频漂移测量的视频时间轴参照）。
    frames: Arc<AtomicU64>,
}

impl SlotSender {
    fn start(
        pipe: VideoPipe,
        latest: Arc<LatestFrame>,
        interval: Duration,
        shared: Arc<Shared>,
    ) -> SlotSender {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let frames = Arc::new(AtomicU64::new(0));
        let frames_thread = frames.clone();
        let diag_route = shared.route_id.clone();
        let handle = std::thread::Builder::new()
            .name("sr-cfr-sender".into())
            .spawn(move || {
                let start = Instant::now();
                let mut k: u64 = 0;
                // 诊断（SR_DIAG_AV=1）：新鲜帧/重复帧/节拍滞后——WGC 停滞或管道背压的
                // 直接证据（重复帧 = 源欠产；滞后 = 发送节拍被写阻塞顶住）。
                let diag = sr_audio::wasapi::diag_enabled();
                let (mut slots, mut fresh, mut repeats, mut run, mut max_run) = (0u64, 0u64, 0u64, 0u64, 0u64);
                let mut max_late_ms = 0.0f64;
                let mut write_failed = false;
                let mut last: Option<Arc<Vec<u8>>> = None;
                loop {
                    if stop_flag.load(Ordering::Acquire) {
                        break;
                    }
                    let target = start + interval * (k + 1) as u32;
                    let now = Instant::now();
                    if now < target {
                        std::thread::sleep(target - now);
                    }
                    if stop_flag.load(Ordering::Acquire) {
                        break;
                    }
                    if diag {
                        max_late_ms =
                            max_late_ms.max(Instant::now().saturating_duration_since(target).as_secs_f64() * 1e3);
                    }
                    // 该槽发送「当前最近帧」（新鲜或重复；无帧产出跳过——首帧前为启动间隙）。
                    if let Some(frame) = latest.get() {
                        if diag {
                            slots += 1;
                            if last.as_ref().is_some_and(|p| Arc::ptr_eq(p, &frame)) {
                                repeats += 1;
                                run += 1;
                                max_run = max_run.max(run);
                            } else {
                                fresh += 1;
                                run = 0;
                            }
                            last = Some(frame.clone());
                        }
                        if let Err(e) = pipe.write_all(&frame) {
                            let detail = format!("{e:?}");
                            if let Ok(mut se) = shared.send_error.lock() {
                                *se = Some(detail);
                            }
                            // 对端退出（232/109）：停发（worker 经 child 退出归因 E_SIDECAR_EXIT）。
                            write_failed = true;
                            break;
                        }
                        frames_thread.fetch_add(1, Ordering::Release);
                    }
                    k += 1;
                }
                if diag {
                    eprintln!(
                        "[diag-video:{diag_route}] slots={slots} fresh={fresh} repeats={repeats} max_repeat_run={max_run} max_late_ms={max_late_ms:.1} write_failed={write_failed}"
                    );
                }
            })
            .expect("spawn CFR 发送线程");
        SlotSender {
            stop,
            handle: Some(handle),
            frames,
        }
    }

    /// 视频已发送槽数句柄（音频漂移纠正的参照时钟）。
    fn frame_counter(&self) -> Arc<AtomicU64> {
        self.frames.clone()
    }

    /// 停发并 join（EOF 顺序由调用方把控：停发 → 关管道）。
    fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// M4 音频路由配置（§3.4 音轨扩展面；video 保持必填——M4 用例均为窗口目标路由。
/// audio = 隔离模式源列表，每 App 一条独立音轨、互不混音，REQ-17）。
#[derive(Clone, Debug)]
pub struct AudioRouteConfig {
    pub target: RouteTarget,
    pub video: VideoCfg,
    pub audio: Vec<AudioSourceSpec>,
    pub output_dir: PathBuf,
}

/// M16/REQ-12：路级停止原因（端点失效；进入快照/终态事件载荷）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StopNotice {
    /// 错误码（当前仅 `E_TARGET_LOST`）。
    pub code: String,
    /// 失效端点名（友好名；缺名回退 device_id）。
    pub endpoint: Option<String>,
    /// 明细。
    pub detail: String,
}

/// M16/REQ-13：启动时解析后的音源（System 已钉住实际 device_id）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvedAudioSource {
    App { pid: u32, created_at_unix_ms: u64 },
    RenderEndpoint { device_id: String },
    System { device_id: String },
    Mic { device: String },
}

/// M5 多路单路配置（§3.4 RouteConfig 的多路投影；M3/M4 既有 config 字面量不动——
/// 三者为并列公共面）。video 必填（D6 固定画布）；audio 空 = 纯视频路，非空 =
/// 每 App 源一条独立音轨（M4 隔离模式语义）——即 REQ-12 的「视频+可选音频」。
#[derive(Clone, Debug)]
pub struct MultiRouteConfig {
    pub target: RouteTarget,
    pub video: VideoCfg,
    pub audio: Vec<AudioSourceSpec>,
    pub output_dir: PathBuf,
}

/// 启动单路视频路由（M3 既有 API；等价于无音轨的 [`start_audio_route`] 形态）。
pub fn start_route(cfg: RouteConfig) -> Result<RouteHandle, RouteError> {
    start_route_with_options(cfg, false)
}

/// M14：单路视频启动 + 显式保留策略。
///
/// `keep_segments == false`（默认设置）：正常停止且成品验证成功后递归清理会话临时
/// 目录（原段）；`true`：保留原段（调试/排查）。失败路径两种取值都保留原段。
pub fn start_route_with_options(
    cfg: RouteConfig,
    keep_segments: bool,
) -> Result<RouteHandle, RouteError> {
    start_route_impl(
        cfg.target,
        cfg.video,
        Vec::new(),
        cfg.output_dir,
        None,
        keep_segments,
        sr_audio::wasapi::PIPELINE_SAMPLE_RATE,
        crate::naming::OutputContainer::Mkv,
    )
}

/// 启动带音轨的路由（M4）。Ok = 输出开始（§3.1，同 [`start_route`]）；
/// 句柄/收尾语义复用 [`RouteHandle`]（request_stop/wait_finalized/segments）。
/// 隔离模式：每 `App` 源一条独立音轨（aac/输出率/立体声；输出率=设置，默认 48k）；音源 PID 消失 → 该轨
/// 静音继续（墙钟静音续流）、路由不失败（REQ-14/S3 §3.3）。
pub fn start_audio_route(cfg: AudioRouteConfig) -> Result<RouteHandle, RouteError> {
    start_route_impl(
        cfg.target,
        cfg.video,
        cfg.audio,
        cfg.output_dir,
        None,
        false,
        sr_audio::wasapi::PIPELINE_SAMPLE_RATE,
        crate::naming::OutputContainer::Mkv,
    )
}

/// 批量启动多路（§3.3 `start_routes` 的管线层对应物，REQ-12）：**全成功或回滚**。
///
/// - `Ok(handles)`：与 cfgs **同序同长**；每路均已到达「输出开始」（§3.1 启动协议
///   逐路完成；顺序启动合法——红报告 §6.4 scratch 实测 3 路 1080p 顺序启动 <1s）。
/// - `Err`：任一路启动失败（无效目标 → `TargetInvalid`；编码器/授权/环回环境缺失 →
///   对应变体）时，**已启动各路全部停止清理后**才返回 Err——严格回滚（request_stop →
///   wait_finalized → 删除成品/段）；未被批次接受的产物不得提升到输出目录（REQ-24
///   提交语义，m5-red-report §3.3/§8-4）。
/// - 句柄/收尾/错误语义复用 M3/M4（request_stop / wait_finalized / segments /
///   `SidecarExit` 强匹配）。
pub fn start_routes(cfgs: Vec<MultiRouteConfig>) -> Result<Vec<RouteHandle>, RouteError> {
    start_routes_with_options(cfgs, None, false)
}

/// 批量启动（P2：设置中的显式编码器经命令层在此生效）。
///
/// `encoder == Some(name)`：先对锁定 ffmpeg 实际初始化探测该编码器；不可用即
/// `EncoderUnavailable`（**不**静默回退到 D4 链——显式配置必须可观察）。
/// `None`：沿用 D4 链逐级实际初始化（§9 T1）。
pub fn start_routes_with_encoder(
    cfgs: Vec<MultiRouteConfig>,
    encoder: Option<String>,
) -> Result<Vec<RouteHandle>, RouteError> {
    start_routes_with_options(cfgs, encoder, false)
}

/// M14：批量启动 + 显式保留策略（命令层把设置 `keep_segments` 透传至此）。
///
/// 语义与 [`start_routes_with_encoder`] 相同；`keep_segments` 仅影响**正常停止且成品
/// 验证成功后**的原段清理（false = 递归删除会话目录；true = 保留）。失败/回滚路径
/// 不删除原段。
pub fn start_routes_with_options(
    cfgs: Vec<MultiRouteConfig>,
    encoder: Option<String>,
    keep_segments: bool,
) -> Result<Vec<RouteHandle>, RouteError> {
    start_routes_with_options_and_rate(
        cfgs,
        encoder,
        keep_segments,
        sr_audio::wasapi::PIPELINE_SAMPLE_RATE,
    )
}

/// M15/REQ-3：批量启动 + 输出率（捕获率 → 输出率重采样；sidecar `-ar` 参数化）。
///
/// `out_rate` = 设置 `audio_sample_rate`（44.1k/48k/88.2k/96k）。48k 基线行为与
/// [`start_routes_with_options`] 完全一致（捕获率==输出率时重采样器保持恒等直通）。
pub fn start_routes_with_options_and_rate(
    cfgs: Vec<MultiRouteConfig>,
    encoder: Option<String>,
    keep_segments: bool,
    out_rate: u32,
) -> Result<Vec<RouteHandle>, RouteError> {
    start_routes_with_options_rate_and_container(
        cfgs,
        encoder,
        keep_segments,
        out_rate,
        crate::naming::OutputContainer::Mkv,
    )
}

/// M17/REQ-17：批量启动 + 成品容器（启动时冻结进路由配置；`start_routes_with_options_and_rate`
/// 保持 MKV 默认行为不变）。
pub fn start_routes_with_options_rate_and_container(
    cfgs: Vec<MultiRouteConfig>,
    encoder: Option<String>,
    keep_segments: bool,
    out_rate: u32,
    container: crate::naming::OutputContainer,
) -> Result<Vec<RouteHandle>, RouteError> {
    assert!(out_rate > 0, "输出采样率必须为正");
    let mut handles: Vec<RouteHandle> = Vec::new();
    for cfg in cfgs {
        match start_route_impl(
            cfg.target,
            cfg.video,
            cfg.audio,
            cfg.output_dir,
            encoder.as_deref(),
            keep_segments,
            out_rate,
            container,
        ) {
            Ok(h) => handles.push(h),
            Err(e) => {
                // M17/REQ-17：仅批量回滚侧显式取消/丢弃后台收尾；P1-2：取消与提交
                // 通过提交门互斥，并在返回前 join worker 线程——确定性确认收尾线程
                // 与 sidecar/媒体子进程已回收，杜绝迟到成品。
                for h in &handles {
                    h.cancel_finalize_for_rollback();
                    ROLLBACK_CANCELS.fetch_add(1, Ordering::Release);
                    h.request_stop();
                    let mut outcome = h.wait_finalized(Duration::from_secs(120));
                    if matches!(outcome, Err(RouteError::WaitTimeout)) {
                        eprintln!(
                            "[route] 批量回滚：收尾等待超时；继续等待 worker 终止以确认 sidecar 回收"
                        );
                    }
                    h.join_worker();
                    if matches!(outcome, Err(RouteError::WaitTimeout)) {
                        // join 已确认线程终止：结果应在 join 前写入；再读一次（零等待）。
                        outcome = h.wait_finalized(Duration::ZERO);
                    }
                    match outcome {
                        Ok(fin) => {
                            // 严格回滚：删除**本次事务已提交**的成品/段/收尾清单
                            // （Ok 仅由 safe_commit 成功后返回，成品归属本事务）。
                            let _ = std::fs::remove_file(&fin.final_path);
                            for s in &fin.segments {
                                let _ = std::fs::remove_file(s);
                            }
                            let _ = std::fs::remove_file(&fin.segment_list);
                        }
                        Err(e) => {
                            // P1-1：取消/失败的收尾没有提交任何成品——预留路径可能已被
                            // 外部文件占用，绝不能在此删除（只清理本路已见段）。
                            eprintln!("[route] 批量回滚：后台收尾已结束（{e:?}），丢弃已见段");
                            for s in h.segments() {
                                let _ = std::fs::remove_file(s);
                            }
                        }
                    }
                }
                return Err(e);
            }
        }
    }
    Ok(handles)
}

/// M17/REQ-17：L2 确定性注入——remux 阶段暂停（模拟长 remux；批量回滚取消必须生效）。
pub fn set_remux_delay_for_tests(delay: Duration) {
    REMUX_DELAY_MS.store(delay.as_millis() as u64, Ordering::Release);
}

/// M17/REQ-17：L2 确定性注入——启动阶段延迟（模拟慢启动，让同批前序路产出路段）。
pub fn set_start_delay_for_tests(delay: Duration) {
    START_DELAY_MS.store(delay.as_millis() as u64, Ordering::Release);
}

static REMUX_DELAY_MS: AtomicU64 = AtomicU64::new(0);
static START_DELAY_MS: AtomicU64 = AtomicU64::new(0);

/// M18/REQ-21：L2 注入——区域尺寸编码器探测结果（0=实际探测，1=强制通过，2=强制失败）。
static REGION_SIZE_PROBE_OVERRIDE: AtomicU32 = AtomicU32::new(0);

/// M18/REQ-21：L2 确定性注入（`it_region_size_unsupported_rejects`）。
#[doc(hidden)]
pub fn set_region_size_probe_override_for_tests(result: Option<bool>) {
    REGION_SIZE_PROBE_OVERRIDE.store(
        match result {
            None => 0,
            Some(true) => 1,
            Some(false) => 2,
        },
        Ordering::Release,
    );
}

fn region_size_probe_override() -> Option<bool> {
    match REGION_SIZE_PROBE_OVERRIDE.load(Ordering::Acquire) {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

type RegionLatchHook = dyn Fn() + Send + Sync;

fn region_latch_hook_slot() -> &'static Mutex<Option<Arc<RegionLatchHook>>> {
    static SLOT: std::sync::OnceLock<Mutex<Option<Arc<RegionLatchHook>>>> =
        std::sync::OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// M18/REQ-24：L2 屏障式交错注入——worker 读到闩锁为空后、决定收尾前调用。
#[doc(hidden)]
pub fn set_region_latch_hook_for_tests(hook: Option<Arc<dyn Fn() + Send + Sync>>) {
    *region_latch_hook_slot()
        .lock()
        .expect("region latch hook 锁") = hook;
}

fn run_region_latch_hook() {
    let hook = region_latch_hook_slot()
        .lock()
        .ok()
        .and_then(|slot| slot.clone());
    if let Some(hook) = hook {
        hook();
    }
}

/// 收尾事务读取的 remux 注入延迟（生产默认 None）。
pub(crate) fn remux_test_delay() -> Option<Duration> {
    let ms = REMUX_DELAY_MS.load(Ordering::Acquire);
    (ms > 0).then(|| Duration::from_millis(ms))
}

static ROLLBACK_CANCELS: AtomicU64 = AtomicU64::new(0);

/// M17/REQ-17：批量回滚取消计数（L2 断言「回滚确实显式取消后台收尾」）。
#[doc(hidden)]
pub fn rollback_cancel_count_for_tests() -> u64 {
    ROLLBACK_CANCELS.load(Ordering::Acquire)
}

type RouteStartedHook = dyn Fn(&Path) + Send + Sync;

fn route_started_hook_slot() -> &'static Mutex<Option<Arc<RouteStartedHook>>> {
    static SLOT: std::sync::OnceLock<Mutex<Option<Arc<RouteStartedHook>>>> =
        std::sync::OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// P1-1 回归注入：路由成功启动（预留最终名后）回调，测试可在但仅可在该路径写入
/// 外部占位文件，验证回滚失败分支绝不删除外部文件。生产路径永不设置。
#[doc(hidden)]
pub fn set_route_started_hook_for_tests(hook: Option<Arc<dyn Fn(&Path) + Send + Sync>>) {
    *route_started_hook_slot()
        .lock()
        .expect("route started hook 锁") = hook;
}

fn invoke_route_started_hook(final_path: &Path) {
    let hook = route_started_hook_slot()
        .lock()
        .ok()
        .and_then(|slot| slot.clone());
    if let Some(hook) = hook {
        hook(final_path);
    }
}

/// M17/REQ-13：启动时一次解析 System 的实际 device_id，并在轨数/管道分配前按
/// 实际 ID 去重（endpoint==system → 单轨，保留端点失效监控语义）。
pub fn resolve_audio_sources(
    sources: &[AudioSourceSpec],
    default_render_device_id: impl FnOnce() -> Option<String>,
) -> Result<Vec<ResolvedAudioSource>, RouteError> {
    let needs_default = sources.iter().any(|s| matches!(s, AudioSourceSpec::System));
    let pinned_default = if needs_default {
        default_render_device_id().ok_or_else(|| RouteError::LoopbackUnavailable {
            detail: "System 音源：默认渲染端点解析失败（REQ-13 钉住解析）".into(),
        })?
    } else {
        String::new()
    };

    let mut resolved: Vec<ResolvedAudioSource> = sources
        .iter()
        .map(|src| match src {
            AudioSourceSpec::App {
                pid,
                created_at_unix_ms,
            } => ResolvedAudioSource::App {
                pid: *pid,
                created_at_unix_ms: *created_at_unix_ms,
            },
            AudioSourceSpec::System => ResolvedAudioSource::System {
                device_id: pinned_default.clone(),
            },
            AudioSourceSpec::Mic { device } => ResolvedAudioSource::Mic {
                device: device.clone(),
            },
            AudioSourceSpec::RenderEndpoint { device_id } => ResolvedAudioSource::RenderEndpoint {
                device_id: device_id.clone(),
            },
        })
        .collect();

    // 去重发生在轨数/管道分配前：端点优先保留（失效监控语义）。
    let endpoint_ids: std::collections::HashSet<String> = resolved
        .iter()
        .filter_map(|s| match s {
            ResolvedAudioSource::RenderEndpoint { device_id } => Some(device_id.clone()),
            _ => None,
        })
        .collect();
    resolved.retain(|s| match s {
        ResolvedAudioSource::System { device_id } => !endpoint_ids.contains(device_id),
        _ => true,
    });
    let mut seen_endpoints = std::collections::HashSet::new();
    resolved.retain(|s| match s {
        ResolvedAudioSource::RenderEndpoint { device_id } => {
            seen_endpoints.insert(device_id.clone())
        }
        _ => true,
    });
    Ok(resolved)
}

/// 解析后音源 → 采集目标（激活用 `get_device(钉住ID)`）。
/// App 源走进程环回 + 身份校验专用路径，故返回 None。
pub fn capture_target_for(src: &ResolvedAudioSource) -> Option<CaptureTarget> {
    match src {
        ResolvedAudioSource::App { .. } => None,
        ResolvedAudioSource::RenderEndpoint { device_id } => Some(CaptureTarget::RenderEndpoint {
            device_id: device_id.clone(),
        }),
        ResolvedAudioSource::System { device_id } => Some(CaptureTarget::DeviceLoopback {
            device_id: Some(device_id.clone()),
        }),
        ResolvedAudioSource::Mic { device } => Some(CaptureTarget::Microphone {
            device: device.clone(),
        }),
    }
}

struct StartedCapture {
    stream: LoopbackStream,
    /// 显式端点的失效监控/notice 标签（device_id + 友好名）。
    endpoint: Option<(String, String)>,
}

/// 按音源规格启动流式采集（同一采集核；App 先做身份前置校验）。
fn start_audio_stream(src: &ResolvedAudioSource) -> Result<StartedCapture, CaptureError> {
    match src {
        ResolvedAudioSource::App {
            pid,
            created_at_unix_ms,
        } => LoopbackStream::start(*pid, *created_at_unix_ms).map(|stream| StartedCapture {
            stream,
            endpoint: None,
        }),
        ResolvedAudioSource::RenderEndpoint { device_id } => {
            let label = sr_audio::wasapi::endpoint_friendly_name(device_id)
                .unwrap_or_else(|| device_id.clone());
            let stream = LoopbackStream::start_target(CaptureTarget::RenderEndpoint {
                device_id: device_id.clone(),
            })?;
            Ok(StartedCapture {
                stream,
                endpoint: Some((device_id.clone(), label)),
            })
        }
        ResolvedAudioSource::System { device_id } => {
            LoopbackStream::start_target(CaptureTarget::DeviceLoopback {
                device_id: Some(device_id.clone()),
            })
            .map(|stream| StartedCapture {
                stream,
                endpoint: None,
            })
        }
        ResolvedAudioSource::Mic { device } => {
            LoopbackStream::start_target(CaptureTarget::Microphone {
                device: device.clone(),
            })
            .map(|stream| StartedCapture {
                stream,
                endpoint: None,
            })
        }
    }
}

/// 采集启动错误的路级映射（环回不可用 → `LoopbackUnavailable`〔L2 → BLOCKED〕；
/// 其余 → `CaptureFailed`；`MicUnavailable` 的路级专用变体本红未钉，此处明确报错）。
fn capture_start_error(e: CaptureError) -> RouteError {
    match e {
        CaptureError::LoopbackUnavailable { detail } => RouteError::LoopbackUnavailable { detail },
        CaptureError::EndpointInvalid { detail } => RouteError::TargetInvalid { detail },
        other => RouteError::CaptureFailed {
            detail: format!("音频采集启动失败: {other:?}"),
        },
    }
}

// ─────────────────────────── 纯录音（REQ-19：start_audio_only） ───────────────────────────

/// REQ-19 纯录音配置（§3.4 的来源列表形态；无视频/WGC/编码器参与）。
#[derive(Clone, Debug)]
pub struct AudioOnlyConfig {
    pub sources: Vec<AudioSourceSpec>,
    pub output_dir: PathBuf,
}

/// 纯录音成品：与 sources **同序同长**；每来源一条独立 WAV（输出率 s16 立体声，
/// 默认 48k；M15/REQ-3）位于 output_dir 内（文件名为实现自由）。默认成功后其源
/// 会话目录随策略清理（`keep_segments=true` 时保留）。
#[derive(Clone, Debug)]
pub struct AudioOnlyFinalized {
    pub wav_paths: Vec<PathBuf>,
}

struct AudioOnlyShared {
    stop: Arc<AtomicBool>,
    result: Mutex<Option<Result<AudioOnlyFinalized, RouteError>>>,
    cond: Condvar,
}

impl AudioOnlyShared {
    fn set_result(&self, outcome: Result<AudioOnlyFinalized, RouteError>) {
        let mut guard = self.result.lock().expect("audio-only 结果锁");
        if guard.is_none() {
            *guard = Some(outcome);
            self.cond.notify_all();
        }
    }
}

/// 纯录音句柄：request_stop 立即返回「已请求停止」；wait_finalized 返回成品
/// （**停止即成品**：WAV 无 concat/后台收尾）。
pub struct AudioOnlyHandle {
    shared: Arc<AudioOnlyShared>,
}

impl AudioOnlyHandle {
    /// 「已请求停止」：立即返回；写线程在后台停采集并封口 WAV。
    pub fn request_stop(&self) {
        self.shared.stop.store(true, Ordering::Release);
    }

    /// 等待成品（各来源 WAV 封口完成）；超时返回 `WaitTimeout`。
    pub fn wait_finalized(&self, timeout: Duration) -> Result<AudioOnlyFinalized, RouteError> {
        let deadline = Instant::now() + timeout;
        let mut guard = self.shared.result.lock().expect("audio-only 结果锁");
        loop {
            if let Some(outcome) = guard.as_ref() {
                return outcome.clone();
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(RouteError::WaitTimeout);
            }
            let (g, _) = self
                .shared
                .cond
                .wait_timeout(guard, deadline - now)
                .expect("audio-only 结果锁");
            guard = g;
        }
    }
}

impl Drop for AudioOnlyHandle {
    fn drop(&mut self) {
        // 兜底：句柄丢弃即请求停止（写线程自行封口；不阻塞 drop）。
        self.shared.stop.store(true, Ordering::Release);
    }
}

/// 启动纯录音（REQ-19）：1..N 来源 → 各来源独立 WAV（输出率 s16 立体声，默认 48k；
/// M15/REQ-3 写线程重采样；每来源一个采集客户端 + 一个 WAV 写线程）。**停止即成品**：
/// request_stop 后写线程停采集并封口 WAV，wait_finalized 直接返回产物路径（无
/// concat/后台收尾）。
/// 错误映射：环回不可用 → `LoopbackUnavailable`（L2 → BLOCKED）；其余启动失败 →
/// `StartFailed`/`TargetInvalid`/`CaptureFailed`。
///
/// 等价于 [`start_audio_only_with_options`] 且 `keep_segments=false`（M14 默认策略：
/// 成功后清理会话目录）。
pub fn start_audio_only(cfg: AudioOnlyConfig) -> Result<AudioOnlyHandle, RouteError> {
    start_audio_only_with_options(cfg, false)
}

/// M14b：纯录音启动 + 显式保留策略（命令层把设置 `keep_segments` 透传至此）。
///
/// `keep_segments == false`（默认设置）：正常停止且 WAV 安全提交成功后递归清理
/// `<output_dir>/.screenrec_tmp/<session_id>`；`true`：保留会话目录（调试/排查）。
/// 注意：纯录音的 WAV 即成品，安全提交后已移出会话目录；true 保留的是会话目录
/// （manifest 等），不复制成品 WAV。失败/崩溃路径两种取值都保留原段（恢复状态机
/// 扫描不变）。
pub fn start_audio_only_with_options(
    cfg: AudioOnlyConfig,
    keep_segments: bool,
) -> Result<AudioOnlyHandle, RouteError> {
    start_audio_only_with_options_and_rate(
        cfg,
        keep_segments,
        sr_audio::wasapi::PIPELINE_SAMPLE_RATE,
    )
}

/// M15/REQ-3：纯录音 + 输出率——WAV 写线程把实际捕获样本重采样到 `out_rate` 后
/// 写入（WAV 头采样率 = `out_rate`）；`out_rate == 捕获率` 且 0ppm 时重采样器
/// 保持恒等直通（48k 基线行为不变）。
pub fn start_audio_only_with_options_and_rate(
    cfg: AudioOnlyConfig,
    keep_segments: bool,
    out_rate: u32,
) -> Result<AudioOnlyHandle, RouteError> {
    assert!(out_rate > 0, "输出采样率必须为正");
    // App 源身份前置校验（REQ-14/D9：激活前、快速失败；与 start_audio_route 同语义）。
    for src in &cfg.sources {
        if let AudioSourceSpec::App {
            pid,
            created_at_unix_ms,
        } = src
        {
            check_app_identity(*pid, *created_at_unix_ms).map_err(|e| {
                RouteError::TargetInvalid {
                    detail: format!("音频源身份校验失败: {e:?}"),
                }
            })?;
        }
    }
    // M16/REQ-13：纯录音不接受端点音源；System 同样一次解析并钉住实际 device_id。
    if cfg
        .sources
        .iter()
        .any(|s| matches!(s, AudioSourceSpec::RenderEndpoint { .. }))
    {
        return Err(RouteError::TargetInvalid {
            detail: "纯录音不接受 render_endpoint 音源（REQ-13）".into(),
        });
    }
    let resolved_sources = resolve_audio_sources(&cfg.sources, || {
        sr_audio::wasapi::default_render_device_id()
    })?;

    std::fs::create_dir_all(&cfg.output_dir).map_err(|e| RouteError::StartFailed {
        detail: format!("创建输出目录失败（{}）: {e}", cfg.output_dir.display()),
    })?;

    // P1-3：纯录音纳入恢复状态机——真实会话目录 + manifest（streams=["wav"]），
    // WAV 先落会话目录（崩溃后 recovery 可就地封口/修复），正常停止再安全提交到输出根。
    let seq = ROUTE_SEQ.fetch_add(1, Ordering::Relaxed);
    let millis = current_unix_ms();
    let session_id = format!("{millis}_{seq}");
    let session_dir =
        crate::recovery::create_session_dir(&cfg.output_dir, &session_id).map_err(|e| {
            RouteError::StartFailed {
                detail: format!("创建会话临时目录失败: {e:?}"),
            }
        })?;
    let route_id = format!("route-{seq}");
    let manifest = crate::recovery::SessionManifest {
        version: 1,
        session_id: session_id.clone(),
        created_at_unix_ms: millis,
        start_qpc: crate::epoch::qpc_now_hns(),
        routes: vec![crate::recovery::ManifestRoute {
            route_id,
            identity: crate::recovery::RouteIdentity {
                pid: std::process::id(),
                created_at_unix_ms: sr_audio::wasapi::process_created_unix_ms(std::process::id())
                    .unwrap_or(0),
                name: "audio_only".into(),
                path: std::env::current_exe().unwrap_or_default(),
            },
            streams: vec!["wav".to_string()],
            segments: Vec::new(),
            state: "recording".into(),
            final_path: None,
        }],
    };
    if let Err(e) = crate::recovery::save_manifest_atomic(&session_dir, &manifest) {
        let _ = std::fs::remove_dir_all(&session_dir);
        return Err(RouteError::StartFailed {
            detail: format!("写会话 manifest 失败: {e:?}"),
        });
    }

    // 逐源启动采集（同步握手 = 激活成功才 Ok；失败即停已启动流并返回）。
    // M15/REQ-8：Ready 携带实际捕获格式，写线程据此构造重采样器。
    let stop = Arc::new(AtomicBool::new(false));
    let mut streams: Vec<(LoopbackStream, sr_audio::wasapi::CaptureFormat)> = Vec::new();
    for src in &resolved_sources {
        match start_audio_stream(src) {
            Ok(started) => {
                let fmt = started.stream.capture_format();
                streams.push((started.stream, fmt));
            }
            Err(e) => {
                for (s, _) in streams {
                    s.stop();
                }
                let _ = std::fs::remove_dir_all(&session_dir);
                return Err(capture_start_error(e));
            }
        }
    }

    // 每源一条独立 WAV（REQ-19：同序同长；纯分配禁覆盖）+ 一个写线程。
    // M15/REQ-3：WAV 头部采样率 = 输出率（写线程内部把捕获样本重采样到该率）。
    let ts = format_timestamp(current_unix_ms(), local_utc_offset_minutes());
    let mut plans: Vec<(PathBuf, WavWriter)> = Vec::new();
    for i in 0..streams.len() {
        let fmt = streams[i].1;
        let path = next_available(&session_dir, &format!("{ts}_audio{i}"), "wav");
        match WavWriter::create(&path, out_rate, fmt.channels) {
            Ok(w) => plans.push((path, w)),
            Err(e) => {
                for (s, _) in streams {
                    s.stop();
                }
                let _ = std::fs::remove_dir_all(&session_dir);
                return Err(RouteError::StartFailed {
                    detail: format!("创建 WAV 失败（{}）: {e:?}", path.display()),
                });
            }
        }
    }

    // 写线程：写线程独占流（任一退出路径先停采集；停止 → 封口 → 记录路径）。
    let mut streams_opt: Vec<Option<(LoopbackStream, sr_audio::wasapi::CaptureFormat)>> =
        streams.into_iter().map(Some).collect();
    let mut wav_paths: Vec<PathBuf> = Vec::new();
    let mut writers: Vec<std::thread::JoinHandle<Result<(), String>>> = Vec::new();
    for (i, (path, writer)) in plans.into_iter().enumerate() {
        let (stream, capture_format) = streams_opt
            .get_mut(i)
            .and_then(|s| s.take())
            .expect("采集流与写计划同长");
        let stop_flag = stop.clone();
        let spawn = std::thread::Builder::new()
            .name(format!("sr-audio-only-{i}"))
            .spawn(move || audio_only_writer(stream, capture_format, writer, stop_flag, out_rate));
        match spawn {
            Ok(h) => {
                wav_paths.push(path);
                writers.push(h);
            }
            Err(e) => {
                // 写线程启动失败：停掉所有剩余采集流，等待已启动写线程收口。
                stop.store(true, Ordering::Release);
                for slot in streams_opt.iter_mut() {
                    if let Some((s, _)) = slot.take() {
                        s.stop();
                    }
                }
                for h in writers {
                    let _ = h.join();
                }
                return Err(RouteError::Internal {
                    detail: format!("启动 WAV 写线程失败: {e}"),
                });
            }
        }
    }

    // 协调线程：等停止请求 → join 全部写线程（各自已封口）→ 安全提交成品。
    let shared = Arc::new(AudioOnlyShared {
        stop,
        result: Mutex::new(None),
        cond: Condvar::new(),
    });
    let coord_shared = shared.clone();
    let out_dir = cfg.output_dir.clone();
    let session_for_final = session_dir.clone();
    let ts_for_final = ts.clone();
    let coord = std::thread::Builder::new()
        .name(format!("sr-audio-only-final-{millis}"))
        .spawn(move || {
            while !coord_shared.stop.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(20));
            }
            let mut first_err: Option<String> = None;
            for (h, path) in writers.into_iter().zip(wav_paths.iter()) {
                match h.join() {
                    Ok(Ok(())) => {}
                    Ok(Err(detail)) => {
                        first_err.get_or_insert(detail);
                    }
                    Err(_) => {
                        first_err.get_or_insert(format!("WAV 写线程 panic（{}）", path.display()));
                    }
                }
            }
            let outcome = match first_err {
                Some(detail) => Err(RouteError::FinalizeFailed { detail }),
                None => commit_audio_only(
                    &out_dir,
                    &session_for_final,
                    &ts_for_final,
                    &wav_paths,
                    keep_segments,
                )
                .map(|wav_paths| AudioOnlyFinalized { wav_paths }),
            };
            coord_shared.set_result(outcome);
        })
        .map_err(|e| RouteError::Internal {
            detail: format!("启动纯录音协调线程失败: {e}"),
        })?;
    let _ = coord;

    Ok(AudioOnlyHandle { shared })
}

/// P1-3：纯录音正常收尾——会话目录 WAV 已封口 → safe_commit 到输出根 → 清单置 final →
/// 按 M14b 策略清理/保留会话目录。失败时已在会话目录的 WAV 保留（recovery 可修复/重试）。
fn commit_audio_only(
    output_dir: &Path,
    session_dir: &Path,
    ts: &str,
    wav_paths: &[PathBuf],
    keep_segments: bool,
) -> Result<Vec<PathBuf>, RouteError> {
    let verify = |path: &Path| -> Result<(), String> {
        let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if len > 44 {
            Ok(())
        } else {
            Err(format!("WAV 校验失败（{}: {len} 字节）", path.display()))
        }
    };
    let mut committed = Vec::with_capacity(wav_paths.len());
    for (i, src) in wav_paths.iter().enumerate() {
        let final_path = next_available(output_dir, &format!("{ts}_audio{i}"), "wav");
        crate::naming::safe_commit(src, &final_path, verify).map_err(|e| {
            RouteError::FinalizeFailed {
                detail: format!("WAV 安全提交失败: {e:?}"),
            }
        })?;
        committed.push(final_path);
    }
    if let Ok(mut manifest) = crate::recovery::load_manifest(session_dir) {
        if let Some(route) = manifest.routes.first_mut() {
            route.state = "final".into();
            route.final_path = committed.first().cloned();
        }
        let _ = crate::recovery::save_manifest_atomic(session_dir, &manifest);
    }
    // M14b：成品安全提交成功 → 按保留策略清理源会话目录（失败仅日志，不改变本路成功）。
    cleanup_session_after_finalize(session_dir, keep_segments);
    Ok(committed)
}

/// 单来源 WAV 写线程：收包 → 重采样（捕获率 → 输出率）→ 补零/写样本；停止请求 →
/// 停采集 → 冲刷重采样尾部 → 封口（停止即成品）。
fn audio_only_writer(
    stream: LoopbackStream,
    capture_format: sr_audio::wasapi::CaptureFormat,
    mut writer: WavWriter,
    stop: Arc<AtomicBool>,
    out_rate: u32,
) -> Result<(), String> {
    let channels = capture_format.channels as usize;
    let mut resampler = DriftResampler::new(channels, capture_format.sample_rate, out_rate);
    let write_block = |writer: &mut WavWriter,
                       resampler: &mut DriftResampler,
                       pad_frames: u64,
                       samples: &[f32]|
     -> Result<(), String> {
        let pad = pad_frames as usize;
        let frames = samples.len() / channels;
        if pad > 0 || frames > 0 {
            // 补齐帧一并进入重采样（时间轴缩放必须覆盖整段输出）。
            let mut block = Vec::with_capacity((pad + frames) * channels);
            block.resize(pad * channels, 0.0);
            block.extend_from_slice(samples);
            let out = resampler.process(&block);
            if !out.is_empty() {
                writer
                    .write_f32_interleaved(&out)
                    .map_err(|e| format!("WAV 写入失败: {e:?}"))?;
            }
        }
        Ok(())
    };
    let mut stream = Some(stream);
    let outcome = (|| -> Result<(), String> {
        loop {
            if stop.load(Ordering::Acquire) {
                if let Some(s) = stream.take() {
                    s.stop();
                }
                let tail = resampler.flush();
                if !tail.is_empty() {
                    writer
                        .write_f32_interleaved(&tail)
                        .map_err(|e| format!("WAV 写入失败: {e:?}"))?;
                }
                writer
                    .finalize()
                    .map_err(|e| format!("WAV 封口失败: {e:?}"))?;
                return Ok(());
            }
            match stream
                .as_ref()
                .expect("stream 存活期")
                .recv_timeout(Duration::from_millis(200))
            {
                Some(StreamEvent::Pcm { plan, samples }) => {
                    write_block(
                        &mut writer,
                        &mut resampler,
                        plan.pad_frames,
                        samples.as_slice(),
                    )?;
                }
                Some(StreamEvent::EndpointLost { device_id, reason }) => {
                    // M16/REQ-12：端点失效同属运行期终止；封口已写内容并如实报错。
                    if let Some(s) = stream.take() {
                        s.stop();
                    }
                    let tail = resampler.flush();
                    if !tail.is_empty() {
                        writer
                            .write_f32_interleaved(&tail)
                            .map_err(|e| format!("WAV 写入失败: {e:?}"))?;
                    }
                    writer
                        .finalize()
                        .map_err(|e| format!("WAV 封口失败: {e:?}"))?;
                    return Err(format!("音频采集端点失效（{device_id}）: {reason}"));
                }
                Some(StreamEvent::Error(e)) => {
                    // 采集流运行期终止：封口已写内容并如实报错。
                    if let Some(s) = stream.take() {
                        s.stop();
                    }
                    let tail = resampler.flush();
                    if !tail.is_empty() {
                        writer
                            .write_f32_interleaved(&tail)
                            .map_err(|e| format!("WAV 写入失败: {e:?}"))?;
                    }
                    writer
                        .finalize()
                        .map_err(|e| format!("WAV 封口失败: {e:?}"))?;
                    return Err(format!("音频采集流终止: {e:?}"));
                }
                None => {}
            }
        }
    })();
    // 未走到 finalize 的错误路径（写失败）也尝试封口，保证「停止即成品」不产生半成品。
    if outcome.is_err() {
        let _ = writer.finalize();
    }
    outcome
}

enum StreamKind {
    Window(u64),
    Monitor(u32),
    MonitorRegion {
        index: u32,
        crop: RegionCrop,
        full_width: u32,
        full_height: u32,
        loss: CaptureLossLatch,
    },
}

/// M18/REQ-24：区域显示器监视（启动身份 + 启动几何 + 失效闩锁）。
struct RegionWatch {
    display_device: String,
    display_name: String,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    loss: CaptureLossLatch,
}

impl RegionWatch {
    /// 轮询显示器拓扑/几何（路由线程内执行；无新帧/静止桌面不触发）。
    fn poll(&self) -> Result<(), String> {
        let monitors =
            sr_enum::monitors::list_monitors().map_err(|e| format!("显示器枚举失败：{e}"))?;
        let found = monitors.into_iter().find(|m| {
            m.device_path
                .as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case(&self.display_device))
                || m.gdi_device_name
                    .as_deref()
                    .is_some_and(|g| g.eq_ignore_ascii_case(&self.display_device))
        });
        let Some(m) = found else {
            return Err("显示器已移除/失效".to_string());
        };
        if m.x != self.x || m.y != self.y || m.width_px != self.width || m.height_px != self.height
        {
            return Err(format!(
                "显示器几何变化（启动 ({},{}) {}x{}；当前 ({},{}) {}x{}）",
                self.x, self.y, self.width, self.height, m.x, m.y, m.width_px, m.height_px
            ));
        }
        Ok(())
    }
}

/// worker 共享上下文。
struct RunCtx {
    control: Mutex<Option<StreamControl>>,
    pipe: VideoPipe,
    child: Mutex<std::process::Child>,
    sender: Mutex<Option<SlotSender>>,
    audio: Mutex<Vec<AudioTrack>>,
    shared: Arc<Shared>,
    final_path: PathBuf,
    /// 收尾 concat 输入清单（实现写：仅含 >0 字节段）。
    finalize_list: PathBuf,
    /// 诊断：窗口目标 HWND（捕获自然结束时确认窗口是否仍存活）。
    target_hwnd: Option<u64>,
    /// M18/REQ-24：区域显示器监视（路由线程归属，worker 结束时自然停止）。
    region_watch: Option<RegionWatch>,
}

/// 单条音频轨：进程环回流 → 管道写线程（写线程独占 stream，任一退出路径先停采集）。
struct AudioTrack {
    stop_writer: Arc<AtomicBool>,
    writer: Option<std::thread::JoinHandle<()>>,
    pipe: VideoPipe,
}

impl AudioTrack {
    /// 全量关停：停管道写线程（内部先停采集）→ EOF（顺序与视频路一致）。
    fn shutdown(&mut self) {
        self.stop_writer.store(true, Ordering::Release);
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
        self.pipe.close();
    }
}

/// 关停全部音频轨（优雅停止与 sidecar 退出两条路径共用）。
fn shutdown_audio(tracks: &Mutex<Vec<AudioTrack>>) {
    if let Ok(mut tracks) = tracks.lock() {
        for t in tracks.iter_mut() {
            t.shutdown();
        }
    }
}

/// 音频写线程诊断计数（`SR_DIAG_AV=1` 时在停止路径打到 stderr；默认关闭）。
///
/// 覆盖 M12-H 归因所需：WASAPI 包/静音补齐/迟到裁切（`diag-audio`）、管道写耗时、
/// 漂移测量与校正事件、停止冲刷量。计数本身是常数开销，仅 `Instant` 计时与输出受开关控制。
#[derive(Default)]
struct AudioDiag {
    pcm_events: u64,
    pad_frames: u64,
    payload_frames: u64,
    writes: u64,
    write_us_total: u128,
    write_us_max: u128,
    measures: u64,
    corrections: u64,
    ppm_min: f64,
    ppm_max: f64,
    offset_min_ms: f64,
    offset_max_ms: f64,
    flush_frames: u64,
}

impl AudioDiag {
    fn new() -> Self {
        Self {
            ppm_min: f64::INFINITY,
            ppm_max: f64::NEG_INFINITY,
            offset_min_ms: f64::INFINITY,
            offset_max_ms: f64::NEG_INFINITY,
            ..Default::default()
        }
    }

    fn on_pcm(&mut self, pad_frames: u64, payload_frames: u64) {
        self.pcm_events += 1;
        self.pad_frames += pad_frames;
        self.payload_frames += payload_frames;
    }

    fn on_write(&mut self, started: Instant) {
        let us = started.elapsed().as_micros();
        self.writes += 1;
        self.write_us_total += us;
        self.write_us_max = self.write_us_max.max(us);
    }

    fn on_measure(&mut self, offset_ms: f64, ppm: f64) {
        self.measures += 1;
        self.offset_min_ms = self.offset_min_ms.min(offset_ms);
        self.offset_max_ms = self.offset_max_ms.max(offset_ms);
        self.ppm_min = self.ppm_min.min(ppm);
        self.ppm_max = self.ppm_max.max(ppm);
        if ppm != 0.0 {
            self.corrections += 1;
        }
    }

    /// `exit`：stop / write-error / capture-error（诊断归因）。
    fn report(&self, track: usize, exit: &str) {
        let measured = self.measures > 0;
        let (ppm_min, ppm_max) = if measured {
            (self.ppm_min, self.ppm_max)
        } else {
            (0.0, 0.0)
        };
        let (off_min, off_max) = if measured {
            (self.offset_min_ms, self.offset_max_ms)
        } else {
            (0.0, 0.0)
        };
        let write_avg_ms = if self.writes > 0 {
            self.write_us_total as f64 / self.writes as f64 / 1e3
        } else {
            0.0
        };
        eprintln!(
            "[diag-audio-pipe] a{track}: exit={exit} pcm={} pad_frames={} payload_frames={} writes={} write_avg_ms={write_avg_ms:.2} write_max_ms={:.2} measures={} corrections={} ppm=[{ppm_min:.0},{ppm_max:.0}] offset_ms=[{off_min:.1},{off_max:.1}] flush_frames={}",
            self.pcm_events,
            self.pad_frames,
            self.payload_frames,
            self.writes,
            self.write_us_max as f64 / 1e3,
            self.measures,
            self.corrections,
            self.flush_frames,
        );
    }
}

/// 进程内已预留的成品路径（同秒同目标并发启动时避免两路拿到同一最终名）。
static FINAL_RESERVATIONS: std::sync::OnceLock<Mutex<std::collections::HashSet<PathBuf>>> =
    std::sync::OnceLock::new();

fn final_reservations() -> &'static Mutex<std::collections::HashSet<PathBuf>> {
    FINAL_RESERVATIONS.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

/// 分配并**预留**成品路径（`{stem}.{ext}`/`{stem}_N.{ext}`；存在或已预留 → 递增 N）。
/// 预留跨「启动 → 收尾」窗口：收尾完成/失败后由 [`release_final_path`] 释放。
/// M17/REQ-17：`ext` 来自启动冻结的容器（预留从启动即按 `.mp4`/`.mkv`）。
fn reserve_final_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut reserved = final_reservations().lock().expect("成品预留锁");
    let mut n: u32 = 0;
    loop {
        let candidate = if n == 0 {
            dir.join(format!("{stem}.{ext}"))
        } else {
            dir.join(format!("{stem}_{n}.{ext}"))
        };
        if !candidate.exists() && !reserved.contains(&candidate) {
            reserved.insert(candidate.clone());
            return candidate;
        }
        n = n.checked_add(1).expect("序号溢出：目录内同名条目过多");
    }
}

fn release_final_path(path: &Path) {
    if let Ok(mut reserved) = final_reservations().lock() {
        reserved.remove(path);
    }
}

/// 预留守卫：收尾函数任意路径返回时释放（成功时成品已存在，释放不影响后续分配）。
struct FinalReservation(PathBuf);
impl Drop for FinalReservation {
    fn drop(&mut self) {
        release_final_path(&self.0);
    }
}

/// M18/REQ-22：解析区域目标（稳定 device_path / GDI szDevice → 单显示器 + 帧内 crop）。
fn resolve_region(
    display_device: &str,
    rect: RegionRect,
) -> Result<(u32, RegionCrop, RegionWatch), RouteError> {
    let key = display_device.trim();
    if key.is_empty() {
        return Err(RouteError::TargetInvalid {
            detail: "区域目标缺少 display_device（device_path 与 GDI szDevice 均不可得）".into(),
        });
    }
    let monitors = sr_enum::monitors::list_monitors().map_err(|e| RouteError::TargetInvalid {
        detail: format!("显示器枚举失败：{e}"),
    })?;
    let monitor = monitors
        .into_iter()
        .find(|m| {
            m.device_path
                .as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case(key))
                || m.gdi_device_name
                    .as_deref()
                    .is_some_and(|g| g.eq_ignore_ascii_case(key))
        })
        .ok_or_else(|| RouteError::TargetInvalid {
            detail: format!("显示器不存在或已移除（display_device={key}）"),
        })?;
    let crop = region_crop_rect(
        rect,
        monitor.x,
        monitor.y,
        monitor.width_px,
        monitor.height_px,
    )
    .map_err(|detail| RouteError::TargetInvalid { detail })?;
    let display_name = monitor
        .friendly_name
        .clone()
        .or_else(|| monitor.gdi_device_name.clone())
        .unwrap_or_else(|| format!("Monitor {}", monitor.index));
    let loss = CaptureLossLatch::new();
    let watch = RegionWatch {
        display_device: key.to_string(),
        display_name,
        x: monitor.x,
        y: monitor.y,
        width: monitor.width_px,
        height: monitor.height_px,
        loss: loss.clone(),
    };
    Ok((monitor.index, crop, watch))
}

/// 启动路由核心（视频 + 可选音频轨；M3/M4 共用编排）。
fn start_route_impl(
    target: RouteTarget,
    video: VideoCfg,
    audio: Vec<AudioSourceSpec>,
    output_dir: PathBuf,
    encoder_override: Option<&str>,
    keep_segments: bool,
    out_rate: u32,
    container: crate::naming::OutputContainer,
) -> Result<RouteHandle, RouteError> {
    let start_delay = START_DELAY_MS.load(Ordering::Acquire);
    if start_delay > 0 {
        std::thread::sleep(Duration::from_millis(start_delay));
    }
    let seq = ROUTE_SEQ.fetch_add(1, Ordering::Relaxed);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    // ── 目标解析（M18：区域 = 单显示器内帧内 crop；窗口/显示器路径不变） ──
    let mut region_watch: Option<RegionWatch> = None;
    let (stream_kind, target_name) = match &target {
        RouteTarget::Window { hwnd } => (StreamKind::Window(*hwnd), window_title(*hwnd)),
        RouteTarget::Monitor { index } => (StreamKind::Monitor(*index), format!("Monitor {index}")),
        RouteTarget::Region {
            display_device,
            rect,
        } => {
            let (index, crop, watch) = resolve_region(display_device, *rect)?;
            let (full_width, full_height) = (watch.width, watch.height);
            let loss = watch.loss.clone();
            let target_name = format!(
                "Region {}x{} ({})",
                crop.width(),
                crop.height(),
                watch.display_name
            );
            region_watch = Some(watch);
            (
                StreamKind::MonitorRegion {
                    index,
                    crop,
                    full_width,
                    full_height,
                    loss,
                },
                target_name,
            )
        }
    };
    let is_region = matches!(stream_kind, StreamKind::MonitorRegion { .. });

    // REQ-23：区域目标必须至少一个音源（命令层矩阵的管线侧兜底；不静默录无声）。
    if is_region && audio.is_empty() {
        return Err(RouteError::TargetInvalid {
            detail: "区域目标没有任何音源（REQ-23：至少选择 1 个应用/系统/麦克风音源）".into(),
        });
    }

    // ── 音频源解析（M6：App=进程环回 / System=默认端点设备环回 REQ-15 / Mic=采集
    //     设备 REQ-16；全音轨 = 给定来源列表各一条独立轨、互不混音，REQ-17）。 ──
    let app_sources: Vec<(u32, u64)> = audio
        .iter()
        .filter_map(|src| match src {
            AudioSourceSpec::App {
                pid,
                created_at_unix_ms,
            } => Some((*pid, *created_at_unix_ms)),
            _ => None,
        })
        .collect();

    // ── 编码器（D4 链逐级实际初始化；P2：显式设置优先且须实际初始化成功） ──
    let encoder = match encoder_override {
        Some(name) => {
            if !crate::sidecar::probe_named_encoder(name) {
                return Err(RouteError::EncoderUnavailable);
            }
            name.to_string()
        }
        None => detect_encoder().ok_or(RouteError::EncoderUnavailable)?,
    };
    // REQ-21：区域路以真实区域尺寸初始化编码器（探测仍 256×256；不支持 → 失败不缩放/补边）。
    if let StreamKind::MonitorRegion { crop, .. } = &stream_kind {
        let supported = region_size_probe_override().unwrap_or_else(|| {
            crate::sidecar::probe_encoder_size(&encoder, crop.width(), crop.height())
        });
        if !supported {
            return Err(RouteError::RegionSizeUnsupported {
                width: crop.width(),
                height: crop.height(),
            });
        }
    }

    // ── 音频源身份前置校验（REQ-14/D9：激活前、快速失败） ──
    for (pid, created) in &app_sources {
        check_app_identity(*pid, *created).map_err(|e| RouteError::TargetInvalid {
            detail: format!("音频源身份校验失败: {e:?}"),
        })?;
    }

    // ── M16/REQ-13：一次解析并钉住 System device_id；端点启动校验（存在且为 Render）。
    //     必须在轨数/管道分配与输出布局之前完成 —— 失败不产生路段。 ──
    let resolved_audio =
        resolve_audio_sources(&audio, || sr_audio::wasapi::default_render_device_id())?;
    for src in &resolved_audio {
        if let ResolvedAudioSource::RenderEndpoint { device_id } = src {
            sr_audio::wasapi::validate_render_endpoint(device_id).map_err(|e| match e {
                CaptureError::EndpointInvalid { detail } => RouteError::TargetInvalid { detail },
                other => RouteError::CaptureFailed {
                    detail: format!("端点启动校验失败: {other:?}"),
                },
            })?;
        }
    }

    // ── 输出布局（REQ-24/D12：成品名 = {ts}_{sanitize(目标名)}.mkv；纯分配禁覆盖；
    //     段/ffconcat 清单落会话临时目录 <output>/.screenrec_tmp/<session_id>；
    //     会话开始时创建并写 manifest（原子；REQ-22 恢复锚点）。 ──
    std::fs::create_dir_all(&output_dir).map_err(|e| RouteError::StartFailed {
        detail: format!("创建输出目录失败（{}）: {e}", output_dir.display()),
    })?;
    let token = sanitize_target_name(&target_name);
    let ts = format_timestamp(current_unix_ms(), local_utc_offset_minutes());
    let final_path =
        reserve_final_path(&output_dir, &format!("{ts}_{token}"), container.extension());
    // 启动失败路径释放预留；成功时把预留移交给收尾函数（`finalize` 内的守卫）。
    let final_reservation = FinalReservation(final_path.clone());
    let seg_prefix = format!("out{seq}_");
    let session_id = format!("{millis}_{seq}");
    let session_dir =
        crate::recovery::create_session_dir(&output_dir, &session_id).map_err(|e| {
            RouteError::StartFailed {
                detail: format!("创建会话临时目录失败: {e:?}"),
            }
        })?;
    let route_id = format!("route-{seq}");
    let manifest = crate::recovery::SessionManifest {
        version: 1,
        session_id: session_id.clone(),
        created_at_unix_ms: millis,
        start_qpc: crate::epoch::qpc_now_hns(),
        routes: vec![crate::recovery::ManifestRoute {
            route_id: route_id.clone(),
            identity: crate::recovery::RouteIdentity {
                pid: std::process::id(),
                created_at_unix_ms: sr_audio::wasapi::process_created_unix_ms(std::process::id())
                    .unwrap_or(0),
                name: target_name.clone(),
                path: std::env::current_exe().unwrap_or_default(),
            },
            streams: {
                let mut s = vec!["v".to_string()];
                for k in 0..resolved_audio.len() {
                    s.push(format!("a{k}"));
                }
                s
            },
            segments: Vec::new(),
            state: "recording".into(),
            final_path: None,
        }],
    };
    crate::recovery::save_manifest_atomic(&session_dir, &manifest).map_err(|e| {
        RouteError::StartFailed {
            detail: format!("写会话 manifest 失败: {e:?}"),
        }
    })?;
    let segment_list = session_dir.join(format!("segments{seq}.ffconcat"));
    let pipe_name = format!(
        r"\\.\pipe\screenrec_{}_{}_{}_v",
        std::process::id(),
        millis,
        seq
    );

    // ── 启动协议：建管（视频 8MiB + 每音源一条 f32le 小缓冲管道）→ spawn sidecar
    //     → 外置超时连接 → 捕获启动。写超时 = S1b 冻结下限（3s）之上取 5s
    //     （PIPE_WRITE_TIMEOUT；仅覆盖「活但不读」，对端死亡由内核即时解除）。 ──
    let video_opts = PipeOptions {
        buffer_bytes: sr_capture::pipe::VIDEO_PIPE_BUFFER_BYTES,
        write_timeout: Some(PIPE_WRITE_TIMEOUT),
    };
    let pipe =
        VideoPipe::create_with(&pipe_name, video_opts).map_err(|e| RouteError::StartFailed {
            detail: format!("建管失败（{pipe_name}）: {e:?}"),
        })?;
    let audio_opts = PipeOptions {
        buffer_bytes: AUDIO_PIPE_BUFFER_BYTES,
        write_timeout: Some(PIPE_WRITE_TIMEOUT),
    };
    let mut audio_pipes = Vec::new();
    for k in 0..resolved_audio.len() {
        let name = format!(
            r"\\.\pipe\screenrec_{}_{}_{}_a{k}",
            std::process::id(),
            millis,
            seq
        );
        audio_pipes.push(VideoPipe::create_with(&name, audio_opts).map_err(|e| {
            RouteError::StartFailed {
                detail: format!("建管失败（{name}）: {e:?}"),
            }
        })?);
    }

    let (encoder_width, encoder_height) = match &stream_kind {
        StreamKind::MonitorRegion { crop, .. } => (crop.width(), crop.height()),
        _ => (video.canvas_w & !1, video.canvas_h & !1),
    };
    let video_spec = VideoPipeSpec {
        width: encoder_width,
        height: encoder_height,
        fps: video.fps,
        pipe_path: pipe.name().to_string(),
    };
    let audio_pipe_specs: Vec<AudioPipeSpec> = audio_pipes
        .iter()
        .map(|p| AudioPipeSpec {
            pipe_path: p.name().to_string(),
            sample_rate: out_rate,
        })
        .collect();
    let args = build_encode_args(
        &video_spec,
        &audio_pipe_specs,
        &encoder,
        SEGMENT_SECS,
        &segment_list,
        &session_dir.join(format!("{seg_prefix}%05d.mkv")),
    );
    let mut child = spawn_sidecar(&args).map_err(|e| RouteError::StartFailed {
        detail: format!("spawn sidecar 失败: {e}"),
    })?;
    let stderr_tail = Arc::new(Mutex::new(String::new()));
    let stderr_drainer = spawn_stderr_drainer(&mut child, stderr_tail.clone());

    // 连接（外置超时；ffmpeg 按 argv 序开输入：视频 → 各音频管道）。
    if let Err(e) = pipe.connect_wait(CONNECT_TIMEOUT) {
        let _ = child.kill();
        let _ = child.wait();
        pipe.close();
        for p in &audio_pipes {
            p.close();
        }
        return Err(match e {
            PipeConnectError::Timeout => RouteError::StartFailed {
                detail: format!("sidecar {CONNECT_TIMEOUT:?} 内未连接视频管道（外置超时）"),
            },
            PipeConnectError::Failed { detail } => RouteError::StartFailed { detail },
        });
    }

    let shared = Arc::new(Shared {
        stop: AtomicBool::new(false),
        stop_notice: Mutex::new(None),
        region_loss: region_watch.as_ref().map(|w| w.loss.clone()),
        capture_ended_for_tests: AtomicBool::new(false),
        force_finalize_failure: AtomicBool::new(false),
        finalize_control: crate::finalize::FinalizeControl::new(),
        container,
        output_dir: session_dir.clone(),
        session_dir,
        route_id,
        final_path: final_path.clone(),
        keep_segments,
        seg_prefix,
        result: Mutex::new(None),
        cond: Condvar::new(),
        stderr_tail,
        send_error: Mutex::new(None),
        worker: Mutex::new(None),
    });

    // ── 捕获会话启动（回调只更新最近帧槽；管道写由 CFR 发送器执行） ──
    let latest = LatestFrame::new();
    let target_hwnd = match &stream_kind {
        StreamKind::Window(hwnd) => Some(*hwnd),
        StreamKind::Monitor(_) | StreamKind::MonitorRegion { .. } => None,
    };
    let control = match &stream_kind {
        StreamKind::Window(hwnd) => wgc::start_window_stream(
            *hwnd,
            video_spec.width,
            video_spec.height,
            video_spec.fps,
            video.cursor,
            latest.clone(),
        ),
        StreamKind::Monitor(index) => wgc::start_monitor_stream(
            *index,
            video_spec.width,
            video_spec.height,
            video_spec.fps,
            video.cursor,
            latest.clone(),
        ),
        StreamKind::MonitorRegion {
            index,
            crop,
            full_width,
            full_height,
            loss,
        } => wgc::start_region_stream(
            *index,
            *full_width,
            *full_height,
            *crop,
            video_spec.fps,
            video.cursor,
            latest.clone(),
            loss.clone(),
        ),
    };
    let control = match control {
        Ok(c) => c,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            pipe.close();
            for p in &audio_pipes {
                p.close();
            }
            return Err(match e {
                StreamStartError::TargetInvalid { detail } => RouteError::TargetInvalid { detail },
                StreamStartError::ConsentDenied => RouteError::ConsentDenied,
                StreamStartError::Failed { detail } => RouteError::CaptureFailed { detail },
            });
        }
    };

    // ── CFR 槽发送器（30fps 节拍：最近帧，无新帧则重复；§3.3 连续 NV12 流）。
    //     音频管道的连接必须等发送器供数：ffmpeg 的 find_stream_info 需先读到
    //     input 0 的数据才会开启 input 1（双输入连接序，M4 实测）。 ──
    let mut sender = SlotSender::start(
        pipe.clone(),
        latest,
        Duration::from_secs_f64(1.0 / video_spec.fps as f64),
        shared.clone(),
    );

    // 音频管道连接（ffmpeg input 1..n；发送器已供数 ⇒ 连接在毫秒级完成）。
    for (k, p) in audio_pipes.iter().enumerate() {
        if let Err(e) = p.connect_wait(CONNECT_TIMEOUT) {
            sender.stop();
            let _ = control.stop(); // 消费并 join 捕获线程（启动失败回滚）
            let _ = child.kill();
            let _ = child.wait();
            pipe.close();
            p.close();
            for q in audio_pipes.iter().skip(k + 1) {
                q.close();
            }
            return Err(match e {
                PipeConnectError::Timeout => RouteError::StartFailed {
                    detail: format!(
                        "sidecar {CONNECT_TIMEOUT:?} 内未连接音频管道 a{k}（外置超时）"
                    ),
                },
                PipeConnectError::Failed { detail } => RouteError::StartFailed { detail },
            });
        }
    }

    // ── 音频轨：流式采集（App=进程环回 / System=设备环回 REQ-15 / Mic=麦克风 REQ-16，
    //     同一采集核）→ 漂移纠正（REQ-26/D7：Timeline + DriftController + 变比重采样）
    //     → 管道写线程（f32le/输出率/2ch；音源消失 → 墙钟静音续流，REQ-14/S3 §3.3）。
    //     激活失败在此显式化（E_LOOPBACK_UNAVAILABLE 等）。 ──
    let mut audio_tracks: Vec<AudioTrack> = Vec::new();
    let video_frames = sender.frame_counter();
    let video_fps = video_spec.fps;
    let drift_timeline = crate::epoch::Timeline::new(
        crate::epoch::qpc_frequency(),
        crate::epoch::qpc_now_ticks(),
        current_unix_ms(),
    );
    for (k, (src, apipe)) in resolved_audio
        .iter()
        .zip(audio_pipes.into_iter())
        .enumerate()
    {
        let started = match start_audio_stream(src) {
            Ok(s) => s,
            Err(e) => {
                shutdown_audio(&Mutex::new(audio_tracks));
                let _ = child.kill();
                let _ = child.wait();
                sender.stop();
                pipe.close();
                apipe.close();
                return Err(capture_start_error(e));
            }
        };
        let capture_format = started.stream.capture_format();
        let endpoint_watch = started.endpoint;
        let stop_writer = Arc::new(AtomicBool::new(false));
        let writer_stop = stop_writer.clone();
        let shared_for_writer = shared.clone();
        let writer_pipe = apipe.clone();
        let writer_frames = video_frames.clone();
        let writer = {
            std::thread::Builder::new()
                .name(format!("sr-audio-pipe-{k}"))
                .spawn(move || {
                    // 写线程独占 stream：任一退出路径先停采集（join 内部线程）再返回。
                    let mut stream = Some(started.stream);
                    // M16/REQ-12：显式端点 → 订阅失效通知（IMMNotificationClient 分发；
                    // 测试注入走同一分发路径）。System/Mic/App 仍由 HRESULT 路径覆盖。
                    let endpoint_monitor = endpoint_watch
                        .as_ref()
                        .map(|(device_id, _)| sr_audio::wasapi::DeviceMonitor::watch(device_id));
                    let stop_stream = |stream: &mut Option<LoopbackStream>| {
                        if let Some(s) = stream.take() {
                            s.stop();
                        }
                    };
                    // REQ-26：每轨独立漂移决策 + 变比重采样；参照 = 视频 CFR 槽计数，
                    // Timeline 提供 T0/墙钟兜底（视频尚未出帧时）。
                    // M15/REQ-6/8：基础比率 = 输出率/实际捕获率；输入计数在捕获率域，
                    // 漂移比较统一换算到秒。
                    let capture_rate = capture_format.sample_rate;
                    let channels = capture_format.channels as usize;
                    let mut drift = DriftController::new(DriftCfg {
                        enter_ms: DRIFT_ENTER_MS,
                        exit_ms: DRIFT_EXIT_MS,
                        rate_limit_ppm: DRIFT_LIMIT_PPM,
                    });
                    let mut resampler = DriftResampler::new(channels, capture_rate, out_rate);
                    let mut device_frames: u64 = 0;
                    let mut base_video: Option<u64> = None;
                    let mut base_rel_hns: i64 = 0;
                    let mut next_measure = Instant::now();
                    let mut diag = AudioDiag::new();
                    let diag_on = sr_audio::wasapi::diag_enabled();
                    loop {
                        // M16/REQ-12：通知链（OnDeviceRemoved/StateChanged）检测。
                        if let Some(monitor) = &endpoint_monitor {
                            if let Some(loss) = monitor.try_recv() {
                                diag.flush_frames += handle_endpoint_loss(
                                    &shared_for_writer,
                                    &mut stream,
                                    &writer_pipe,
                                    &mut resampler,
                                    channels,
                                    &endpoint_watch,
                                    &loss.device_id,
                                    &loss.reason,
                                ) as u64;
                                if diag_on {
                                    diag.report(k, "endpoint-lost");
                                }
                                return;
                            }
                        }
                        if writer_stop.load(Ordering::Acquire) {
                            stop_stream(&mut stream);
                            // REQ-26 停止冲刷：音频管道关闭/EOF 之前补写重采样器
                            // 尚未吐出的尾部（直通 1 帧 / rubato 滤波延迟量级）。
                            diag.flush_frames += flush_audio_tail(
                                &writer_pipe,
                                &shared_for_writer,
                                &mut resampler,
                                channels,
                            ) as u64;
                            if diag_on {
                                diag.report(k, "stop");
                            }
                            return;
                        }
                        match stream
                            .as_ref()
                            .expect("stream 存活期")
                            .recv_timeout(Duration::from_millis(200))
                        {
                            Some(StreamEvent::Pcm { plan, samples }) => {
                                if base_video.is_none() {
                                    base_video = Some(writer_frames.load(Ordering::Acquire));
                                    base_rel_hns = drift_timeline
                                        .map(|t| t.qpc_to_rel_hns(crate::epoch::qpc_now_ticks()))
                                        .unwrap_or(0);
                                }
                                let pad_frames = plan.pad_frames as usize;
                                let payload_frames = samples.len() / channels;
                                diag.on_pcm(pad_frames as u64, payload_frames as u64);
                                if pad_frames > 0 || payload_frames > 0 {
                                    // 补齐帧一并进入重采样（时间轴缩放必须覆盖整段输出）。
                                    let mut block = Vec::with_capacity(
                                        (pad_frames + payload_frames) * channels,
                                    );
                                    block.resize(pad_frames * channels, 0.0);
                                    block.extend_from_slice(&samples);
                                    let out = resampler.process(&block);
                                    device_frames += (pad_frames + payload_frames) as u64;
                                    if !out.is_empty() {
                                        let t_write = diag_on.then(Instant::now);
                                        if let Err(e) = writer_pipe.write_all(&bytes_of_f32(&out)) {
                                            record_send_error(&shared_for_writer, &e);
                                            stop_stream(&mut stream);
                                            if diag_on {
                                                diag.report(k, "write-error");
                                            }
                                            return;
                                        }
                                        if let Some(t) = t_write {
                                            diag.on_write(t);
                                        }
                                    }
                                }
                                // 漂移测量（节流 200ms）：内容偏差 = 参照秒 − 已编排秒。
                                if next_measure.elapsed() >= DRIFT_MEASURE_INTERVAL {
                                    let vf = writer_frames
                                        .load(Ordering::Acquire)
                                        .saturating_sub(base_video.unwrap_or(0));
                                    let reference_secs = if vf >= 2 {
                                        vf as f64 / video_fps as f64
                                    } else {
                                        drift_timeline
                                            .map(|t| {
                                                (t.qpc_to_rel_hns(crate::epoch::qpc_now_ticks())
                                                    - base_rel_hns)
                                                    as f64
                                                    / crate::epoch::HNS_PER_SEC as f64
                                            })
                                            .unwrap_or(device_frames as f64 / capture_rate as f64)
                                    };
                                    let offset_ms = content_offset_ms(
                                        reference_secs,
                                        device_frames,
                                        capture_rate,
                                    );
                                    let action = drift.on_measurement(offset_ms);
                                    let ppm = match action {
                                        DriftAction::Correct { rate_ppm } => rate_ppm,
                                        DriftAction::Hold => 0.0,
                                    };
                                    diag.on_measure(offset_ms, ppm);
                                    resampler.set_rate_ppm(ppm);
                                    next_measure = Instant::now();
                                }
                            }
                            Some(StreamEvent::EndpointLost { device_id, reason }) => {
                                diag.flush_frames += handle_endpoint_loss(
                                    &shared_for_writer,
                                    &mut stream,
                                    &writer_pipe,
                                    &mut resampler,
                                    channels,
                                    &endpoint_watch,
                                    &device_id,
                                    &reason,
                                ) as u64;
                                if diag_on {
                                    diag.report(k, "endpoint-lost");
                                }
                                return;
                            }
                            Some(StreamEvent::Error(e)) => {
                                record_send_error(
                                    &shared_for_writer,
                                    &sr_capture::pipe::PipeWriteError::Io {
                                        detail: format!("音频采集流终止: {e:?}"),
                                    },
                                );
                                stop_stream(&mut stream);
                                // 采集流自然终止同属停止路径：EOF 前同样冲刷尾部。
                                diag.flush_frames += flush_audio_tail(
                                    &writer_pipe,
                                    &shared_for_writer,
                                    &mut resampler,
                                    channels,
                                ) as u64;
                                if diag_on {
                                    diag.report(k, "capture-error");
                                }
                                return;
                            }
                            None => {}
                        }
                    }
                })
                .map_err(|e| RouteError::Internal {
                    detail: format!("启动音频写线程失败: {e}"),
                })?
        };
        audio_tracks.push(AudioTrack {
            stop_writer,
            writer: Some(writer),
            pipe: apipe,
        });
    }

    // ── worker 线程：停止/退出监听 + EOF 优雅停止 + 后台 concat 收尾 ──
    let worker_shared = shared.clone();
    let worker = std::thread::Builder::new()
        .name(format!("sr-route-worker-{seq}"))
        .spawn(move || {
            let outcome = run_worker(RunCtx {
                control: Mutex::new(Some(control)),
                pipe,
                child: Mutex::new(child),
                sender: Mutex::new(Some(sender)),
                audio: Mutex::new(audio_tracks),
                shared: worker_shared.clone(),
                final_path,
                finalize_list: segment_list.with_extension("finalize.ffconcat"),
                target_hwnd,
                region_watch,
            });
            let _ = stderr_drainer.join();
            worker_shared.set_result(outcome);
        })
        .map_err(|e| RouteError::Internal {
            detail: format!("启动 route worker 失败: {e}"),
        })?;
    if let Ok(mut slot) = shared.worker.lock() {
        *slot = Some(worker);
    }
    std::mem::forget(final_reservation); // 预留移交后台收尾（成功/失败由 finalize 守卫释放）

    invoke_route_started_hook(&shared.final_path);
    Ok(RouteHandle { shared })
}

/// f32 交错样本 → 管道字节（little-endian；f32le 约定）。
fn bytes_of_f32(samples: &[f32]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}

/// 漂移内容偏差（毫秒；正 = 音频落后基准）：统一换算到**秒域**比较
/// （`reference_secs − captured_frames / capture_rate`）。REQ-8：不得再把原始
/// 捕获帧固定解释为 48k（混合率会制造虚假校正）。
fn content_offset_ms(reference_secs: f64, captured_frames: u64, capture_rate: u32) -> f64 {
    assert!(capture_rate > 0, "capture_rate 必须为正");
    (reference_secs - captured_frames as f64 / capture_rate as f64) * 1000.0
}

/// REQ-26 停止冲刷：把重采样器尾部写入音频管道（EOF 之前；失败仅记诊断，
/// 不改变停止/收尾归因）。重复调用由 `DriftResampler::flush` 幂等保证。
/// 返回冲刷的帧数（诊断：EOF 前的尾量；`channels` = 实际捕获声道数）。
fn flush_audio_tail(
    pipe: &VideoPipe,
    shared: &Shared,
    resampler: &mut DriftResampler,
    channels: usize,
) -> usize {
    let tail = resampler.flush();
    if tail.is_empty() {
        return 0;
    }
    if let Err(e) = pipe.write_all(&bytes_of_f32(&tail)) {
        record_send_error(shared, &e);
    }
    tail.len() / channels
}

/// M16/REQ-12：写线程收到端点失效 → 记录共享 stop_notice（首因）→ shared.stop
/// （worker 走既有优雅停路）→ 停采集并冲刷重采样尾部。返回冲刷帧数。
fn handle_endpoint_loss(
    shared: &Shared,
    stream: &mut Option<LoopbackStream>,
    pipe: &VideoPipe,
    resampler: &mut DriftResampler,
    channels: usize,
    endpoint_watch: &Option<(String, String)>,
    device_id: &str,
    reason: &str,
) -> usize {
    let endpoint = endpoint_watch
        .as_ref()
        .map(|(_, label)| label.clone())
        .unwrap_or_else(|| device_id.to_string());
    shared.set_stop_notice(StopNotice {
        code: "E_TARGET_LOST".to_string(),
        endpoint: Some(endpoint),
        detail: reason.to_string(),
    });
    if let Some(s) = stream.take() {
        s.stop();
    }
    flush_audio_tail(pipe, shared, resampler, channels)
}

/// 记录音频管道写错误（诊断；对端死亡归因仍以 child 退出码为准）。
fn record_send_error(shared: &Shared, e: &sr_capture::pipe::PipeWriteError) {
    if let Ok(mut se) = shared.send_error.lock() {
        *se = Some(format!("{e:?}"));
    }
}

/// worker 主流程：停止请求 / sidecar 退出 / 捕获自然结束 三路归因，然后收尾。
fn run_worker(ctx: RunCtx) -> Result<Finalized, RouteError> {
    let mut last_refresh = Instant::now();
    let mut last_region_poll = Instant::now();
    loop {
        // 1) 停止请求 → 优雅停止（EOF）→ 收尾。
        if ctx.shared.stop.load(Ordering::Acquire) {
            if sr_audio::wasapi::diag_enabled() {
                eprintln!("[diag-route] stop path: request_stop");
            }
            return graceful_stop(ctx);
        }
        // 2) sidecar 退出（未请求停止）→ E_SIDECAR_EXIT（保留分段，§3.5）。
        if let Some(code) = child_exit_code(&ctx.child) {
            if sr_audio::wasapi::diag_enabled() {
                eprintln!("[diag-route] stop path: sidecar exited code={code}");
            }
            stop_capture(&ctx.control);
            shutdown_audio(&ctx.audio);
            if code == 0 {
                // 无 EOF 的 0 退出（异常但段可能有效）：按已产出段尝试收尾。
                return finalize(&ctx);
            }
            return Err(sidecar_exit(&ctx, code));
        }
        // 3) 捕获自然结束（目标关闭/写错误直停）→ 按目标丢失语义优雅收尾（保留分段）。
        //
        // M18/REQ-24 闩锁顺序：先让生产者有机会在 capture_ended 之前同步写首因；
        // worker 在判定 ended 后**重读**闩锁，消除「检查为空 → 事件入队 → 结束」竞态。
        if let Some(watch) = &ctx.region_watch {
            if !watch.loss.is_set() {
                run_region_latch_hook();
            }
        }
        if capture_ended(&ctx.shared, &ctx.control) {
            let notice = ctx.region_watch.as_ref().and_then(|watch| {
                watch
                    .loss
                    .get()
                    .map(|reason| (watch.display_name.clone(), reason))
            });
            if let Some((endpoint, detail)) = notice {
                ctx.shared.set_stop_notice(StopNotice {
                    code: "E_TARGET_LOST".to_string(),
                    endpoint: Some(endpoint),
                    detail,
                });
            }
            if sr_audio::wasapi::diag_enabled() {
                let window_alive = ctx.target_hwnd.map(|h| unsafe {
                    windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(
                        windows::Win32::Foundation::HWND(h as usize as *mut core::ffi::c_void),
                    ))
                    .as_bool()
                });
                eprintln!(
                    "[diag-route] stop path: capture ended (WGC thread finished) window_alive={window_alive:?}"
                );
            }
            return graceful_stop(ctx);
        }
        // 4) 区域显示器拓扑/几何轮询（路由线程归属；无新帧/静止桌面不判失效）。
        if let Some(watch) = &ctx.region_watch {
            if last_region_poll.elapsed() >= Duration::from_secs(1) {
                if let Err(reason) = watch.poll() {
                    // 枚举失败是查询面噪声，不得误判目标失效；仅真实移除/几何变化停路。
                    if reason.contains("显示器已移除") || reason.contains("显示器几何变化")
                    {
                        ctx.shared.set_stop_notice(StopNotice {
                            code: "E_TARGET_LOST".to_string(),
                            endpoint: Some(watch.display_name.clone()),
                            detail: reason,
                        });
                    } else {
                        eprintln!("[route] 区域显示器轮询失败（继续录制）：{reason}");
                    }
                }
                last_region_poll = Instant::now();
            }
        }
        // 5) 清单新鲜度刷新（节流 ~1s；best-effort——写失败注入期间静默跳过）。
        if last_refresh.elapsed() >= Duration::from_secs(1) {
            refresh_manifest_segments(&ctx.shared);
            last_refresh = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// 停捕获 → 停发 → EOF → 等 sidecar（超时强杀）→ 收尾。EOF 后非零退出仍归 E_SIDECAR_EXIT。
fn graceful_stop(ctx: RunCtx) -> Result<Finalized, RouteError> {
    let diag = sr_audio::wasapi::diag_enabled();
    stop_capture(&ctx.control);
    if diag {
        eprintln!("[diag-route] graceful: capture stopped");
    }
    if let Some(mut sender) = ctx.sender.lock().expect("发送器锁").take() {
        sender.stop();
    }
    if diag {
        eprintln!("[diag-route] graceful: sender stopped");
    }
    ctx.pipe.close(); // EOF：唯一优雅停止路径（S1b 实验 5a）
    if diag {
        eprintln!("[diag-route] graceful: video pipe closed");
    }
    shutdown_audio(&ctx.audio);
    if diag {
        eprintln!("[diag-route] graceful: audio stopped");
    }
    let code = wait_child_or_kill(&ctx.child, SIDECAR_STOP_TIMEOUT);
    match code {
        Some(0) => finalize(&ctx),
        other => Err(sidecar_exit(&ctx, other.unwrap_or(-1))),
    }
}

fn capture_ended(shared: &Shared, control: &Mutex<Option<StreamControl>>) -> bool {
    if shared.capture_ended_for_tests.load(Ordering::Acquire) {
        return true;
    }
    let guard = control.lock().expect("capture control 锁");
    match guard.as_ref() {
        Some(c) => c.is_finished(),
        None => true,
    }
}

fn stop_capture(control: &Mutex<Option<StreamControl>>) {
    if let Some(c) = control.lock().expect("capture control 锁").take() {
        let _ = c.stop(); // POST WM_QUIT + join 捕获线程
    }
}

fn child_exit_code(child: &Mutex<std::process::Child>) -> Option<i32> {
    let mut c = child.lock().expect("sidecar 锁");
    c.try_wait().ok().flatten().and_then(|st| st.code())
}

/// 等待 sidecar 退出（上限超时 → 进程内强杀兜底），返回退出码。
fn wait_child_or_kill(child: &Mutex<std::process::Child>, timeout: Duration) -> Option<i32> {
    let deadline = Instant::now() + timeout;
    loop {
        let mut c = child.lock().expect("sidecar 锁");
        match c.try_wait() {
            Ok(Some(st)) => return st.code(),
            Ok(None) if Instant::now() >= deadline => {
                let _ = c.kill();
                return c.wait().ok().and_then(|st| st.code());
            }
            Ok(None) => drop(c),
            Err(_) => return None,
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn sidecar_exit(ctx: &RunCtx, code: i32) -> RouteError {
    // 写失败（磁盘满/写拒绝等 ENOSPC 类）→ E_DISK_FULL（§3.4/§3.5：安全停止、
    // 保留原段、收尾可重试）。必须扫描**完整 stderr 尾部（16KB）**——只取最后 3 行
    // 摘要会漏掉 "Permission denied" 类信息（m8-red §6.6-R1 rehearsal 实证）。
    let full = ctx
        .shared
        .stderr_tail
        .lock()
        .map(|t| t.clone())
        .unwrap_or_default();
    let lower = full.to_lowercase();
    if lower.contains("permission denied")
        || lower.contains("no space left")
        || lower.contains("disk full")
        || lower.contains("not enough space")
    {
        let tail3: String = full.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
        return RouteError::DiskFull {
            detail: format!("输出写失败（exit {code}）；stderr 尾部: {tail3}"),
        };
    }
    let tail = ctx
        .shared
        .stderr_tail
        .lock()
        .map(|t| t.lines().rev().take(3).collect::<Vec<_>>().join(" | "))
        .unwrap_or_default();
    let send_note = ctx
        .shared
        .send_error
        .lock()
        .ok()
        .and_then(|e| e.clone())
        .map(|e| format!("；发送器: {e}"))
        .unwrap_or_default();
    RouteError::SidecarExit {
        detail: format!("退出码 {code}；stderr 尾部: {tail}{send_note}"),
    }
}

/// M17/REQ-17：remux 独立超时预算（非 concat 60s）。
const REMUX_TIMEOUT: Duration = Duration::from_secs(180);

/// 媒体子进程等待结果（收尾事务用；区分超时/取消以便失败矩阵归因）。
enum MediaWait {
    Exited(Option<i32>),
    Timeout,
    Cancelled,
}

/// 等待媒体子进程（可取消）；超时/取消 → 强杀并回收（Job 绑定已在 spawn）。
fn wait_media_child(
    child: &Mutex<std::process::Child>,
    timeout: Duration,
    cancel: Option<&crate::finalize::FinalizeControl>,
) -> MediaWait {
    let deadline = Instant::now() + timeout;
    loop {
        if cancel.is_some_and(crate::finalize::FinalizeControl::is_cancelled) {
            let mut c = child.lock().expect("媒体子进程锁");
            let _ = c.kill();
            let _ = c.wait();
            return MediaWait::Cancelled;
        }
        {
            let mut c = child.lock().expect("媒体子进程锁");
            match c.try_wait() {
                Ok(Some(st)) => return MediaWait::Exited(st.code()),
                Ok(None) => {}
                Err(_) => return MediaWait::Exited(None),
            }
        }
        if Instant::now() >= deadline {
            let mut c = child.lock().expect("媒体子进程锁");
            let _ = c.kill();
            let _ = c.wait();
            return MediaWait::Timeout;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// 生产媒体执行器：锁定 ffmpeg sidecar + 独立超时 + stderr 排空 + 取消。
pub(crate) struct FfmpegRunner;

impl FfmpegRunner {
    fn run(
        args: Vec<String>,
        what: &str,
        timeout: Duration,
        cancel: Option<&crate::finalize::FinalizeControl>,
        phase: Option<crate::finalize::FinalizeTestPhase>,
    ) -> Result<(), String> {
        let mut child = spawn_sidecar(&args).map_err(|e| format!("spawn {what} 失败: {e}"))?;
        if let Some(phase) = phase {
            // P1-2 L2 屏障：子进程已 spawn（remux 进行中）时确定性停住。
            crate::finalize::hit_test_phase(phase);
        }
        let tail: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        let drainer = spawn_stderr_drainer(&mut child, tail.clone());
        let outcome = wait_media_child(&Mutex::new(child), timeout, cancel);
        let _ = drainer.join();
        match outcome {
            MediaWait::Exited(Some(0)) => Ok(()),
            MediaWait::Exited(code) => {
                let tail = tail.lock().map(|t| t.clone()).unwrap_or_default();
                Err(format!("{what} 退出码 {code:?}；stderr 尾部: {tail}"))
            }
            MediaWait::Timeout => Err(format!("{what} 超时（{}s）", timeout.as_secs())),
            MediaWait::Cancelled => Err(format!("已取消（批量回滚），{what} 中止")),
        }
    }
}

impl crate::finalize::MediaRunner for FfmpegRunner {
    fn concat(
        &self,
        list: &Path,
        out: &Path,
        cancel: Option<&crate::finalize::FinalizeControl>,
    ) -> Result<(), String> {
        let args = vec![
            "-hide_banner".into(),
            "-nostdin".into(),
            "-f".into(),
            "concat".into(),
            "-safe".into(),
            "0".into(),
            "-i".into(),
            list.to_string_lossy().to_string(),
            "-map".into(),
            "0".into(),
            "-c".into(),
            "copy".into(),
            "-f".into(),
            "matroska".into(),
            out.to_string_lossy().to_string(),
        ];
        Self::run(args, "concat", FINALIZE_TIMEOUT, cancel, None)
    }

    fn remux(
        &self,
        input: &Path,
        out: &Path,
        cancel: Option<&crate::finalize::FinalizeControl>,
    ) -> Result<(), String> {
        Self::run(
            crate::finalize::build_remux_args(input, out),
            "remux",
            REMUX_TIMEOUT,
            cancel,
            Some(crate::finalize::FinalizeTestPhase::RemuxStarted),
        )
    }
}

/// 收尾（REQ-21/M17）：统一收尾事务——段清单 → concat → 基础校验 →（MP4）remux →
/// MP4 校验 → safe_commit；全程持有会话级占用。
fn finalize(ctx: &RunCtx) -> Result<Finalized, RouteError> {
    let _reservation = FinalReservation(ctx.final_path.clone());
    let _occupancy =
        crate::recovery::SessionOccupancy::acquire(&ctx.shared.session_dir).map_err(|e| {
            RouteError::FinalizeFailed {
                detail: format!("会话级占用失败：{e:?}"),
            }
        })?;
    let segments = scan_segments(&ctx.shared.output_dir, &ctx.shared.seg_prefix);
    if segments.is_empty() {
        return Err(RouteError::FinalizeFailed {
            detail: "无有效段（全部 0 字节或未写出）".into(),
        });
    }
    if ctx.shared.force_finalize_failure.load(Ordering::Acquire) {
        // L2 失败分支注入（保留原段，不清理会话目录）。
        return Err(RouteError::FinalizeFailed {
            detail: "test-injected finalize failure".into(),
        });
    }

    let audio_tracks = ctx.audio.lock().map(|tracks| tracks.len()).unwrap_or(0);
    let job = crate::finalize::FinalizeJob {
        session_dir: &ctx.shared.session_dir,
        output_dir: ctx
            .shared
            .session_dir
            .parent()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| ctx.shared.session_dir.clone()),
        final_path: &ctx.final_path,
        list_path: &ctx.finalize_list,
        segments: &segments,
        expected: crate::finalize::MediaExpectation {
            video: 1,
            audio: audio_tracks,
        },
        container: ctx.shared.container,
    };
    crate::finalize::run_finalize_transaction(
        &job,
        &FfmpegRunner,
        Some(&ctx.shared.finalize_control),
    )
    .map_err(|detail| RouteError::FinalizeFailed { detail })?;

    let fin = Finalized {
        final_path: ctx.final_path.clone(),
        segment_list: ctx.finalize_list.clone(),
        segments,
        stop_notice: ctx.shared.stop_notice(),
    };
    update_manifest_final(&ctx.shared, &fin.final_path);
    // 清理前释放占用（删除会话目录时占用标记文件不得仍被打开）。
    drop(_occupancy);
    // M14：成品验证成功 → 按保留策略清理原段。清理失败保留会话目录并记录日志，
    // 不改变本路成功归因（成品已提交）。
    cleanup_session_after_finalize(&ctx.shared.session_dir, ctx.shared.keep_segments);
    Ok(fin)
}

/// M14/M14b：成品验证成功后按保留策略清理会话目录（视频成品与纯录音 WAV 共用）。
///
/// `keep_segments=true`：保留原段（调试/排查）；false：递归删除会话目录。删除失败
/// （文件占用/权限等）只记录日志并保留现场——调用方不得因此把路由判失败。
fn cleanup_session_after_finalize(session_dir: &Path, keep_segments: bool) {
    if keep_segments || !session_dir.is_dir() {
        return;
    }
    if let Err(e) = crate::recovery::discard_session(session_dir) {
        eprintln!(
            "[route] 成品验证成功，但清理原段失败（保留会话目录 {}）：{e:?}",
            session_dir.display()
        );
    }
}

/// 录制期清单刷新（best-effort，节流由调用方）：段列表 = 会话目录重扫（closed =
/// 非最高序号；最高序号可能仍在写）；时间范围为名义段区间（REQ-22「段完成但清单未
/// 更新 → 恢复时目录重扫核对」的互补面——陈旧清单始终可由重扫纠正）。
fn refresh_manifest_segments(shared: &Shared) {
    let Ok(mut manifest) = crate::recovery::load_manifest(&shared.session_dir) else {
        return;
    };
    let mut segments: Vec<PathBuf> = std::fs::read_dir(&shared.session_dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.file_name()
                            .map(|n| n.to_string_lossy().starts_with(&shared.seg_prefix))
                            .unwrap_or(false)
                        && p.extension()
                            .map_or(false, |x| x.eq_ignore_ascii_case("mkv"))
                        && std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    segments.sort();
    let Some(route) = manifest
        .routes
        .iter_mut()
        .find(|r| r.route_id == shared.route_id)
    else {
        return;
    };
    let last = segments.len().saturating_sub(1);
    let fresh: Vec<crate::recovery::ManifestSegment> = segments
        .iter()
        .enumerate()
        .map(|(i, p)| crate::recovery::ManifestSegment {
            path: p.clone(),
            t_start: i as f64 * SEGMENT_SECS as f64,
            t_end: (i + 1) as f64 * SEGMENT_SECS as f64,
            closed: i != last,
        })
        .collect();
    let changed = fresh.len() != route.segments.len()
        || fresh
            .iter()
            .zip(route.segments.iter())
            .any(|(a, b)| a.path != b.path || a.closed != b.closed);
    if !changed {
        return;
    }
    route.segments = fresh;
    let _ = crate::recovery::save_manifest_atomic(&shared.session_dir, &manifest);
}

/// 收尾成功后把成品路径写回 manifest（幂等锚点；best-effort）。
fn update_manifest_final(shared: &Shared, final_path: &Path) {
    let Ok(mut manifest) = crate::recovery::load_manifest(&shared.session_dir) else {
        return;
    };
    if let Some(route) = manifest
        .routes
        .iter_mut()
        .find(|r| r.route_id == shared.route_id)
    {
        route.final_path = Some(final_path.to_path_buf());
        route.state = "final".into();
    }
    let _ = crate::recovery::save_manifest_atomic(&shared.session_dir, &manifest);
}

/// 扫描本路由已写出段（`{prefix}%05d.mkv`；0 字节壳不列入；字典序=数值序）。
fn scan_segments(dir: &Path, prefix: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else {
            continue;
        };
        if !name.starts_with(prefix) || !name.ends_with(".mkv") {
            continue;
        }
        if std::fs::metadata(&path)
            .map(|m| m.len() > 0)
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// 当前 UNIX 毫秒（恢复成品命名共用）。
pub(crate) fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 本地时区 UTC 偏移（分钟；实现侧取系统 TZ，REQ-24 命名时间戳的运行侧来源）。
pub(crate) fn local_utc_offset_minutes() -> i32 {
    use windows::Win32::System::SystemServices::TIME_ZONE_ID_DAYLIGHT;
    use windows::Win32::System::Time::{
        GetDynamicTimeZoneInformation, DYNAMIC_TIME_ZONE_INFORMATION,
    };
    unsafe {
        let mut tz = DYNAMIC_TIME_ZONE_INFORMATION::default();
        let id = GetDynamicTimeZoneInformation(&mut tz);
        let mut offset = -tz.Bias as i32;
        if id == TIME_ZONE_ID_DAYLIGHT && !tz.DynamicDaylightTimeDisabled {
            offset += 60;
        }
        offset
    }
}

/// 窗口标题（RouteTarget::Window 的目标名来源；R7：窗口标题即目标名）。
fn window_title(hwnd: u64) -> String {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowTextW;
    if hwnd == 0 {
        return String::new();
    }
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(HWND(hwnd as usize as *mut core::ffi::c_void), &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// M14 单元测试夹具：`<temp>/.screenrec_tmp/sess-1` 内含清单与一个段。
    fn temp_session(tag: &str) -> (PathBuf, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("sr_m14_cleanup_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let session = root.join(".screenrec_tmp").join("sess-1");
        std::fs::create_dir_all(&session).expect("创建会话目录");
        std::fs::write(session.join("manifest.json"), br#"{"version":1}"#).expect("写会话清单");
        std::fs::write(session.join("out0_00000.mkv"), b"segment-bytes").expect("写段文件");
        (root, session)
    }

    #[test]
    fn finalize_cleanup_removes_session_dir_by_default() {
        let (root, session) = temp_session("default");
        cleanup_session_after_finalize(&session, false);
        assert!(
            !session.exists(),
            "M14 默认：成品验证成功后应递归删除会话目录（{}）",
            session.display()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn finalize_cleanup_keeps_session_dir_when_requested() {
        let (root, session) = temp_session("keep");
        cleanup_session_after_finalize(&session, true);
        assert!(
            session.join("out0_00000.mkv").exists(),
            "M14 keep_segments=true：原段应保留（{}）",
            session.display()
        );
        assert!(
            session.join("manifest.json").exists(),
            "M14 keep_segments=true：会话清单应保留（{}）",
            session.display()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// M14b 单元测试夹具：`<temp>/.screenrec_tmp/sess-audio` 内含清单与一个 WAV
    /// （>44 字节 = `commit_audio_only` 的 WAV 校验下限）。
    fn temp_audio_session(tag: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "sr_m14b_audio_cleanup_{tag}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let session = root.join(".screenrec_tmp").join("sess-audio");
        std::fs::create_dir_all(&session).expect("创建会话目录");
        std::fs::write(session.join("manifest.json"), br#"{"version":1}"#).expect("写会话清单");
        std::fs::write(session.join("20261018_120000_audio0.wav"), vec![0u8; 100]).expect("写 WAV");
        (root, session)
    }

    #[test]
    fn audio_only_commit_cleanup_removes_session_dir_by_default() {
        let (root, session) = temp_audio_session("default");
        let out = root.join("out");
        std::fs::create_dir_all(&out).expect("创建输出目录");
        let wav = session.join("20261018_120000_audio0.wav");
        let committed = commit_audio_only(&out, &session, "20261018_120000", &[wav], false)
            .expect("M14b：纯录音 WAV 安全提交应成功");
        assert_eq!(committed.len(), 1);
        assert!(
            committed[0].exists(),
            "M14b：提交后的 WAV 应位于输出目录（{}）",
            committed[0].display()
        );
        assert!(
            !session.exists(),
            "M14b 默认：纯录音成功后应递归删除会话目录（{}）",
            session.display()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn audio_only_commit_cleanup_keeps_session_dir_when_requested() {
        let (root, session) = temp_audio_session("keep");
        let out = root.join("out");
        std::fs::create_dir_all(&out).expect("创建输出目录");
        let wav = session.join("20261018_120000_audio0.wav");
        let committed = commit_audio_only(&out, &session, "20261018_120000", &[wav], true)
            .expect("M14b：纯录音 WAV 安全提交应成功");
        assert!(
            committed[0].exists(),
            "M14b：提交后的 WAV 应位于输出目录（{}）",
            committed[0].display()
        );
        assert!(
            session.join("manifest.json").exists(),
            "M14b keep_segments=true：会话目录/清单应保留（{}）",
            session.display()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// REQ-8：捕获 96k / 输出 48k 稳态输入 → 漂移比较统一到秒域，corrections ≈ 0
    /// （不因「帧数 ÷ 48k」与 96k 帧数混用而产生虚假校正）。
    #[test]
    fn unit_drift_mixed_rates_no_false_correction() {
        let capture_rate = 96_000u32;
        let mut drift = DriftController::new(DriftCfg {
            enter_ms: DRIFT_ENTER_MS,
            exit_ms: DRIFT_EXIT_MS,
            rate_limit_ppm: DRIFT_LIMIT_PPM,
        });
        let mut corrections = 0;
        let mut max_abs = 0.0f64;
        for step in 1..=100u64 {
            // 视频 30fps 参照：每 0.2s 采样一次；捕获帧数按 96k 域累计。
            let reference_secs = step as f64 * 0.2;
            let capture_frames = (reference_secs * capture_rate as f64).round() as u64;
            let offset_ms = content_offset_ms(reference_secs, capture_frames, capture_rate);
            max_abs = max_abs.max(offset_ms.abs());
            if !matches!(drift.on_measurement(offset_ms), DriftAction::Hold) {
                corrections += 1;
            }
        }
        assert!(
            max_abs < 0.01,
            "96k 捕获/48k 输出的稳态偏差应为 0（得到 {max_abs}ms）"
        );
        assert_eq!(corrections, 0, "REQ-8：不得产生虚假漂移校正");
    }
    // ── M16/REQ-13：System device_id 一次解析并钉住 / 去重 ──

    #[test]
    fn unit_system_target_resolution_pinned_uses_resolved_id_once() {
        let calls = std::cell::Cell::new(0u32);
        let resolved = resolve_audio_sources(
            &[
                AudioSourceSpec::RenderEndpoint {
                    device_id: "ep-display".into(),
                },
                AudioSourceSpec::System,
            ],
            || {
                calls.set(calls.get() + 1);
                Some("ep-display".into())
            },
        )
        .expect("解析应成功");
        assert_eq!(calls.get(), 1, "REQ-13：System 默认端点只解析一次");
        assert_eq!(
            resolved,
            vec![ResolvedAudioSource::RenderEndpoint {
                device_id: "ep-display".into()
            }],
            "endpoint==system 应去重为单轨并保留端点的失效监控语义（端点在前）"
        );
    }

    #[test]
    fn unit_system_target_resolution_keeps_two_tracks_when_ids_differ() {
        let resolved = resolve_audio_sources(
            &[
                AudioSourceSpec::RenderEndpoint {
                    device_id: "ep-display".into(),
                },
                AudioSourceSpec::System,
                AudioSourceSpec::Mic {
                    device: "mic-1".into(),
                },
            ],
            || Some("render-default".into()),
        )
        .expect("解析应成功");
        assert_eq!(
            resolved,
            vec![
                ResolvedAudioSource::RenderEndpoint {
                    device_id: "ep-display".into()
                },
                ResolvedAudioSource::System {
                    device_id: "render-default".into()
                },
                ResolvedAudioSource::Mic {
                    device: "mic-1".into()
                },
            ],
            "REQ-13：端点≠系统 → 两条；轨序保持 UI 标签序 [endpoint, system, mic]"
        );
    }

    #[test]
    fn unit_system_target_resolution_pinned_survives_default_change() {
        // 第一次解析（启动快照）得到 A；此后默认端点改为 B，不得回退/重新查询。
        let mut default = "render-A".to_string();
        let first = resolve_audio_sources(&[AudioSourceSpec::System], || Some(default.clone()))
            .expect("首次解析");
        default = "render-B".to_string();
        assert_eq!(default, "render-B", "模拟解析后默认端点已变化");
        assert_eq!(
            first,
            vec![ResolvedAudioSource::System {
                device_id: "render-A".into()
            }],
            "REQ-13：解析后默认设备变化 → 仍使用钉住 ID（激活不二次查询默认端点）"
        );
        let target = capture_target_for(&first[0]).expect("System 应映射为设备环回目标");
        assert_eq!(
            target,
            CaptureTarget::DeviceLoopback {
                device_id: Some("render-A".into())
            },
            "REQ-13：钉住 ID 必须进入捕获目标（激活走 get_device(钉住ID)）"
        );
    }

    #[test]
    fn unit_system_target_resolution_default_failure_is_explicit() {
        let err = resolve_audio_sources(&[AudioSourceSpec::System], || None)
            .expect_err("默认渲染端点解析失败必须显式报错");
        assert!(
            matches!(err, RouteError::LoopbackUnavailable { .. }),
            "得到 {err:?}"
        );
    }

    #[test]
    fn unit_capture_target_maps_render_endpoint_and_mic() {
        assert_eq!(
            capture_target_for(&ResolvedAudioSource::RenderEndpoint {
                device_id: "ep-1".into()
            })
            .expect("端点应映射"),
            CaptureTarget::RenderEndpoint {
                device_id: "ep-1".into()
            }
        );
        assert_eq!(
            capture_target_for(&ResolvedAudioSource::Mic {
                device: "mic-1".into()
            })
            .expect("麦克风应映射"),
            CaptureTarget::Microphone {
                device: "mic-1".into()
            }
        );
    }
}
