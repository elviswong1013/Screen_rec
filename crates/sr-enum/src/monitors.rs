//! 显示器枚举（REQ-2，specs/001-screen-recorder.md §2.1）。
//!
//! 物理分辨率与位置来自 `EnumDisplaySettingsW(ENUM_CURRENT_SETTINGS)` 的 DEVMODE
//! （不受进程 DPI 感知影响的物理像素契约，REQ-2/REQ-6 同源）；主显示器标识来自
//! `GetMonitorInfoW` 的 `MONITORINFOF_PRIMARY`（有且仅有一台）。`index` 为 0..n-1
//! 稠密索引，作为 `RouteConfig::Monitor(idx)` 的引用基础（§3.4）。

use sr_core::error::Error;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, DEVMODEW, ENUM_CURRENT_SETTINGS,
    HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

/// 显示器条目（REQ-2）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorEntry {
    /// 稠密唯一索引 0..n-1（枚举序），供 RouteConfig::Monitor(idx) 引用。
    pub index: u32,
    /// 主显示器标识（MONITORINFOF_PRIMARY，有且仅有一台）。
    pub is_primary: bool,
    /// 桌面物理坐标 x（可为负，DEVMODE dmPosition 语义）。
    pub x: i32,
    /// 桌面物理坐标 y（可为负）。
    pub y: i32,
    /// 物理像素宽（DPI 无关）。
    pub width_px: u32,
    /// 物理像素高（DPI 无关）。
    pub height_px: u32,
}

/// 列出当前活动的显示设备。
pub fn list_monitors() -> Result<Vec<MonitorEntry>, Error> {
    let mut raw: Vec<RawMonitor> = Vec::new();
    unsafe {
        // EnumDisplayMonitors 同线程同步回调；返回 BOOL。
        EnumDisplayMonitors(
            None,
            None,
            Some(monitor_proc),
            LPARAM(&mut raw as *mut Vec<RawMonitor> as isize),
        )
        .ok()
        .map_err(|e| Error::Internal(format!("EnumDisplayMonitors 失败: {e}")))?;
    }
    Ok(raw
        .into_iter()
        .enumerate()
        .map(|(index, m)| MonitorEntry {
            index: index as u32,
            is_primary: m.primary,
            x: m.x,
            y: m.y,
            width_px: m.width_px,
            height_px: m.height_px,
        })
        .collect())
}

struct RawMonitor {
    x: i32,
    y: i32,
    width_px: u32,
    height_px: u32,
    primary: bool,
}

/// 回调契约：extern "system" 内禁止 unwinding——collect_monitor 全程无 panic 路径。
unsafe extern "system" fn monitor_proc(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> windows::core::BOOL {
    let out = &mut *(lparam.0 as *mut Vec<RawMonitor>);
    collect_monitor(hmonitor, out);
    windows::core::BOOL::from(true)
}

fn collect_monitor(hmonitor: HMONITOR, out: &mut Vec<RawMonitor>) {
    unsafe {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        let info_ok = GetMonitorInfoW(
            hmonitor,
            &mut info.monitorInfo as *mut _ as *mut MONITORINFO,
        )
        .as_bool();
        let primary = info_ok && (info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY) != 0;

        // 物理分辨率/位置：DEVMODE（不受进程 DPI 感知影响）；失败兜底 rcMonitor（可能被虚拟化）。
        let device = HSTRING::from(wide_to_string(&info.szDevice));
        let mut dm = DEVMODEW::default();
        dm.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        let (x, y, width_px, height_px) = if EnumDisplaySettingsW(
            PCWSTR::from_raw(device.as_ptr()),
            ENUM_CURRENT_SETTINGS,
            &mut dm,
        )
        .as_bool()
        {
            let pos = dm.Anonymous1.Anonymous2.dmPosition;
            (pos.x, pos.y, dm.dmPelsWidth, dm.dmPelsHeight)
        } else if info_ok {
            let r = &info.monitorInfo.rcMonitor;
            (
                r.left,
                r.top,
                (r.right - r.left) as u32,
                (r.bottom - r.top) as u32,
            )
        } else {
            return;
        };
        out.push(RawMonitor {
            x,
            y,
            width_px,
            height_px,
            primary,
        });
    }
}

fn wide_to_string(wide: &[u16]) -> String {
    let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..end])
}
