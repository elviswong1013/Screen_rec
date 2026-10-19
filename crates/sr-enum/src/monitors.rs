//! 显示器枚举（REQ-2，specs/001-screen-recorder.md §2.1；M16/REQ-10 元数据链路）。
//!
//! 物理分辨率与位置来自 `EnumDisplaySettingsW(ENUM_CURRENT_SETTINGS)` 的 DEVMODE
//! （不受进程 DPI 感知影响的物理像素契约，REQ-2/REQ-6 同源）；主显示器标识来自
//! `GetMonitorInfoW` 的 `MONITORINFOF_PRIMARY`（有且仅有一台）。`index` 为 0..n-1
//! 稠密索引，作为 `RouteConfig::Monitor(idx)` 的引用基础（§3.4）。
//!
//! M16/REQ-10：保留 GDI 稠密顺序，以 `szDevice` 与 DisplayConfig `viewGdiDeviceName`
//! **按设备名字符串关联**（禁止按位置配对两个枚举列表）→ 友好名 + `monitorDevicePath`
//! → CM devnode → `DEVPKEY_Device_ContainerId`。元数据失败只降级字段（None），
//! 不丢显示器、不改 index/几何。

use sr_core::error::Error;
use windows::core::{GUID, HSTRING, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_PropertyW, CM_Get_Device_Interface_PropertyW, CM_Locate_DevNodeW,
    CM_LOCATE_DEVNODE_NORMAL, CR_SUCCESS,
};
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_PATH_SOURCE_INFO, DISPLAYCONFIG_PATH_TARGET_INFO,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Devices::Properties::{DEVPKEY_Device_ContainerId, DEVPROPTYPE};
use windows::Win32::Foundation::{ERROR_SUCCESS, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, DEVMODEW, ENUM_CURRENT_SETTINGS,
    HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

/// 显示器条目（REQ-2 + M16/REQ-10 元数据字段）。
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
    /// 显示器友好名（DisplayConfig；缺失/失败 → None）。
    pub friendly_name: Option<String>,
    /// GDI 设备名（`\\.\DISPLAY1`；M18/REQ-22 稳定路径不可得时的录制回退键）。
    pub gdi_device_name: Option<String>,
    /// `monitorDevicePath`（稳定设备路径；缺失 → None；UI 手选状态键）。
    pub device_path: Option<String>,
    /// PnP ContainerId（规范化小写无花括号；缺失/全零 → None）。
    pub container_id: Option<String>,
}

/// DisplayConfig 目标元数据（已按 GDI 名关联前的中间形状）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayTargetMeta {
    /// `DISPLAYCONFIG_SOURCE_DEVICE_NAME.viewGdiDeviceName`（如 `\\.\DISPLAY1`）。
    pub view_gdi_device_name: String,
    pub friendly_name: Option<String>,
    pub monitor_device_path: Option<String>,
    pub container_id: Option<String>,
}

/// 单显示器元数据（与 GDI 稠密顺序一一对应）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MonitorMetadata {
    pub friendly_name: Option<String>,
    pub device_path: Option<String>,
    pub container_id: Option<String>,
}

/// 按 GDI 设备名关联 DisplayConfig 元数据（纯函数；禁止位置配对）。
pub fn join_display_metadata(
    gdi_device_names: &[String],
    targets: &[DisplayTargetMeta],
) -> Vec<MonitorMetadata> {
    gdi_device_names
        .iter()
        .map(|name| {
            targets
                .iter()
                .find(|t| {
                    !t.view_gdi_device_name.trim().is_empty()
                        && t.view_gdi_device_name.eq_ignore_ascii_case(name.trim())
                })
                .map(|t| MonitorMetadata {
                    friendly_name: t.friendly_name.clone(),
                    device_path: t.monitor_device_path.clone(),
                    container_id: t.container_id.clone(),
                })
                .unwrap_or_default()
        })
        .collect()
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
    let names: Vec<String> = raw.iter().map(|m| m.device_name.clone()).collect();
    let targets = query_display_targets();
    let metas = join_display_metadata(&names, &targets);
    Ok(raw
        .into_iter()
        .enumerate()
        .map(|(index, m)| {
            let meta = metas.get(index).cloned().unwrap_or_default();
            MonitorEntry {
                index: index as u32,
                is_primary: m.primary,
                x: m.x,
                y: m.y,
                width_px: m.width_px,
                height_px: m.height_px,
                friendly_name: meta.friendly_name,
                gdi_device_name: (!m.device_name.trim().is_empty()).then_some(m.device_name),
                device_path: meta.device_path,
                container_id: meta.container_id,
            }
        })
        .collect())
}

