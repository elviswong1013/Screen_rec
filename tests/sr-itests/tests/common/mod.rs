//! sr-itests 共享测试助手（L2 前置检查 + Win32 交叉验证原语）。
//!
//! 用途：为 specs/001-screen-recorder.md §4 的 L2 用例提供独立于被测实现的
//! 「地面真值」枚举（窗口/显示器/进程创建时间），以及 §4 的 BLOCKED 机制
//! （环境缺失 → 打印 BLOCKED 并 fail，不静默 skip）。
//!
//! 本模块只包含测试代码，不含任何产品逻辑。

#![allow(dead_code)]

use std::time::{Duration, Instant};

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_CLOAK, DWMWA_CLOAKED,
};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, DEVMODEW, ENUM_CURRENT_SETTINGS,
    HMONITOR, MONITORENUMPROC, MONITORINFOEXW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, EnumWindows, GetWindowTextW,
    GetWindowThreadProcessId, IsWindow, IsWindowVisible, RegisterClassW, SendMessageW,
    CW_USEDEFAULT, MONITORINFOF_PRIMARY, WINDOW_EX_STYLE, WM_CLOSE, WNDCLASSW, WNDENUMPROC,
    WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

// ─────────────────────────── L2 前置检查 / BLOCKED（§4） ───────────────────────────

/// §4 BLOCKED 机制：环境前置条件不满足 → 打印 BLOCKED 并使测试失败（绝不静默 skip）。
pub fn blocked(msg: &str) -> ! {
    panic!(
        "BLOCKED: {msg}（specs/001-screen-recorder.md §4：L2 环境缺失记 BLOCKED，SKIP 不算通过）"
    )
}

/// L2 要求交互式用户会话（Session 0 / 服务会话不合格，§4）。
pub fn require_interactive_session() {
    match std::env::var("SESSIONNAME") {
        Ok(v) if v == "Console" || v.starts_with("RDP-") => {}
        other => blocked(&format!(
            "需要交互式用户会话，当前 SESSIONNAME={other:?}（Session 0/服务会话不合格）"
        )),
    }
}

/// 记录 OS build（§4：L2 固定环境须记录 OS build）。
pub fn record_os_build() -> String {
    match std::process::Command::new("cmd")
        .args(["/c", "ver"])
        .output()
    {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            println!("[env] OS: {s}");
            s
        }
        Err(e) => blocked(&format!("无法读取 OS build（cmd /c ver 失败：{e}）")),
    }
}

/// 通用轮询等待：每 interval 求值一次 cond，命中即返回；超时返回 None。
pub fn wait_until<T>(
    timeout: Duration,
    interval: Duration,
    mut cond: impl FnMut() -> Option<T>,
) -> Option<T> {
    let start = Instant::now();
    loop {
        if let Some(v) = cond() {
            return Some(v);
        }
        if start.elapsed() >= timeout {
            return None;
        }
        std::thread::sleep(interval);
    }
}

// ─────────────────────────── 进程身份（地面真值） ───────────────────────────

/// FILETIME（1601 epoch，100ns）→ UNIX epoch 毫秒。
fn filetime_to_unix_ms(ft: u64) -> u64 {
    const EPOCH_DIFF_100NS: u64 = 116_444_736_000_000_000;
    (ft.saturating_sub(EPOCH_DIFF_100NS)) / 10_000
}

/// 进程创建时间（UNIX epoch 毫秒）——与 REQ-1 条目身份同源的地面真值。
pub fn process_created_unix_ms(pid: u32) -> Option<u64> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut created = Default::default();
        let mut exit_t = Default::default();
        let mut kernel = Default::default();
        let mut user = Default::default();
        let ok = GetProcessTimes(h, &mut created, &mut exit_t, &mut kernel, &mut user).is_ok();
        let _ = windows::Win32::Foundation::CloseHandle(h);
        if !ok {
            return None;
        }
        let raw = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
        Some(filetime_to_unix_ms(raw))
    }
}

/// 本测试进程自己的创建时间（供 ProbeWindow 身份断言）。
pub fn self_created_unix_ms() -> u64 {
    process_created_unix_ms(std::process::id())
        .unwrap_or_else(|| blocked("无法读取测试进程自身创建时间"))
}

// ─────────────────────────── 窗口枚举（地面真值） ───────────────────────────

