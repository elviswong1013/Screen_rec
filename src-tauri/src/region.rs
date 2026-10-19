//! P1-1 区域框选后端（契约固定）：虚拟桌面物理边界/缩放 + 无边框置顶 overlay 生命周期。
//!
//! 坐标契约（前端 ↔ 后端唯一口径）：
//! - `get_overlay_geometry()` 返回虚拟桌面的**物理**边界（`origin_x/origin_y/width/height`）
//!   与 overlay 的 DPI 缩放 `scale`；
//! - 前端物理坐标 = `origin + client * scale`（client 为 region.html 的 CSS 像素）；
//! - `take_screenshot(Region{ x, y, w, h })` 接收**物理**像素（沿用 sr-shot 桌面物理
//!   坐标语义；负坐标合法），因此前端必须按上式换算后再调用。
//!
//! overlay 窗口：label 固定 `region-overlay`、加载 `region.html`、无边框/置顶/覆盖
//! 整个虚拟桌面（创建后按物理像素再次 set_position/set_size，避免混合 DPI 下的逻辑
//! 坐标取整偏差）。重复调用 `open_region_overlay` 复用已有窗口（show + focus）。
//! `open_region_overlay` 必须保持 async：sync 命令在 WebView2 IPC 回调里执行，同步
//! 创建第二个 WebView2 控制器会重入死锁（窗口出现但不绘制、invoke 不返回）。
//!
//! P1-A 区域截图污染修复：截图顺序由后端主导（[`take_region_screenshot`]）——
//! 隐藏覆盖层 → `DwmFlush` + ≥150ms 等待合成 → sr-shot 物理像素截图 → 成功关闭覆盖层。
//! 前端不得先截图后关窗（否则会捕获覆盖层遮罩/边框像素）。失败时覆盖层恢复可见并在
//! 页面内提示错误（P2：不得静默关闭、不得残留隐藏窗口）。

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use sr_shot::screenshot::{self, ScreenRect, ScreenshotRequest, ShotTarget};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder,
};
use windows::Win32::Graphics::Dwm::DwmFlush;

use crate::commands::{blocking, invalid, resolve_output_dir, shot_error_string, RectIn};

/// overlay 窗口 label（前端与后端共用；钉死）。
pub const REGION_OVERLAY_LABEL: &str = "region-overlay";
/// overlay 页面（Tauri app 资源相对路径；由前端任务提供）。
pub const REGION_OVERLAY_PAGE: &str = "region.html";
/// 区域录制确认交付事件（主窗监听；载荷 = [`RegionTargetAddedOut`]）。
pub const REGION_TARGET_ADDED_EVENT: &str = "region_target_added";
/// 区域录制取消/关闭事件（主窗据此清理当前 request_id；载荷 = [`RegionRecordCancelledOut`]）。
pub const REGION_RECORD_CANCELLED_EVENT: &str = "region_record_cancelled";
/// 最小有效区域尺寸（吸附后；NV12/编码约束）。
pub const REGION_MIN_SIZE: u32 = 64;

/// `get_region_context()` 回执（overlay 初始化上下文；M18/REQ-21）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RegionContextOut {
    /// `screenshot`（默认，兼容旧调用）| `record`。
    pub mode: String,
    /// 主窗生成的请求代次 id（截图模式可为空）。
    pub request_id: String,
}

/// 区域录制显示器载荷（事件/回执共用；`device_name` = GDI szDevice）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RegionDisplayOut {
    pub device_name: String,
    pub device_path: Option<String>,
}

/// `confirm_region_record` 回执 / `region_target_added` 事件载荷。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RegionTargetAddedOut {
    pub request_id: String,
    /// 已向下取偶的最终区域（物理桌面坐标）。
    pub rect: RectIn,
    pub display: RegionDisplayOut,
}

/// `region_record_cancelled` 事件载荷（主窗只清理匹配的当前 request_id）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RegionRecordCancelledOut {
    pub request_id: String,
}

#[derive(Default)]
struct RegionOverlayInner {
    mode: Option<String>,
    request_id: String,
    /// 每次 open 递增；旧代次确认一律丢弃（复用异模式以最新代次为准）。
    generation: u64,
    /// record 确认一次性交付（true = 尚可确认）。
    pending_record: bool,
    /// 显式 Closing：close 已排队但窗口尚未销毁时禁止 open 复用同 label。
    closing: bool,
}

/// overlay 上下文/代次状态（Tauri managed state）。
#[derive(Default)]
pub struct RegionOverlayState {
    inner: Mutex<RegionOverlayInner>,
    /// 生命周期串行锁（**与 inner 分离**）：open/cancel/close/confirm 的
    /// 代次切换与窗口副作用（hide/show/emit/close）在同一临界区执行。
    /// 禁止跨 `.await` 持有（hide/commit 均为同步段）。
    lifecycle: Mutex<()>,
}

impl RegionOverlayState {
    /// 串行执行一段 overlay 生命周期操作（不得跨 await 持有）。
    fn with_lifecycle<T>(&self, f: impl FnOnce() -> T) -> T {
        let _guard = self.lifecycle.lock().expect("region overlay 生命周期锁");
        f()
    }

    /// 仅当代次/模式/request_id 仍为当前时执行 `f`；check 与副作用在同一
    /// lifecycle 临界区内（cancel/open 要么在之前完成使本段 stale，要么等本段结束）。
    fn run_if_current<T>(
        &self,
        generation: u64,
        request_id: &str,
        f: impl FnOnce() -> T,
    ) -> Result<T, String> {
        self.with_lifecycle(|| {
            if !self.is_current_confirm(generation, request_id) {
                return Err(invalid(
                    "E_INVALID_REQUEST",
                    format!("区域录制确认已失效（request_id={request_id}）"),
                ));
            }
            Ok(f())
        })
    }

    fn begin(&self, mode: &str, request_id: &str) -> u64 {
        let mut inner = self.inner.lock().expect("region overlay 状态锁");
        inner.generation = inner.generation.wrapping_add(1);
        inner.mode = Some(mode.to_string());
        inner.request_id = request_id.to_string();
        inner.pending_record = mode == "record";
        inner.generation
    }

