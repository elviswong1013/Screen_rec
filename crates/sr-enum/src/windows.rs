//! 可见顶层窗口枚举（REQ-1/REQ-3，specs/001-screen-recorder.md §2.1）。
//!
//! - 仅列出可见（`IsWindowVisible`）且未 cloaked（`DWMWA_CLOAKED`=0）的顶层窗口；
//!   cloaked 与被挂起的 UWP 幽灵窗口同机制，一律不出现在结果中（REQ-1/REQ-3）。
//! - 条目身份 = HWND + PID + 属主进程创建时间（UNIX ms，防 PID 复用，REQ-1）。
//! - 应用名统一解析（M1 最小实现）：打包应用读窗口 `System.AppUserModel.ID` 属性
//!   （UWP/打包窗口由此归属真实应用标识，而非宿主进程名 ApplicationFrameHost）；
//!   未打包应用取属主进程映像名去扩展名。REQ-14 的完整有界规则解析在 M4+ 扩展。
//! - 刷新语义（REQ-3）：每次调用为一次完整重枚举，窗口消失后下一次调用即不再列出；
//!   单次调用为一次 `EnumWindows` 线性扫描（≤100 窗口预算 ≤2s，实测毫秒级）。

use std::collections::HashSet;

use sr_core::error::Error;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PROPVARIANT};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
};

/// 窗口条目（REQ-1）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowEntry {
    /// 窗口句柄（数值化 HWND）。
    pub hwnd: u64,
    /// 窗口属主进程 PID（非启动器 PID）。
    pub pid: u32,
    /// 属主进程创建时间（UNIX epoch 毫秒；身份字段，防 PID 复用）。
    pub process_created_at: u64,
    /// 窗口标题（GetWindowTextW 语义）。
    pub title: String,
    /// 应用显示名（非空；打包应用经 AppUserModelID 统一解析，其余取映像名）。
    pub app_name: String,
    /// 属主进程映像名，如 "Notepad.exe"。
    pub process_name: String,
}

/// 列出当前可见且未 cloaked 的顶层窗口。
pub fn list_windows() -> Result<Vec<WindowEntry>, Error> {
    let mut out: Vec<WindowEntry> = Vec::new();
    unsafe {
        // EnumWindows 同线程同步回调，以裸指针携带输出缓冲（与测试助手同模式）。
        EnumWindows(
            Some(enum_proc),
            LPARAM(&mut out as *mut Vec<WindowEntry> as isize),
        )
        .map_err(|e| Error::Internal(format!("EnumWindows 失败: {e}")))?;
    }
    // 以 HWND 去重（保序保留首个；防御性，正常枚举不产生重复）。
    let mut seen = HashSet::new();
    out.retain(|entry| seen.insert(entry.hwnd));
    Ok(out)
}

/// 回调契约：extern "system" 内禁止 unwinding——collect_window 全程无 panic 路径
/// （Option/Result 驱动，仅分配失败可中止进程）。
unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
    let out = &mut *(lparam.0 as *mut Vec<WindowEntry>);
    collect_window(hwnd, out);
    windows::core::BOOL::from(true)
}

fn collect_window(hwnd: HWND, out: &mut Vec<WindowEntry>) {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return;
        }
        // cloaked 过滤（REQ-1）：DWM 报告非 0 即挂起/幽灵窗口。
        // DWM 查询失败（窗口恰在销毁）视为未知，不过滤（与测试地面真值一致）。
        let mut cloaked = 0u32;
        let hr = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut core::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        );
        if hr.is_ok() && cloaked != 0 {
            return;
        }
        let mut buf = [0u16; 512];
        let len = GetWindowTextW(hwnd, &mut buf);
        let title = if len > 0 {
            String::from_utf16_lossy(&buf[..len as usize])
        } else {
            String::new()
        };
        // 身份不可得（进程已亡/受限）→ 不列出（REQ-14：身份不可靠不静默编造）。
        let Some(identity) = crate::identity::process_identity(pid) else {
            return;
        };
        // 应用名：打包应用 AppUserModelID → 映像名 → 窗口标题 → 兜底，保证非空。
        let app_name = window_aumid(hwnd)
            .or(Some(identity.app_name.clone()))
            .or_else(|| {
                if title.is_empty() {
                    None
                } else {
                    Some(title.clone())
                }
            })
            .unwrap_or_else(|| "Unknown".to_string());
        out.push(WindowEntry {
            hwnd: hwnd.0 as usize as u64,
            pid,
            process_created_at: identity.created_unix_ms,
            title,
            app_name,
            process_name: identity.process_name,
        });
    }
}

/// 窗口 `System.AppUserModel.ID` 属性（打包应用的统一应用标识，M1 最小 UWP 身份解析）。
fn window_aumid(hwnd: HWND) -> Option<String> {
    unsafe {
        let store: IPropertyStore = SHGetPropertyStoreForWindow(hwnd).ok()?;
        let mut pv = store.GetValue(&PKEY_AppUserModel_ID).ok()?;
        let value = propvariant_to_string(&mut pv);
        let _ = PropVariantClear(&mut pv);
        value.filter(|s| !s.is_empty())
    }
}

/// 仅读取 VT_LPWSTR 形态的 PROPVARIANT；其余形态返回 None（调用方负责 PropVariantClear）。
unsafe fn propvariant_to_string(pv: &mut PROPVARIANT) -> Option<String> {
    if unsafe { pv.Anonymous.Anonymous.vt } != VT_LPWSTR {
        return None;
    }
    let pwsz = unsafe { pv.Anonymous.Anonymous.Anonymous.pwszVal };
    if pwsz.0.is_null() {
        return None;
    }
    unsafe { pwsz.to_string() }.ok()
}