/// 测试侧独立枚举到的顶层窗口快照。
#[derive(Clone, Debug)]
pub struct SysWindow {
    pub hwnd: u64,
    pub pid: u32,
    pub title: String,
    pub visible: bool,
    /// DWMWA_CLOAKED：0=未 cloaked；1=APP；2=SHELL；None=DWM 查询失败。
    pub cloaked: Option<u32>,
}

fn hwnd_from_u64(hwnd_u64: u64) -> HWND {
    HWND(hwnd_u64 as usize as *mut core::ffi::c_void)
}

/// DWMWA_CLOAKED 查询（0=未 cloaked；None=DWM 查询失败）。
pub fn cloak_of_u64(hwnd_u64: u64) -> Option<u32> {
    unsafe {
        let mut val: u32 = 0;
        let hr = DwmGetWindowAttribute(
            hwnd_from_u64(hwnd_u64),
            DWMWA_CLOAKED,
            &mut val as *mut u32 as *mut core::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        );
        if hr.is_ok() {
            Some(val)
        } else {
            None
        }
    }
}

pub fn is_window_alive_u64(hwnd_u64: u64) -> bool {
    unsafe { IsWindow(Some(hwnd_from_u64(hwnd_u64))).as_bool() }
}

/// EnumWindows 独立快照（含不可见窗口，便于调用方自行过滤）。
pub fn enum_sys_windows() -> Vec<SysWindow> {
    unsafe {
        let mut out: Vec<SysWindow> = Vec::new();
        let box_ptr = &mut out as *mut Vec<SysWindow>;
        let proc: WNDENUMPROC = Some(enum_cb);
        let _ = EnumWindows(proc, LPARAM(box_ptr as isize));
        out
    }
}

unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
    let out = &mut *(lparam.0 as *mut Vec<SysWindow>);
    let mut buf = [0u16; 512];
    let n = GetWindowTextW(hwnd, &mut buf);
    let title = if n > 0 {
        String::from_utf16_lossy(&buf[..n as usize])
    } else {
        String::new()
    };
    let mut pid: u32 = 0;
    let _tid = GetWindowThreadProcessId(hwnd, Some(&mut pid));
    out.push(SysWindow {
        hwnd: hwnd.0 as usize as u64,
        pid,
        title,
        visible: IsWindowVisible(hwnd).as_bool(),
        cloaked: cloak_of_u64(hwnd.0 as usize as u64),
    });
    windows::core::BOOL::from(true)
}

// ─────────────────────────── 测试自有窗口（确定性开/关目标） ───────────────────────────

/// 测试进程自建的可见顶层窗口：PID=测试进程、标题=唯一标记，可在 Drop 时销毁。
/// 用途：REQ-1 字段断言的确定性载体 + REQ-3 打开/关闭 + REQ-1 cloaked 过滤
/// （本机实测 Win11 记事本会合并标签页且启动器 PID ≠ 窗口属主 PID，
/// 纯 notepad 无法提供确定性的「新窗口/关闭窗口」语义，故以自建窗口为主、
/// notepad 为辅做跨应用字段验证）。
pub struct ProbeWindow {
    pub hwnd: HWND,
    pub hwnd_u64: u64,
    pub title: String,
}

static CLASS_REGISTERED: std::sync::Once = std::sync::Once::new();

unsafe extern "system" fn probe_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

fn register_probe_class() {
    CLASS_REGISTERED.call_once(|| unsafe {
        let hinst = GetModuleHandleW(None).ok().expect("GetModuleHandleW");
        // 绑定至局部变量使其存活到 RegisterClassW 之后（原写法 HSTRING 临时值在语句末
        // 释放，lpszClassName 悬垂，属 UB：并行测试下堆复用导致 ERROR_CANNOT_FIND_WND_CLASS）
        let class_name = HSTRING::from("SR_M1_RED_PROBE_WND");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(probe_wndproc),
            hInstance: hinst.into(),
            lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
            ..Default::default()
        };
        let atom = RegisterClassW(&wc);
        assert_ne!(atom, 0, "RegisterClassW 失败");
    });
}

impl ProbeWindow {
    pub fn new(marker: &str) -> ProbeWindow {
        register_probe_class();
        unsafe {
            let hinst = GetModuleHandleW(None).ok().expect("GetModuleHandleW");
            let class = HSTRING::from("SR_M1_RED_PROBE_WND");
            let title = HSTRING::from(marker);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR::from_raw(class.as_ptr()),
                PCWSTR::from_raw(title.as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                480,
                320,
                None,
                None,
                Some(hinst.into()),
                None,
            )
            .expect("CreateWindowExW 失败：无法构造测试窗口");
            ProbeWindow {
                hwnd,
                hwnd_u64: hwnd.0 as usize as u64,
                title: marker.to_string(),
            }
        }
    }

