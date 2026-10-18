//! 单路 WGC 捕获（REQ-9/11：windows-capture 2.0.1，仅取帧）。
//!
//! 每路独立线程 + 独立 frame pool（`start_free_threaded`）；帧到达回调内完成
//! 「源时间戳 → 帧槽决策 → 固定画布缩放/NV12 → 更新最近帧槽」（REQ-10 槽决策）。
//! 管道写不在回调内：由产品侧的 CFR 槽发送器按 30fps 节拍取「最近帧」写管道
//! （§3.3「连续 NV12 帧」，源欠产/瞬时停滞时以最近帧重复保持连续；写阻塞背压
//! 不反压 DWM 回调）。光标开关经 `CursorCaptureSettings` 直通 WGC（REQ-11）。

use std::sync::Arc;

use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

use crate::canvas::CanvasConverter;
use crate::pacer::{FrameAction, FramePacer};

/// WGC SystemRelativeTime 为 TimeSpan（100ns 单位，QPC 域）。
pub const WGC_TICKS_PER_SEC: i64 = 10_000_000;

/// 会话启动错误（产品层映射：ConsentDenied→E_CONSENT_DENIED；TargetInvalid→E_TARGET_LOST）。
#[derive(Debug)]
pub enum StreamStartError {
    /// 目标不存在/无效（HWND 失效、显示器索引越界）。
    TargetInvalid { detail: String },
    /// WGC 授权被拒（E_ACCESSDENIED）。
    ConsentDenied,
    /// 其它启动失败。
    Failed { detail: String },
}

/// 最近画布 NV12 帧槽（回调线程写、CFR 发送线程读；单槽最新值语义）。
pub struct LatestFrame {
    inner: std::sync::Mutex<Option<Arc<Vec<u8>>>>,
}

impl LatestFrame {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: std::sync::Mutex::new(None),
        })
    }

    /// 回调线程：写入最新画布帧（覆盖旧值）。
    pub fn set(&self, nv12: &[u8]) {
        *self.inner.lock().expect("最近帧锁") = Some(Arc::new(nv12.to_vec()));
    }

    /// 发送线程：取当前最近帧（无帧产出时 None）。
    pub fn get(&self) -> Option<Arc<Vec<u8>>> {
        self.inner.lock().expect("最近帧锁").clone()
    }
}

/// 帧转发器配置（经 `Flags` 传入捕获线程；画布尺寸随行）。
pub struct ForwarderFlags {
    pub canvas_w: u32,
    pub canvas_h: u32,
    pub fps: u32,
    /// 最近帧槽（回调写入，发送线程消费）。
    pub latest: Arc<LatestFrame>,
}

/// 帧转发器内部错误（本类型仅为 trait 约束占位）。
#[derive(Debug)]
pub struct ForwarderError;

impl std::fmt::Display for ForwarderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "frame forwarder error")
    }
}

/// 会话控制句柄（`stop()` 消费并 join 捕获线程）。
pub type StreamControl = CaptureControl<FrameForwarder, ForwarderError>;

/// 帧 → 槽 → 画布 → 最近帧槽的回调体（捕获线程私有，无锁）。
pub struct FrameForwarder {
    pacer: FramePacer,
    canvas: CanvasConverter,
    latest: Arc<LatestFrame>,
}