struct RawMonitor {
    x: i32,
    y: i32,
    width_px: u32,
    height_px: u32,
    primary: bool,
    device_name: String,
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
        let device_name = wide_to_string(&info.szDevice);

        // 物理分辨率/位置：DEVMODE（不受进程 DPI 感知影响）；失败兜底 rcMonitor（可能被虚拟化）。
        let device = HSTRING::from(device_name.as_str());
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
            device_name,
        });
    }
}

fn query_display_targets() -> Vec<DisplayTargetMeta> {
    unsafe {
        let mut path_count = 0u32;
        let mut mode_count = 0u32;
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
            != ERROR_SUCCESS
        {
            return Vec::new();
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        if QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            None,
        ) != ERROR_SUCCESS
        {
            return Vec::new();
        }
        paths.truncate(path_count as usize);
        let mut out = Vec::new();
        for path in &paths {
            let Some(gdi) = source_device_name(&path.sourceInfo) else {
                continue;
            };
            let (friendly_name, monitor_device_path) = target_device_name(&path.targetInfo);
            let container_id = monitor_device_path
                .as_deref()
                .and_then(container_id_for_device_path);
            out.push(DisplayTargetMeta {
                view_gdi_device_name: gdi,
                friendly_name,
                monitor_device_path,
                container_id,
            });
        }
        out
    }
}

unsafe fn source_device_name(info: &DISPLAYCONFIG_PATH_SOURCE_INFO) -> Option<String> {
    let mut name = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
    name.header = DISPLAYCONFIG_DEVICE_INFO_HEADER {
        r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
        size: std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
        adapterId: info.adapterId,
        id: info.id,
    };
    if DisplayConfigGetDeviceInfo(&mut name.header as *mut DISPLAYCONFIG_DEVICE_INFO_HEADER) != 0 {
        return None;
    }
    let gdi = wide_to_string(&name.viewGdiDeviceName);
    (!gdi.trim().is_empty()).then_some(gdi)
}

unsafe fn target_device_name(
    info: &DISPLAYCONFIG_PATH_TARGET_INFO,
) -> (Option<String>, Option<String>) {
    let mut name = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
    name.header = DISPLAYCONFIG_DEVICE_INFO_HEADER {
        r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
        size: std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
        adapterId: info.adapterId,
        id: info.id,
    };
    if DisplayConfigGetDeviceInfo(&mut name.header as *mut DISPLAYCONFIG_DEVICE_INFO_HEADER) != 0 {
        return (None, None);
    }
    let friendly = wide_to_string(&name.monitorFriendlyDeviceName);
    let path = wide_to_string(&name.monitorDevicePath);
    (
        (!friendly.trim().is_empty()).then_some(friendly),
        (!path.trim().is_empty()).then_some(path),
    )
}

fn container_id_for_device_path(device_path: &str) -> Option<String> {
    let id = HSTRING::from(device_path);
    unsafe {
        // monitorDevicePath 是**设备接口路径**：先按接口属性读取 ContainerId。
        let mut prop_type = DEVPROPTYPE::default();
        let mut buf = [0u8; 16];
        let mut size = buf.len() as u32;
        let interface_ok = CM_Get_Device_Interface_PropertyW(
            PCWSTR::from_raw(id.as_ptr()),
            &DEVPKEY_Device_ContainerId,
            &mut prop_type,
            Some(buf.as_mut_ptr()),
            &mut size,
            0,
        ) == CR_SUCCESS
            && size >= 16;
        if interface_ok {
            let guid = std::ptr::read_unaligned(buf.as_ptr() as *const GUID);
            if let Some(value) = crate::render_endpoints::guid_string(&guid) {
                return Some(value);
            }
        }

        // 兜底：按 devnode 属性读取（部分驱动/接口路径形态）。
        let mut devinst = 0u32;
        if CM_Locate_DevNodeW(
            &mut devinst,
            PCWSTR::from_raw(id.as_ptr()),
            CM_LOCATE_DEVNODE_NORMAL,
        ) != CR_SUCCESS
        {
            return None;
        }
        let mut prop_type = DEVPROPTYPE::default();
        let mut buf = [0u8; 16];
        let mut size = buf.len() as u32;
        if CM_Get_DevNode_PropertyW(
            devinst,
            &DEVPKEY_Device_ContainerId,
            &mut prop_type,
            Some(buf.as_mut_ptr()),
            &mut size,
            0,
        ) != CR_SUCCESS
            || size < 16
        {
            return None;
        }
        let guid = std::ptr::read_unaligned(buf.as_ptr() as *const GUID);
        crate::render_endpoints::guid_string(&guid)
    }
}

fn wide_to_string(wide: &[u16]) -> String {
    let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..end])
}