    fn context(&self) -> RegionContextOut {
        let inner = self.inner.lock().expect("region overlay 状态锁");
        RegionContextOut {
            mode: inner
                .mode
                .clone()
                .unwrap_or_else(|| "screenshot".to_string()),
            request_id: inner.request_id.clone(),
        }
    }

    /// Closing 门禁：close 已排队但窗口尚未销毁时不得复用/创建同 label 窗口。
    fn ensure_open_allowed(&self) -> Result<(), String> {
        let inner = self.inner.lock().expect("region overlay 状态锁");
        if inner.closing {
            return Err(invalid(
                "E_INTERNAL",
                "区域覆盖层正在关闭，请稍后重试（旧窗口尚未销毁）",
            ));
        }
        Ok(())
    }

    fn is_closing(&self) -> bool {
        self.inner.lock().expect("region overlay 状态锁").closing
    }

    /// 代次复核 + 一次性消费（失败 = 迟到/重复/跨模式确认）。
    fn begin_confirm(&self, request_id: &str) -> Result<u64, String> {
        let mut inner = self.inner.lock().expect("region overlay 状态锁");
        if inner.mode.as_deref() != Some("record") {
            if inner.mode.is_none() && !inner.pending_record {
                return Err(invalid(
                    "E_INVALID_REQUEST",
                    format!("区域录制确认已失效（request_id={request_id}）"),
                ));
            }
            return Err(invalid(
                "E_INVALID_REGION",
                "当前覆盖层不是区域录制模式（record 路径不得复用截图确认）",
            ));
        }
        if !inner.pending_record || inner.request_id != request_id {
            return Err(invalid(
                "E_INVALID_REQUEST",
                format!("区域录制确认已失效（request_id={request_id}）"),
            ));
        }
        inner.pending_record = false;
        Ok(inner.generation)
    }

    fn restore_confirm(&self, generation: u64, request_id: &str) {
        let mut inner = self.inner.lock().expect("region overlay 状态锁");
        if inner.generation == generation
            && inner.mode.as_deref() == Some("record")
            && inner.request_id == request_id
        {
            inner.pending_record = true;
        }
    }

    /// 交付边界只读复核：代次 + 模式 + request_id 仍是当前（cancel/open B 后为 false）。
    fn is_current_confirm(&self, generation: u64, request_id: &str) -> bool {
        let inner = self.inner.lock().expect("region overlay 状态锁");
        inner.generation == generation
            && inner.mode.as_deref() == Some("record")
            && inner.request_id == request_id
    }

    /// cancel 使迟到确认失效（仅当前 request_id；旧代次 cancel 不关闭新会话）。
    fn invalidate_record(&self, request_id: &str) -> bool {
        let mut inner = self.inner.lock().expect("region overlay 状态锁");
        if inner.mode.as_deref() == Some("record") && inner.request_id == request_id {
            inner.pending_record = false;
            inner.mode = None;
            return true;
        }
        false
    }

    fn clear(&self) {
        let mut inner = self.inner.lock().expect("region overlay 状态锁");
        inner.mode = None;
        inner.pending_record = false;
        inner.closing = false;
    }

    /// 成功提交后只清理当前代次；避免随后 cancel/close 误关新窗口。
    fn clear_if_current(&self, generation: u64, request_id: &str) {
        let mut inner = self.inner.lock().expect("region overlay 状态锁");
        if inner.generation == generation && inner.request_id == request_id {
            inner.mode = None;
            inner.pending_record = false;
            inner.closing = false;
        }
    }

    /// 成功提交时标记 Closing（generation/request 限定）。
    fn mark_closing_if_current(&self, generation: u64, request_id: &str) -> bool {
        let mut inner = self.inner.lock().expect("region overlay 状态锁");
        if inner.generation == generation
            && inner.mode.as_deref() == Some("record")
            && inner.request_id == request_id
        {
            inner.closing = true;
            return true;
        }
        false
    }

    /// cancel 已失效当前 request 后标记 Closing（旧 request 不影响新会话）。
    fn mark_closing_if_request(&self, request_id: &str) -> bool {
        let mut inner = self.inner.lock().expect("region overlay 状态锁");
        if !request_id.is_empty() && inner.request_id == request_id {
            inner.closing = true;
            return true;
        }
        false
    }

    /// close_region_overlay 关闭当前 overlay（含 screenshot 模式）时标记 Closing。
    fn mark_closing(&self) {
        self.inner.lock().expect("region overlay 状态锁").closing = true;
    }

    /// 截图命令守卫：record 路径绝不进截图命令（REQ-21）。
    fn ensure_screenshot_mode(&self) -> Result<(), String> {
        let inner = self.inner.lock().expect("region overlay 状态锁");
        if inner.mode.as_deref() == Some("record") {
            return Err(invalid(
                "E_INVALID_REGION",
                "区域录制模式不得调用截图命令（record 路径绝不进 take_region_screenshot）",
            ));
        }
        Ok(())
    }
}

fn normalize_overlay_mode(mode: Option<String>) -> Result<String, String> {
    match mode.as_deref().map(str::trim).unwrap_or("screenshot") {
        "" | "screenshot" => Ok("screenshot".to_string()),
        "record" => Ok("record".to_string()),
        other => Err(invalid(
            "E_INVALID_CONFIG",
            format!("未知区域覆盖层模式：{other}（允许 screenshot|record）"),
        )),
    }
}

/// close 排队后等待实际销毁的上限（同步短轮询；阻塞 async worker，不阻塞主线程）。
const OVERLAY_DESTROY_TIMEOUT: Duration = Duration::from_secs(2);

