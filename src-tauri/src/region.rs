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
use std::time::Duration;

use serde::Serialize;
use sr_shot::screenshot::{self, ScreenRect, ScreenshotRequest, ShotTarget};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};
use windows::Win32::Graphics::Dwm::DwmFlush;

use crate::commands::{blocking, invalid, resolve_output_dir, shot_error_string, RectIn};

/// overlay 窗口 label（前端与后端共用；钉死）。
pub const REGION_OVERLAY_LABEL: &str = "region-overlay";
/// overlay 页面（Tauri app 资源相对路径；由前端任务提供）。
pub const REGION_OVERLAY_PAGE: &str = "region.html";

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
pub async fn open_region_overlay(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(REGION_OVERLAY_LABEL) {
        let _ = w.show();
        let _ = w.set_focus();
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
}

/// §3.3/P1-1：关闭区域框选 overlay（不存在 → Ok，幂等）。
#[tauri::command]
pub fn close_region_overlay(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(REGION_OVERLAY_LABEL) {
        w.close()
            .map_err(|e| format!("E_INTERNAL: 关闭区域框选窗口失败: {e}"))?;
    }
    Ok(())
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
    rect: RectIn,
) -> Result<RegionScreenshotOut, String> {
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
        // P1-A 判据：隐藏覆盖层后至少等待 150ms，确保 DWM 合成不再包含覆盖层。
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
}