    /// DWMWA_CLOAK 置位/清除（制造/解除 cloaked 状态；DWMWA_CLOAKED 报告 DWM_CLOAKED_APP）。
    pub fn set_cloak(&self, on: bool) -> windows::core::Result<()> {
        unsafe {
            let val: i32 = if on { 1 } else { 0 };
            DwmSetWindowAttribute(
                self.hwnd,
                DWMWA_CLOAK,
                &val as *const i32 as *const core::ffi::c_void,
                std::mem::size_of::<i32>() as u32,
            )
        }
    }

    /// 地面真值：当前 DWMWA_CLOAKED 报告值。
    pub fn cloaked_state(&self) -> Option<u32> {
        cloak_of_u64(self.hwnd_u64)
    }

    pub fn is_alive(&self) -> bool {
        unsafe { IsWindow(Some(self.hwnd)).as_bool() }
    }

    pub fn destroy(&self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

impl Drop for ProbeWindow {
    fn drop(&mut self) {
        self.destroy();
    }
}

/// 给外部窗口（如记事本）发送 WM_CLOSE（关闭当前标签页/窗口，非强杀，避免影响用户其它标签）。
pub fn close_window(hwnd_u64: u64) {
    unsafe {
        let _ = SendMessageW(hwnd_from_u64(hwnd_u64), WM_CLOSE, None, None);
    }
}

// ─────────────────────────── 显示器枚举（地面真值，物理像素） ───────────────────────────

/// 来自 EnumDisplaySettingsW(ENUM_CURRENT_SETTINGS) 的物理分辨率 + 位置，
/// 以及 GetMonitorInfoW 的主显示器标识（DPI 虚拟化不影响 DEVMODE 数值）。
#[derive(Clone, Debug, PartialEq)]
pub struct SysMonitor {
    pub x: i32,
    pub y: i32,
    pub width_px: u32,
    pub height_px: u32,
    pub primary: bool,
}

pub fn enum_sys_monitors() -> Vec<SysMonitor> {
    unsafe {
        let mut out: Vec<SysMonitor> = Vec::new();
        let box_ptr = &mut out as *mut Vec<SysMonitor>;
        let proc: MONITORENUMPROC = Some(monitor_cb);
        let _ = EnumDisplayMonitors(None, None, proc, LPARAM(box_ptr as isize));
        out
    }
}

unsafe extern "system" fn monitor_cb(
    hmon: HMONITOR,
    _hdc: windows::Win32::Graphics::Gdi::HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> windows::core::BOOL {
    let out = &mut *(lparam.0 as *mut Vec<SysMonitor>);

    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    let info_ok = GetMonitorInfoW(
        hmon,
        &mut info.monitorInfo as *mut _ as *mut windows::Win32::Graphics::Gdi::MONITORINFO,
    )
    .as_bool();
    let primary = info_ok && (info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY) != 0;

    // 物理分辨率/位置（DEVMODE，不受进程 DPI 感知影响）
    let dev_name = HSTRING::from(wide_to_string(&info.szDevice));
    let mut dm = DEVMODEW::default();
    dm.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
    let (x, y, w, h) = if EnumDisplaySettingsW(
        PCWSTR::from_raw(dev_name.as_ptr()),
        ENUM_CURRENT_SETTINGS,
        &mut dm,
    )
    .as_bool()
    {
        let pos = dm.Anonymous1.Anonymous2.dmPosition;
        (pos.x, pos.y, dm.dmPelsWidth, dm.dmPelsHeight)
    } else if info_ok {
        // DEVMODE 查询失败时退回 MONITORINFO rcMonitor（可能被 DPI 虚拟化，仅兜底）
        let r = &info.monitorInfo.rcMonitor;
        (
            r.left,
            r.top,
            (r.right - r.left) as u32,
            (r.bottom - r.top) as u32,
        )
    } else {
        return windows::core::BOOL::from(true); // 跳过该显示器
    };

    out.push(SysMonitor {
        x,
        y,
        width_px: w,
        height_px: h,
        primary,
    });
    windows::core::BOOL::from(true)
}

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}