/// 可注入 probe 的销毁等待（测试 seam）：probe 返回 true = 已销毁。
fn wait_until_destroyed(timeout: Duration, mut probe: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if probe() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// 调用 `window.close()` 后轮询窗口是否真正销毁；**必须在 lifecycle 内调用**。
fn wait_for_overlay_destroyed(app: &AppHandle, label: &str, timeout: Duration) -> bool {
    wait_until_destroyed(timeout, || app.get_webview_window(label).is_none())
}

fn contains_snapped(rect: RectIn, m: &sr_enum::monitors::MonitorEntry) -> bool {
    let end_x = rect.x as i64 + rect.w as i64;
    let end_y = rect.y as i64 + rect.h as i64;
    rect.x >= m.x
        && rect.y >= m.y
        && end_x <= m.x as i64 + m.width_px as i64
        && end_y <= m.y as i64 + m.height_px as i64
}

/// 区域录制目标载荷（纯函数，L1 注入显示器列表）：单屏包含 + 向下取偶 + ≥64×64。
pub(crate) fn region_target_payload_from(
    request_id: &str,
    rect: RectIn,
    monitors: &[sr_enum::monitors::MonitorEntry],
) -> Result<RegionTargetAddedOut, String> {
    if rect.w == 0 || rect.h == 0 {
        return Err(invalid(
            "E_INVALID_REGION",
            format!("区域尺寸不得为 0（{}x{}）", rect.w, rect.h),
        ));
    }
    let snapped = RectIn {
        x: rect.x,
        y: rect.y,
        w: rect.w & !1,
        h: rect.h & !1,
    };
    if snapped.w < REGION_MIN_SIZE || snapped.h < REGION_MIN_SIZE {
        return Err(invalid(
            "E_INVALID_REGION",
            format!(
                "区域尺寸过小（吸附后 {}x{}，要求 ≥{REGION_MIN_SIZE}x{REGION_MIN_SIZE}）",
                snapped.w, snapped.h
            ),
        ));
    }
    let containing: Vec<&sr_enum::monitors::MonitorEntry> = monitors
        .iter()
        .filter(|m| contains_snapped(snapped, m))
        .collect();
    if containing.is_empty() {
        return Err(invalid(
            "E_INVALID_REGION",
            "区域必须完全位于单个显示器内（跨屏/空洞/越界均拒绝；v1 单屏限制）",
        ));
    }
    if containing.len() > 1 {
        return Err(invalid(
            "E_INVALID_REGION",
            "区域同时命中多个显示器（v1 仅接受完全位于单个显示器内）",
        ));
    }
    let monitor = containing[0];
    let device_name = monitor.gdi_device_name.clone().unwrap_or_default();
    if device_name.is_empty() && monitor.device_path.is_none() {
        return Err(invalid(
            "E_TARGET_LOST",
            "目标显示器缺少稳定设备路径与 GDI szDevice（不得用友好名代替）",
        ));
    }
    Ok(RegionTargetAddedOut {
        request_id: request_id.to_string(),
        rect: snapped,
        display: RegionDisplayOut {
            device_name,
            device_path: monitor.device_path.clone(),
        },
    })
}

#[tauri::command]
pub fn get_region_context(state: tauri::State<'_, RegionOverlayState>) -> RegionContextOut {
    state.context()
}

#[tauri::command]
pub async fn confirm_region_record(
    app: AppHandle,
    state: tauri::State<'_, RegionOverlayState>,
    request_id: String,
    rect: RectIn,
) -> Result<RegionTargetAddedOut, String> {
    // a) 一次性消费 + 代次快照（与 open/cancel 串行）。
    let generation = state.with_lifecycle(|| state.begin_confirm(&request_id))?;

    // a) 显示器枚举/尺寸校验在 lifecycle 外（慢操作不阻塞 cancel/open）。
    let payload = match sr_enum::monitors::list_monitors() {
        Ok(monitors) => match region_target_payload_from(&request_id, rect, &monitors) {
            Ok(p) => p,
            Err(e) => {
                state.with_lifecycle(|| state.restore_confirm(generation, &request_id));
                return Err(e);
            }
        },
        Err(e) => {
            state.with_lifecycle(|| state.restore_confirm(generation, &request_id));
            return Err(invalid("E_TARGET_LOST", format!("显示器枚举失败：{e}")));
        }
    };
    let window = app
        .get_webview_window(REGION_OVERLAY_LABEL)
        .ok_or_else(|| invalid("E_TARGET_LOST", "区域框选窗口不存在（请重新框选）"))?;

    // b) hide 段：check + hide + 失败恢复在同一 lifecycle 临界区；不在此等待 150ms。
    let hide_outcome: Result<(), String> =
        state.run_if_current(generation, &request_id, || -> Result<(), String> {
            match window.hide() {
                Ok(()) => Ok(()),
                Err(err) => {
                    state.restore_confirm(generation, &request_id);
                    let _ = window.show();
                    let _ = window.set_focus();
                    Err(invalid(
                        "E_INTERNAL",
                        format!("隐藏区域框选窗口失败: {err}"),
                    ))
                }
            }
        })?;
    hide_outcome?;

    // c) 等待段：lifecycle 外等待；此窗口内 cancel A / open B 可完成。
    if let Err(e) = blocking(|| {
        wait_for_composition();
        Ok(())
    })
    .await
    {
        // 等待任务异常：仅当前代次恢复可见；stale 则不 show/不 close。
        let restored: Result<(), String> = state.run_if_current(generation, &request_id, || {
            state.restore_confirm(generation, &request_id);
            let _ = window.show();
            let _ = window.set_focus();
        });
        if let Err(stale) = restored {
            return Err(stale);
        }
        return Err(e);
    }

    // d) 提交段：check + emit + close + clear 同一 lifecycle；cancel/open 不能插入。
    let committed: Result<(), String> =
        state.run_if_current(generation, &request_id, || -> Result<(), String> {
            if let Err(e) = app.emit(REGION_TARGET_ADDED_EVENT, &payload) {
                state.restore_confirm(generation, &request_id);
                let _ = window.show();
                let _ = window.set_focus();
                return Err(invalid(
                    "E_INTERNAL",
                    format!("交付区域录制目标失败（region_target_added）: {e}"),
                ));
            }
            state.mark_closing_if_current(generation, &request_id);
            let _ = window.close();
            let destroyed =
                wait_for_overlay_destroyed(&app, REGION_OVERLAY_LABEL, OVERLAY_DESTROY_TIMEOUT);
            if destroyed {
                state.clear_if_current(generation, &request_id);
            } else {
                eprintln!(
                    "[region] region_target_added 已交付，但 overlay 2s 内未确认销毁；\
                     保持 closing=true，拒绝后续 open 复用旧窗口"
                );
            }
            Ok(())
        })?;
    committed?;
    Ok(payload)
}

#[tauri::command]
pub async fn cancel_region_record(
    app: AppHandle,
    state: tauri::State<'_, RegionOverlayState>,
    request_id: String,
) -> Result<(), String> {
    state.with_lifecycle(|| -> Result<(), String> {
        if state.invalidate_record(&request_id) {
            let _ = app.emit(
                REGION_RECORD_CANCELLED_EVENT,
                RegionRecordCancelledOut {
                    request_id: request_id.clone(),
                },
            );
            state.mark_closing_if_request(&request_id);
            if let Some(w) = app.get_webview_window(REGION_OVERLAY_LABEL) {
                w.close()
                    .map_err(|e| invalid("E_INTERNAL", format!("关闭区域框选窗口失败: {e}")))?;
                if !wait_for_overlay_destroyed(&app, REGION_OVERLAY_LABEL, OVERLAY_DESTROY_TIMEOUT)
                {
                    return Err(invalid(
                        "E_INTERNAL",
                        "区域框选窗口关闭超时（保持 closing，拒绝复用旧窗口）",
                    ));
                }
            }
            state.clear();
        }
        Ok(())
    })
}

/// `get_overlay_geometry()` 回执（物理像素 + overlay 缩放）。
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct OverlayGeometryOut {
    /// 虚拟桌面物理原点 x（可为负，REQ-6）。
    pub origin_x: i32,
    /// 虚拟桌面物理原点 y（可为负）。
    pub origin_y: i32,
    /// 虚拟桌面物理宽（跨显示器并集）。
    pub width: u32,
    /// 虚拟桌面物理高。
    pub height: u32,
    /// overlay 缩放（DPI scale；前端物理 = origin + client*scale）。
    pub scale: f64,
}

/// 由各显示器物理矩形求虚拟桌面并集（纯函数，L1 单测覆盖负原点）。
fn union_bounds(rects: &[(i32, i32, u32, u32)]) -> Option<(i32, i32, u32, u32)> {
    let (mut min_x, mut min_y) = (i32::MAX, i32::MAX);
    let (mut max_x, mut max_y) = (i32::MIN, i32::MIN);
    for &(x, y, w, h) in rects {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x.saturating_add(w as i32));
        max_y = max_y.max(y.saturating_add(h as i32));
    }
    if min_x != i32::MAX && max_x > min_x && max_y > min_y {
        Some((min_x, min_y, (max_x - min_x) as u32, (max_y - min_y) as u32))
    } else {
        None
    }
}

