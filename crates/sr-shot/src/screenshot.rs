//! 截图公共 API（REQ-5..7 的契约面；接口契约 = docs/milestones/m2-red-report.md
//! §3/§10，测试即契约；specs/001-screen-recorder.md §2.2/§3.3/§3.4）。
//!
//! 行为要点：
//! - 单位：`width_px/height_px` 与 `ScreenRect` 全部为物理像素；`x/y` 为桌面物理坐标
//!   （负值合法，REQ-6）。物理语义由 worker 线程的 per-monitor-v2 上下文 + DEVMODE
//!   真值校验保证，与调用方进程的 DPI 感知无关。
//! - 索引空间：`ShotTarget::Monitor{index}` 与 `sr_enum::monitors::MonitorEntry.index`
//!   同一索引空间（EnumDisplayMonitors 枚举序，0..n-1 稠密；§3.4 RouteConfig 兼容）。
//! - 窗口：WGC 单帧捕获窗口自身表面（被遮挡仍正确，REQ-7）；WGC 不可用回退
//!   PrintWindow(PW_RENDERFULLCONTENT)；最小化（IsIconic）→ `ShotError::TargetMinimized`。
//! - 区域：按各显示器物理帧在桌面物理坐标系拼合后裁剪（跨多显示器/混合 DPI/负坐标）。
//! - `source` 回执 = 请求目标（原样回传，防实现张冠李戴）。

use std::fmt;
use std::path::PathBuf;

use crate::capture::{self, BgraFrame};
use crate::compose;
use crate::png;

/// 物理像素矩形（桌面物理坐标系；REQ-6：负坐标合法，原点语义 = 桌面原点）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    /// 物理像素（DPI 无关）。
    pub width_px: u32,
    /// 物理像素（DPI 无关）。
    pub height_px: u32,
}

/// 截图目标（REQ-5）。
///
/// `Monitor.index` 与 `sr_enum::monitors::MonitorEntry.index` 为同一索引空间
/// （`RouteConfig::Monitor(idx)` 兼容，§3.4）；`Window.hwnd` 为 HWND 数值化
/// （与 `sr_enum::windows::WindowEntry.hwnd` 同型）；`Region.rect` 为物理桌面坐标，
/// 跨显示器由实现拼合后裁剪（REQ-6）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShotTarget {
    Monitor { index: u32 },
    Window { hwnd: u64 },
    Region { rect: ScreenRect },
}

impl ShotTarget {
    /// 输出文件名中的目标种类标记（REQ-24 命名全量规则属 M8；此处仅用于
    /// 同目录多次输出的可读区分与唯一性）。
    fn kind(&self) -> &'static str {
        match self {
            Self::Monitor { .. } => "monitor",
            Self::Window { .. } => "window",
            Self::Region { .. } => "region",
        }
    }
}

/// 截图请求（按值消费；`output_dir` 由调用方先创建，假定已存在）。
#[derive(Clone, Debug)]
pub struct ScreenshotRequest {
    pub target: ShotTarget,
    pub output_dir: PathBuf,
}

/// 截图结果：元数据即断言面（输出目录内的 PNG 文件；物理像素尺寸；实际捕获来源回执）。
#[derive(Clone, Debug)]
pub struct SavedScreenshot {
    /// 输出目录内的 PNG 文件（REQ-5；REQ-24 命名规则属 M8 另测）。
    pub path: PathBuf,
    /// 物理像素（REQ-6）；与 PNG 实际编码尺寸（IHDR）一致。
    pub width_px: u32,
    /// 物理像素（REQ-6）；与 PNG 实际编码尺寸（IHDR）一致。
    pub height_px: u32,
    /// 实际捕获来源回执（= 请求目标）。
    pub source: ShotTarget,
}

/// 截图错误。契约钉死的仅 `TargetMinimized`（REQ-7；红报告 §10 的最小收紧），
/// 其余变体为实现自定，语义对齐规格 §3.4（`TargetInvalid` ≈ E_TARGET_LOST）。
#[derive(Debug)]
pub enum ShotError {
    /// REQ-7：最小化窗口（IsIconic=true）无有效帧源，返回明确错误。
    TargetMinimized,
    /// 目标不存在/已关闭（E_TARGET_LOST 语义：无效 HWND 等）。
    TargetInvalid { detail: String },
    /// `Monitor.index` 越界（合法域：0..monitor_count-1）。
    MonitorIndexOutOfRange { index: u32, monitor_count: u32 },
    /// 请求区域为空（宽/高为 0）或与任何显示器无交集。
    EmptyRegion { rect: ScreenRect },
    /// 捕获基础设施不可用（D3D/WGC 初始化或取帧失败、物理契约校验不过、
    /// 窗口两条路径均失败等）。
    CaptureUnavailable { detail: String },
    /// 在超时窗口内未取得捕获帧。
    CaptureTimeout { detail: String },
    /// 输出写失败（目录不可写/占用/命名冲突等）。
    OutputWriteFailed { path: PathBuf, detail: String },
    /// 实现内部约束失败（worker 异常等；正常路径不应出现）。
    Internal { detail: String },
}