impl GraphicsCaptureApiHandler for FrameForwarder {
    type Flags = ForwarderFlags;
    type Error = ForwarderError;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let ForwarderFlags {
            canvas_w,
            canvas_h,
            fps,
            latest,
        } = ctx.flags;
        Ok(Self {
            pacer: FramePacer::new(fps, WGC_TICKS_PER_SEC),
            canvas: CanvasConverter::new(canvas_w, canvas_h),
            latest,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        // 最小化/零尺寸占位帧：不出帧（补帧属 REQ-13 后续契约）。
        if frame.width() == 0 || frame.height() == 0 {
            return Ok(());
        }
        // 源时间戳（100ns ticks）→ 槽决策；重复/迟到帧不更新最近帧（REQ-10）。
        let ts = frame.timestamp().map(|t| t.Duration as i64).unwrap_or(0);
        if !matches!(self.pacer.on_frame(ts), FrameAction::Write { .. }) {
            return Ok(());
        }
        // 帧缓冲（BGRA；去行 padding）→ 固定画布 NV12 → 更新最近帧槽。
        let buffer = match frame.buffer() {
            Ok(b) => b,
            Err(_) => return Ok(()), // 单帧读取失败不终止会话（下一帧恢复）
        };
        let (w, h) = (buffer.width(), buffer.height());
        let mut padding_scratch = Vec::new();
        // 无 padding 时库内零拷贝直返；有 padding 时拷入 scratch（紧凑行）。
        let compact: &[u8] = buffer.as_nopadding_buffer(&mut padding_scratch);
        let nv12 = self.canvas.convert(compact, w, h);
        self.latest.set(nv12);
        Ok(())
    }

    /// WGC 目标窗口关闭/捕获项失效（会话终止的正常来源）。
    /// `SR_DIAG_AV=1` 时打点：用于区分「目标关闭」与「采集线程静默结束」。
    fn on_closed(&mut self) -> Result<(), Self::Error> {
        if diag_enabled() {
            eprintln!("[diag-video] WGC capture item closed (window destroyed / item invalidated)");
        }
        Ok(())
    }
}

/// 诊断开关（`SR_DIAG_AV=1`；与 sr-audio 同口径，wgc 侧独立判定避免跨层依赖）。
fn diag_enabled() -> bool {
    matches!(std::env::var("SR_DIAG_AV").as_deref(), Ok("1") | Ok("true"))
}

/// 启动单路捕获（窗口；hwnd 为数值化 HWND，无效 → TargetInvalid）。
pub fn start_window_stream(
    hwnd: u64,
    canvas_w: u32,
    canvas_h: u32,
    fps: u32,
    cursor: bool,
    latest: Arc<LatestFrame>,
) -> Result<StreamControl, StreamStartError> {
    if hwnd == 0 {
        return Err(StreamStartError::TargetInvalid {
            detail: "hwnd=0".into(),
        });
    }
    let window = Window::from_raw_hwnd(hwnd as usize as *mut core::ffi::c_void);
    if !window.is_valid() {
        return Err(StreamStartError::TargetInvalid {
            detail: format!("hwnd={hwnd:#x} 无效（IsWindow=false）"),
        });
    }
    start_stream(Settings::new(
        window,
        cursor_setting(cursor),
        DrawBorderSettings::Default,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        ForwarderFlags {
            canvas_w,
            canvas_h,
            fps,
            latest,
        },
    ))
}

/// 启动单路捕获（显示器；index 为 sr_enum 0 起稠密索引 → windows-capture 1 起序）。
pub fn start_monitor_stream(
    index: u32,
    canvas_w: u32,
    canvas_h: u32,
    fps: u32,
    cursor: bool,
    latest: Arc<LatestFrame>,
) -> Result<StreamControl, StreamStartError> {
    let monitor =
        Monitor::from_index(index as usize + 1).map_err(|e| StreamStartError::TargetInvalid {
            detail: format!("显示器 index={index} 不存在: {e}"),
        })?;
    start_stream(Settings::new(
        monitor,
        cursor_setting(cursor),
        DrawBorderSettings::Default,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        ForwarderFlags {
            canvas_w,
            canvas_h,
            fps,
            latest,
        },
    ))
}

fn cursor_setting(cursor: bool) -> CursorCaptureSettings {
    if cursor {
        CursorCaptureSettings::WithCursor
    } else {
        CursorCaptureSettings::WithoutCursor
    }
}

/// 进程级 MTA 保活（一次性 `CoIncrementMTAUsage`，**永不递减**）。
///
/// 背景（M4 遗留 AV 取证，docs/test-fix/2026-10-16-m4-audio-l2-av.md）：
/// windows-capture 2.0.1 每次会话在线程内 `CoIncrementMTAUsage` + `RoInitialize(MTA)`，
/// 会话收尾 `RoUninitialize` + `CoDecrementMTAUsage`。当 MTA 引用计数归零时 COM/WinRT
/// 运行时会拆解 MTA；而 windows-rs（windows-core）的进程级 `FactoryCache` 静态仍持有
/// 先前的会话激活的工厂指针（leak 引用假定长期有效），下一会话在新线程走
/// `GraphicsCaptureSession::IsSupported` → `FactoryCache::call` 时踩到失效指针 →
/// STATUS_ACCESS_VIOLATION（现场：故障读 `[rax+0x30]`、rax 指向已释放堆区；
/// 上游同类：windows-capture issue #124「第二次 start_free_threaded AV」；建议方案：
/// windows-rs issue #1169「A long-lived MTA is the only recommended approach」）。
/// 本函数在进程生命周期内保持 MTA 存活（cookie 有意泄漏；单个空 MTA 成本可忽略）。
fn keep_alive_process_mta() {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        // combase.dll（Windows 8+）；返回值仅 S_OK/S_FALSE，失败也不致命
        // （后续会话仍按 windows-capture 自身路径工作/报错）。
        let mut cookie: *mut core::ffi::c_void = core::ptr::null_mut();
        unsafe {
            let _ = co_increment_mta_usage(&mut cookie);
        }
    });
}

#[link(name = "combase", kind = "raw-dylib")]
extern "system" {
    #[link_name = "CoIncrementMTAUsage"]
    fn co_increment_mta_usage(cookie: *mut *mut core::ffi::c_void) -> i32;
}

fn start_stream(
    settings: Settings<
        ForwarderFlags,
        impl TryInto<windows_capture::settings::GraphicsCaptureItemType> + Send + 'static,
    >,
) -> Result<StreamControl, StreamStartError> {
    // MTA 进程级保活须先于任何会话启动（含首个）——首个会话激活的 WinRT 工厂
    // 被 windows-rs FactoryCache 缓存到进程级静态，MTA 拆解后该指针即失效。
    keep_alive_process_mta();
    FrameForwarder::start_free_threaded(settings).map_err(map_start_error)
}

fn map_start_error(
    e: windows_capture::capture::GraphicsCaptureApiError<ForwarderError>,
) -> StreamStartError {
    use windows_capture::capture::GraphicsCaptureApiError as E;
    match e {
        E::ItemConvertFailed => StreamStartError::TargetInvalid {
            detail: "捕获项转换失败（CreateForWindow/CreateForMonitor）".into(),
        },
        other => {
            let text = other.to_string();
            // WGC 授权拒绝（E_ACCESSDENIED 0x80070005）→ ConsentDenied 语义。
            if text.contains("0x80070005") || text.contains("ACCESSDENIED") {
                StreamStartError::ConsentDenied
            } else {
                StreamStartError::Failed { detail: text }
            }
        }
    }
}