fn monitor_bounds() -> Result<Vec<(i32, i32, u32, u32)>, String> {
    let monitors = sr_enum::monitors::list_monitors()
        .map_err(|e| format!("E_TARGET_LOST: 枚举显示器失败: {e}"))?;
    Ok(monitors
        .into_iter()
        .map(|m| (m.x, m.y, m.width_px, m.height_px))
        .collect())
}

/// overlay 缩放：优先已存在 overlay 的实际 scale_factor，否则主显示器；无显示器 → 1.0。
fn overlay_scale(app: &AppHandle) -> f64 {
    if let Some(w) = app.get_webview_window(REGION_OVERLAY_LABEL) {
        if let Ok(s) = w.scale_factor() {
            if s > 0.0 {
                return s;
            }
        }
    }
    app.primary_monitor()
        .ok()
        .flatten()
        .map(|m| m.scale_factor())
        .filter(|s| *s > 0.0)
        .unwrap_or(1.0)
}

/// §3.3/P1-1：虚拟桌面物理边界 + overlay 缩放。
#[tauri::command]
pub fn get_overlay_geometry(app: AppHandle) -> Result<OverlayGeometryOut, String> {
    let (origin_x, origin_y, width, height) = union_bounds(&monitor_bounds()?)
        .ok_or_else(|| "E_TARGET_LOST: 无活动显示器".to_string())?;
    Ok(OverlayGeometryOut {
        origin_x,
        origin_y,
        width,
        height,
        scale: overlay_scale(&app),
    })
}

/// §3.3/P1-1：创建/复用区域框选 overlay（无边框、置顶、覆盖虚拟桌面）。
///
/// 必须是 async 命令：sync 命令在 WebView2 IPC 回调（主线程）里执行，此时再同步创建
/// 第二个 WebView2 控制器会重入死锁——窗口已出现但 webview 永不初始化（无
/// Chrome_WidgetWin_1/Document、不绘制遮罩），invoke 也永不返回（前端 busy 卡死）。
/// async 命令由 Tauri 调度到异步运行时线程，窗口/控制器创建再切回主线程执行即可。
#[tauri::command]
pub async fn open_region_overlay(
    app: AppHandle,
    state: tauri::State<'_, RegionOverlayState>,
    mode: Option<String>,
    request_id: Option<String>,
) -> Result<(), String> {
    let mode = normalize_overlay_mode(mode)?;
    let request_id = request_id.unwrap_or_default().trim().to_string();
    if mode == "record" && request_id.is_empty() {
        return Err(invalid(
            "E_INVALID_CONFIG",
            "区域录制模式必须携带 request_id（主窗生成，用于代次复核）",
        ));
    }
    state.with_lifecycle(|| -> Result<(), String> {
        state.ensure_open_allowed()?;
        state.begin(&mode, &request_id);
        if let Some(w) = app.get_webview_window(REGION_OVERLAY_LABEL) {
            let _ = w.show();
            let _ = w.set_focus();
            // 复用窗口：通知页面按最新上下文（模式/request_id/代次）重新初始化。
            let _ = w.eval("window.__regionContextChanged && window.__regionContextChanged()");
            return Ok(());
        }
        let (origin_x, origin_y, width, height) = union_bounds(&monitor_bounds()?)
            .ok_or_else(|| "E_TARGET_LOST: 无活动显示器".to_string())?;
        let scale = overlay_scale(&app);
        let logical_x = origin_x as f64 / scale;
        let logical_y = origin_y as f64 / scale;
        let logical_w = width as f64 / scale;
        let logical_h = height as f64 / scale;
        let window = WebviewWindowBuilder::new(
            &app,
            REGION_OVERLAY_LABEL,
            WebviewUrl::App(REGION_OVERLAY_PAGE.into()),
        )
        .title(REGION_OVERLAY_LABEL)
        .decorations(false)
        .always_on_top(true)
        .resizable(false)
        .skip_taskbar(true)
        .shadow(false)
        .transparent(true)
        .inner_size(logical_w, logical_h)
        .position(logical_x, logical_y)
        .visible(true)
        .build()
        .map_err(|e| format!("E_INTERNAL: 创建区域框选窗口失败: {e}"))?;
        // 物理像素精确覆盖（混合 DPI/负原点下逻辑坐标可能取整偏差）。
        window
            .set_position(PhysicalPosition::new(origin_x, origin_y))
            .map_err(|e| format!("E_INTERNAL: 定位区域框选窗口失败: {e}"))?;
        window
            .set_size(PhysicalSize::new(width, height))
            .map_err(|e| format!("E_INTERNAL: 设置区域框选窗口尺寸失败: {e}"))?;
        let _ = window.set_focus();
        Ok(())
    })
}