impl fmt::Display for ShotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TargetMinimized => write!(f, "E_TARGET_MINIMIZED: 目标窗口已最小化（REQ-7）"),
            Self::TargetInvalid { detail } => write!(f, "E_TARGET_LOST: 目标无效: {detail}"),
            Self::MonitorIndexOutOfRange {
                index,
                monitor_count,
            } => {
                write!(
                    f,
                    "E_TARGET_LOST: 显示器索引 {index} 越界（共 {monitor_count} 台）"
                )
            }
            Self::EmptyRegion { rect } => {
                write!(
                    f,
                    "E_INVALID_REGION: 区域 ({},{}) {}x{} 为空或与任何显示器无交集",
                    rect.x, rect.y, rect.width_px, rect.height_px
                )
            }
            Self::CaptureUnavailable { detail } => write!(f, "捕获不可用: {detail}"),
            Self::CaptureTimeout { detail } => write!(f, "捕获超时: {detail}"),
            Self::OutputWriteFailed { path, detail } => {
                write!(f, "输出写失败（{}）: {detail}", path.display())
            }
            Self::Internal { detail } => write!(f, "内部错误: {detail}"),
        }
    }
}

impl std::error::Error for ShotError {}

/// 拍摄一张截图（REQ-5）：按目标捕获物理像素帧 → 编码 PNG 写入输出目录 →
/// 返回元数据（路径/物理宽高/来源回执）。
///
/// 捕获在独立 worker 线程执行（WinRT apartment + per-monitor-v2 DPI 上下文），
/// 不改变调用线程的任何进程/线程级状态；按值消费请求。
pub fn take_screenshot(req: ScreenshotRequest) -> Result<SavedScreenshot, ShotError> {
    let target = req.target;
    let target_for_job = target.clone();
    let frame = run_on_capture_worker(move || capture_frame(&target_for_job))?;
    let rgba = frame.to_rgba();
    let path = png::write_png_unique(
        &req.output_dir,
        target.kind(),
        frame.width,
        frame.height,
        &rgba,
    )
    .map_err(|e| ShotError::OutputWriteFailed {
        path: e.path,
        detail: e.detail,
    })?;
    Ok(SavedScreenshot {
        path,
        width_px: frame.width,
        height_px: frame.height,
        source: target,
    })
}

/// 按目标类型分派捕获（worker 线程内、per-monitor-v2 上下文中执行）。
fn capture_frame(target: &ShotTarget) -> Result<BgraFrame, ShotError> {
    match target {
        ShotTarget::Monitor { index } => {
            let mons = capture::list_physical_monitors();
            let monitor_count = mons.len() as u32;
            let mon =
                mons.get(*index as usize)
                    .ok_or_else(|| ShotError::MonitorIndexOutOfRange {
                        index: *index,
                        monitor_count,
                    })?;
            capture::capture_monitor(mon)
        }
        ShotTarget::Window { hwnd } => capture::capture_window(*hwnd),
        ShotTarget::Region { rect } => compose::compose_region(rect),
    }
}

/// 在专用 worker 线程上执行捕获（COM/WGC 与调用线程完全隔离；panic 映射为
/// `ShotError::Internal`，不得向调用方 unwinding）。
fn run_on_capture_worker<F, T>(job: F) -> Result<T, ShotError>
where
    F: FnOnce() -> Result<T, ShotError> + Send + 'static,
    T: Send + 'static,
{
    let handle = std::thread::Builder::new()
        .name("sr-shot-capture".into())
        .spawn(move || {
            let _thread_env = capture::CaptureThread::init();
            job()
        })
        .map_err(|e| ShotError::Internal {
            detail: format!("启动捕获 worker 线程失败: {e}"),
        })?;
    let joined = handle.join();
    match joined {
        Ok(result) => result,
        Err(_) => Err(ShotError::Internal {
            detail: "捕获 worker 线程 panic".into(),
        }),
    }
}