/// §3.3/P1-1：关闭区域框选 overlay（不存在 → Ok，幂等）。
#[tauri::command]
pub async fn close_region_overlay(
    app: AppHandle,
    state: tauri::State<'_, RegionOverlayState>,
) -> Result<(), String> {
    state.with_lifecycle(|| -> Result<(), String> {
        let ctx = state.context();
        if ctx.mode == "record" && !ctx.request_id.is_empty() {
            let _ = app.emit(
                REGION_RECORD_CANCELLED_EVENT,
                RegionRecordCancelledOut {
                    request_id: ctx.request_id,
                },
            );
        }
        state.mark_closing();
        if let Some(w) = app.get_webview_window(REGION_OVERLAY_LABEL) {
            w.close()
                .map_err(|e| format!("E_INTERNAL: 关闭区域框选窗口失败: {e}"))?;
            if !wait_for_overlay_destroyed(&app, REGION_OVERLAY_LABEL, OVERLAY_DESTROY_TIMEOUT) {
                return Err(invalid(
                    "E_INTERNAL",
                    "区域框选窗口关闭超时（保持 closing，拒绝复用旧窗口）",
                ));
            }
        }
        state.clear();
        Ok(())
    })
}

// ─────────────────────────── P1-A 区域截图（后端主导） ───────────────────────────

/// 覆盖层隐藏后等待 DWM 完成合成的下限（毫秒；P1-A 判据 ≥150ms）。
pub const REGION_COMPOSE_SETTLE_MS: u64 = 150;

/// `take_region_screenshot(rect)` 回执：成品路径 + 物理像素元数据。
#[derive(Clone, Debug, Serialize)]
pub struct RegionScreenshotOut {
    /// 输出目录内的 PNG 路径。
    pub path: String,
    /// PNG 物理像素宽（与 IHDR 一致）。
    pub width_px: u32,
    /// PNG 物理像素高（与 IHDR 一致）。
    pub height_px: u32,
}

/// `RectIn`（物理像素，负坐标合法）→ sr-shot 区域目标；零尺寸显式拒绝。
fn region_shot_target(rect: RectIn) -> Result<ShotTarget, String> {
    if rect.w == 0 || rect.h == 0 {
        return Err(invalid(
            "E_INVALID_REGION",
            format!("区域尺寸不得为 0（{}x{}）", rect.w, rect.h),
        ));
    }
    Ok(ShotTarget::Region {
        rect: ScreenRect {
            x: rect.x,
            y: rect.y,
            width_px: rect.w,
            height_px: rect.h,
        },
    })
}

/// 等待 DWM 把 `Window::hide` 反映到合成表面。
///
/// `DwmFlush` 阻塞到当前合成提交；hide 是异步窗口状态变更，故先 flush 促成其完成，
/// 再等待固定下限（≥150ms，覆盖 DWM 帧间隔），最后再 flush 一次后允许截图。
fn wait_for_composition() {
    let _ = unsafe { DwmFlush() };
    std::thread::sleep(Duration::from_millis(REGION_COMPOSE_SETTLE_MS));
    let _ = unsafe { DwmFlush() };
}

/// P1-A/P2：区域截图（REQ-8）——隐藏 `region-overlay` → 等待合成 → sr-shot 物理像素
/// 截图 → 成功则关闭覆盖层；**失败则恢复覆盖层可见**（前端在覆盖层内显示错误，供重试
/// 或 Esc 取消），不静默关闭、不留下不可见窗口。
///
/// 仅此顺序能保证成品不含覆盖层遮罩（`rgba(5,8,14,0.55)`）与选区边框像素；
/// 前端（ui/region.js）只负责选区换算与调用本命令，Esc 取消路径不变。
#[tauri::command]
pub async fn take_region_screenshot(
    app: AppHandle,
    state: tauri::State<'_, RegionOverlayState>,
    rect: RectIn,
) -> Result<RegionScreenshotOut, String> {
    state.ensure_screenshot_mode()?;
    let target = region_shot_target(rect)?;
    let window = app
        .get_webview_window(REGION_OVERLAY_LABEL)
        .ok_or_else(|| invalid("E_TARGET_LOST", "区域框选窗口不存在（请重新打开区域框选）"))?;
    let output_dir = PathBuf::from(resolve_output_dir(&app, None));
    let shot_window = window.clone();
    let result = blocking(move || {
        shot_window
            .hide()
            .map_err(|e| invalid("E_INTERNAL", format!("隐藏区域框选窗口失败: {e}")))?;
        wait_for_composition();
        std::fs::create_dir_all(&output_dir).map_err(|e| {
            invalid(
                "E_OUTPUT_UNAVAILABLE",
                format!("创建截图输出目录失败（{}）：{e}", output_dir.display()),
            )
        })?;
        let saved = screenshot::take_screenshot(ScreenshotRequest { target, output_dir })
            .map_err(shot_error_string)?;
        Ok(RegionScreenshotOut {
            path: saved.path.to_string_lossy().into_owned(),
            width_px: saved.width_px,
            height_px: saved.height_px,
        })
    })
    .await;
    match result {
        Ok(out) => {
            // 成功：关闭覆盖层（截图已完成，窗口无需保留）。
            let _ = window.close();
            Ok(out)
        }
        Err(err) => {
            // P2：失败时覆盖层已被 hide，必须恢复可见并聚焦，前端 catch 里的
            // showError 才能被用户看到（原实现无条件 close → 失败提示随窗口消失）。
            let _ = window.show();
            let _ = window.set_focus();
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_bounds_handles_negative_origin_and_mixed_sizes() {
        // 主屏 1920x1080 @ (0,0)，左侧副屏 1280x1024 @ (-1280, 100)。
        let u = union_bounds(&[(0, 0, 1920, 1080), (-1280, 100, 1280, 1024)]).expect("应求并集");
        assert_eq!(u, (-1280, 0, 3200, 1124));
    }

    #[test]
    fn union_bounds_empty_is_none() {
        assert_eq!(union_bounds(&[]), None);
        assert_eq!(union_bounds(&[(0, 0, 0, 0)]), None);
    }

    #[test]
    fn region_shot_target_maps_physical_rect_and_rejects_empty() {
        let target = region_shot_target(RectIn {
            x: -1280,
            y: 100,
            w: 640,
            h: 480,
        })
        .expect("有效区域应映射成功");
        assert_eq!(
            target,
            ShotTarget::Region {
                rect: ScreenRect {
                    x: -1280,
                    y: 100,
                    width_px: 640,
                    height_px: 480
                }
            },
            "RectIn 应原样映射为物理像素 ScreenRect（负坐标合法，REQ-6）"
        );

        let err = region_shot_target(RectIn {
            x: 0,
            y: 0,
            w: 0,
            h: 10,
        })
        .expect_err("零宽区域必须拒绝");
        assert!(
            err.starts_with("E_INVALID_REGION"),
            "错误应带 E_INVALID_REGION 前缀（得到 {err}）"
        );
    }

    #[test]
    fn compose_settle_floor_is_at_least_150ms() {
        assert!(
            REGION_COMPOSE_SETTLE_MS >= 150,
            "合成等待下限应 ≥150ms（得到 {REGION_COMPOSE_SETTLE_MS}）"
        );
    }

    #[test]
    fn wait_for_composition_actually_waits_for_the_floor() {
        // 行为验证（不止常量）：等待函数本身必须消耗 ≥150ms，否则隐藏的覆盖层
        // 仍可能出现在截图中（DWM 帧间隔内提交旧的合成表面）。
        let started = std::time::Instant::now();
        wait_for_composition();
        assert!(
            started.elapsed() >= Duration::from_millis(REGION_COMPOSE_SETTLE_MS),
            "wait_for_composition 实际等待 {}ms，低于 {REGION_COMPOSE_SETTLE_MS}ms 下限",
            started.elapsed().as_millis()
        );
    }

    fn monitor(
        index: u32,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        gdi: Option<&str>,
        path: Option<&str>,
    ) -> sr_enum::monitors::MonitorEntry {
        sr_enum::monitors::MonitorEntry {
            index,
            is_primary: index == 0,
            x,
            y,
            width_px: w,
            height_px: h,
            friendly_name: Some(format!("Monitor {index}")),
            gdi_device_name: gdi.map(str::to_string),
            device_path: path.map(str::to_string),
            container_id: None,
        }
    }

    fn two_monitors() -> Vec<sr_enum::monitors::MonitorEntry> {
        vec![
            monitor(0, 0, 0, 1920, 1080, Some(r"\\.\DISPLAY1"), Some("path-0")),
            monitor(1, -1280, 100, 1280, 1024, Some(r"\\.\DISPLAY2"), None),
        ]
    }

    #[test]
    fn unit_region_record_geometry_150dpi_snap_and_single_screen() {
        // 150% DPI：client 300x200 → 物理 450x300；奇数尺寸向下取偶。
        let payload = region_target_payload_from(
            "req-1",
            RectIn {
                x: 150,
                y: 225,
                w: 451,
                h: 301,
            },
            &two_monitors(),
        )
        .expect("单屏内区域应接受");
        assert_eq!(
            payload.rect,
            RectIn {
                x: 150,
                y: 225,
                w: 450,
                h: 300
            }
        );
        assert_eq!(payload.display.device_name, r"\\.\DISPLAY1");
        assert_eq!(payload.display.device_path.as_deref(), Some("path-0"));

        // 跨屏（跨 0/1 号屏边界）→ 拒绝。
        let err = region_target_payload_from(
            "req-2",
            RectIn {
                x: -100,
                y: 100,
                w: 300,
                h: 200,
            },
            &two_monitors(),
        )
        .expect_err("跨屏必须拒绝");
        assert!(err.contains("单个显示器"), "得到 {err}");

        // <64×64（吸附后）→ 拒绝。
        let err = region_target_payload_from(
            "req-3",
            RectIn {
                x: 10,
                y: 10,
                w: 63,
                h: 100,
            },
            &two_monitors(),
        )
        .expect_err("最小尺寸必须拒绝");
        assert!(err.contains("过小"), "得到 {err}");

        // 负坐标副屏 + 仅 GDI 回退。
        let payload = region_target_payload_from(
            "req-4",
            RectIn {
                x: -1200,
                y: 200,
                w: 640,
                h: 480,
            },
            &two_monitors(),
        )
        .expect("负原点单屏区域应接受");
        assert_eq!(payload.display.device_name, r"\\.\DISPLAY2");
        assert!(payload.display.device_path.is_none(), "缺 path 时 GDI 回退");
    }

    #[test]
    fn unit_region_record_payload_rejects_no_device_identity() {
        let monitors = vec![monitor(0, 0, 0, 1920, 1080, None, None)];
        let err = region_target_payload_from(
            "req",
            RectIn {
                x: 0,
                y: 0,
                w: 100,
                h: 100,
            },
            &monitors,
        )
        .expect_err("无 path/GDI 必须拒绝（不得用友好名）");
        assert!(err.contains("E_TARGET_LOST"), "得到 {err}");
    }

    #[test]
    fn unit_region_context_generation_and_late_confirm_invalidation() {
        let state = RegionOverlayState::default();
        assert_eq!(
            state.context(),
            RegionContextOut {
                mode: "screenshot".into(),
                request_id: String::new()
            },
            "未打开时保持旧截图默认上下文"
        );
        state.begin("record", "req-a");
        assert_eq!(state.context().mode, "record");
        assert_eq!(state.context().request_id, "req-a");
        // 首次确认消费；重复确认（迟到）失效。
        let gen_a = state.begin_confirm("req-a").expect("首次确认");
        assert!(state.is_current_confirm(gen_a, "req-a"));
        let err = state
            .begin_confirm("req-a")
            .expect_err("一次性交付后重复确认必须失效");
        assert!(err.contains("E_INVALID_REQUEST"), "得到 {err}");
        // cancel 使迟到确认失效。
        state.begin("record", "req-b");
        assert!(state.invalidate_record("req-b"));
        let err = state
            .begin_confirm("req-b")
            .expect_err("cancel 后确认必须失效");
        assert!(err.contains("E_INVALID_REQUEST"), "得到 {err}");
        // 旧代次 request_id 不得确认新代次。
        state.begin("record", "req-c");
        let err = state
            .begin_confirm("req-b")
            .expect_err("旧 request_id 必须丢弃");
        assert!(err.contains("E_INVALID_REQUEST"), "得到 {err}");
        // 复用异模式：最新代次为准。
        state.begin("screenshot", "");
        let err = state
            .begin_confirm("req-c")
            .expect_err("截图模式不得走录制确认");
        assert!(err.contains("E_INVALID_REGION"), "得到 {err}");
    }

    /// M18b P1：确认等待边界必须用 generation+request_id 复核；restore 只恢复当前代次。
    #[test]
    fn unit_region_confirm_generation_survives_wait_boundary_only_when_current() {
        let state = RegionOverlayState::default();

        // A 已消费 pending、处于 hide/等待边界。
        state.begin("record", "req-a");
        let gen_a = state.begin_confirm("req-a").expect("A 首次确认");
        assert!(state.is_current_confirm(gen_a, "req-a"));

        // cancel A → 边界复核必须为 false；restore A 不得把已取消代次恢复。
        assert!(state.invalidate_record("req-a"));
        assert!(!state.is_current_confirm(gen_a, "req-a"));
        state.restore_confirm(gen_a, "req-a");
        let err = state
            .begin_confirm("req-a")
            .expect_err("cancel 后 restore 不得重新打开确认");
        assert!(err.contains("E_INVALID_REQUEST"), "得到 {err}");

        // open B（复用同一 overlay）→ A 的迟到路径 false；A 的 restore 不得影响 B。
        state.begin("record", "req-b");
        let gen_b = state.begin_confirm("req-b").expect("B 首次确认");
        assert!(state.is_current_confirm(gen_b, "req-b"));
        assert!(!state.is_current_confirm(gen_a, "req-a"));
        assert!(
            !state.invalidate_record("req-a"),
            "旧代次 cancel 不得关闭新会话"
        );
        assert_eq!(state.context().request_id, "req-b");
        state.restore_confirm(gen_a, "req-a");
        let err = state
            .begin_confirm("req-b")
            .expect_err("旧代次 restore 不得把新代次重新置为可确认");
        assert!(err.contains("E_INVALID_REQUEST"), "得到 {err}");
        assert_eq!(state.context().request_id, "req-b", "上下文仍为 B");

        // 当前代次 restore（仅错误重试路径）仍可恢复自身。
        state.restore_confirm(gen_b, "req-b");
        assert!(state.begin_confirm("req-b").is_ok(), "当前代次可恢复重试");
    }

    #[test]
    fn unit_region_lifecycle_hide_blocks_open_and_stale_commit_skips_side_effects() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{mpsc, Arc, Barrier};
        use std::thread;
        let state = Arc::new(RegionOverlayState::default());
        let gen_a = state.with_lifecycle(|| {
            state.begin("record", "req-a");
            state.begin_confirm("req-a").expect("A 首次确认")
        });
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let hide_done = Arc::new(AtomicBool::new(false));
        let (e, r, done, s_hide) = (
            entered.clone(),
            release.clone(),
            hide_done.clone(),
            state.clone(),
        );
        let hide = thread::spawn(move || {
            s_hide.run_if_current(gen_a, "req-a", || {
                e.wait();
                r.wait();
                done.store(true, Ordering::SeqCst);
                true
            })
        });
        entered.wait(); // A 已持有 lifecycle 并完成 check，模拟 hide 中

        // open B 必须被串行化到 A 的 hide 段之后。
        let (open_attempted, open_waiting) = mpsc::channel();
        let (tx, rx) = mpsc::channel();
        let s_open = state.clone();
        let open = thread::spawn(move || {
            open_attempted.send(()).expect("signal");
            let request = s_open.with_lifecycle(|| {
                s_open.begin("record", "req-b");
                s_open.context().request_id
            });
            tx.send(request).expect("open reply");
        });
        open_waiting
            .recv_timeout(Duration::from_secs(2))
            .expect("open 线程应开始");
        assert!(
            rx.recv_timeout(Duration::from_millis(150)).is_err(),
            "A hide 持有 lifecycle 时 open B 不得插入"
        );
        release.wait();
        assert!(hide.join().expect("hide thread").expect("A hide 应执行"));
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2))
                .expect("B 应在 A 后完成"),
            "req-b"
        );
        open.join().expect("open thread");

        // 等待段结束后的提交段：A stale → 不执行 emit/close 副作用。
        let side_effect = Arc::new(AtomicBool::new(false));
        let flag = side_effect.clone();
        let commit = state.run_if_current(gen_a, "req-a", || {
            flag.store(true, Ordering::SeqCst);
            true
        });
        assert!(commit.is_err(), "open B 后 A 的提交必须 stale");
        assert!(!side_effect.load(Ordering::SeqCst), "stale 不得 emit/close");
    }

    #[test]
    fn unit_region_lifecycle_cancel_and_old_request_do_not_close_new_session() {
        let state = RegionOverlayState::default();
        state.with_lifecycle(|| state.begin("record", "req-a"));
        let gen_a = state.with_lifecycle(|| state.begin_confirm("req-a").expect("A 首次确认"));
        assert!(state.with_lifecycle(|| state.invalidate_record("req-a")));
        // cancel A 后提交段 stale：emit/close 闭包不得执行。
        let mut side_effect = false;
        let commit = state.run_if_current(gen_a, "req-a", || {
            side_effect = true;
        });
        assert!(commit.is_err(), "cancel 后 A 提交必须 stale");
        assert!(!side_effect, "stale 不得执行 emit/close");

        // open B 复用同一 overlay；旧 request 的 cancel/提交均不得关闭 B。
        state.with_lifecycle(|| state.begin("record", "req-b"));
        assert!(
            !state.with_lifecycle(|| state.invalidate_record("req-a")),
            "旧代次 cancel 不关闭 B"
        );
        assert_eq!(state.context().request_id, "req-b");
        let mut side_effect = false;
        let commit = state.run_if_current(gen_a, "req-a", || {
            side_effect = true;
        });
        assert!(commit.is_err(), "旧代次提交必须 stale");
        assert!(!side_effect);
    }

    #[test]
    fn unit_region_lifecycle_success_commit_clears_only_current() {
        let state = RegionOverlayState::default();
        state.with_lifecycle(|| state.begin("record", "req-a"));
        let gen_a = state.with_lifecycle(|| state.begin_confirm("req-a").expect("A 首次确认"));
        state
            .run_if_current(gen_a, "req-a", || state.clear_if_current(gen_a, "req-a"))
            .expect("当前提交应执行");
        assert_eq!(state.context().mode, "screenshot", "提交后上下文已清理");

        // 新会话 B 打开后，A 的迟到 cancel/提交不得关闭 B。
        state.with_lifecycle(|| state.begin("record", "req-b"));
        assert!(!state.with_lifecycle(|| state.invalidate_record("req-a")));
        let mut side_effect = false;
        let commit = state.run_if_current(gen_a, "req-a", || {
            side_effect = true;
        });
        assert!(commit.is_err());
        assert!(!side_effect);
        assert_eq!(state.context().request_id, "req-b");
    }

    #[test]
    fn unit_region_closing_gate_blocks_open_until_cleared_and_old_clear_is_scoped() {
        let state = RegionOverlayState::default();
        state.with_lifecycle(|| state.begin("record", "req-a"));
        let gen_a = state.with_lifecycle(|| state.begin_confirm("req-a").expect("A 确认"));
        assert!(state.with_lifecycle(|| state.mark_closing_if_current(gen_a, "req-a")));
        assert!(state.is_closing());
        assert!(
            state
                .with_lifecycle(|| state.ensure_open_allowed())
                .is_err(),
            "closing=true 时 open 必须拒绝"
        );

        // 旧代次 clear 不得清掉新代次的 closing。
        state.with_lifecycle(|| state.begin("record", "req-b"));
        assert!(state.with_lifecycle(|| state.mark_closing_if_request("req-b")));
        state.with_lifecycle(|| state.clear_if_current(gen_a, "req-a"));
        assert!(state.is_closing(), "旧代次 clear 不得清除 B 的 closing");

        // 当前代次 clear 后允许 open。
        state.with_lifecycle(|| state.clear());
        assert!(!state.is_closing());
        assert!(state.with_lifecycle(|| state.ensure_open_allowed()).is_ok());
    }

    #[test]
    fn unit_region_closing_waits_for_real_destruction_before_open() {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
        use std::sync::Arc;
        let state = RegionOverlayState::default();
        let destroyed = Arc::new(AtomicBool::new(false));
        let probes = Arc::new(AtomicU32::new(0));
        state.with_lifecycle(|| state.begin("record", "req-a"));
        let gen_a = state.with_lifecycle(|| state.begin_confirm("req-a").expect("A 确认"));
        assert!(state.with_lifecycle(|| state.mark_closing_if_current(gen_a, "req-a")));
        assert!(
            state
                .with_lifecycle(|| state.ensure_open_allowed())
                .is_err(),
            "close 排队期间 open 必须拒绝"
        );

        let d = destroyed.clone();
        let p = probes.clone();
        let confirmed = wait_until_destroyed(Duration::from_millis(200), move || {
            p.fetch_add(1, Ordering::SeqCst);
            d.load(Ordering::SeqCst)
        });
        assert!(!confirmed, "probe 未确认销毁时不得返回成功");
        assert!(probes.load(Ordering::SeqCst) >= 2, "应短轮询多次");

        destroyed.store(true, Ordering::SeqCst);
        assert!(wait_until_destroyed(Duration::from_secs(1), {
            let d = destroyed.clone();
            move || d.load(Ordering::SeqCst)
        }));
        state.with_lifecycle(|| state.clear());
        assert!(state.with_lifecycle(|| state.ensure_open_allowed()).is_ok());
    }

    #[test]
    fn unit_region_lifecycle_commands_are_async() {
        // P1-1：cancel/close 不得在主线程同步阻塞 lifecycle；签名必须是 async。
        let src = include_str!("region.rs");
        for sig in [
            "pub async fn open_region_overlay",
            "pub async fn confirm_region_record",
            "pub async fn cancel_region_record",
            "pub async fn close_region_overlay",
        ] {
            assert!(src.contains(sig), "生命周期命令必须 async：{sig}");
        }
    }

    #[test]
    fn unit_region_record_path_never_enters_screenshot_command() {
        let state = RegionOverlayState::default();
        state.begin("record", "req");
        let err = state
            .ensure_screenshot_mode()
            .expect_err("record 模式必须拒绝截图命令");
        assert!(err.contains("E_INVALID_REGION"), "得到 {err}");
        state.begin("screenshot", "");
        assert!(state.ensure_screenshot_mode().is_ok(), "截图模式应允许");
    }
}
