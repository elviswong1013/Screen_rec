//! 最小进程身份解析（REQ-1 条目身份与 REQ-14 的公共底座）。
//!
//! 提供「PID → 创建时间(UNIX ms) + 映像名 + 应用名」的最小解析，供窗口/音频会话枚举
//! 共用（身份 = PID + 创建时间 + 路径校验，防 PID 复用，specs/001-screen-recorder.md §2.1）。
//! REQ-14 的完整有界规则身份解析（后代树、有界提升、系统宿主禁入）属 M4+ 任务，
//! 届时在本模块上扩展，不改变现有底座语义。

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

/// 进程身份（最小解析结果）。
pub(crate) struct ProcessIdentity {
    /// 进程创建时间（UNIX epoch 毫秒）。
    pub created_unix_ms: u64,
    /// 映像文件名（含扩展名），如 "Notepad.exe"。
    pub process_name: String,
    /// 应用名兜底：映像名去扩展名（非空）。
    pub app_name: String,
}

/// FILETIME（1601 epoch，100ns）→ UNIX epoch 毫秒。
fn filetime_to_unix_ms(ft: u64) -> u64 {
    const EPOCH_DIFF_100NS: u64 = 116_444_736_000_000_000;
    ft.saturating_sub(EPOCH_DIFF_100NS) / 10_000
}

/// 解析进程身份；任一步失败（进程已退出/受限进程）返回 None，由调用方决定跳过该条目
/// （REQ-14：身份不可靠不静默编造）。
pub(crate) fn process_identity(pid: u32) -> Option<ProcessIdentity> {
    // PROCESS_QUERY_LIMITED_INFORMATION 即可读取创建时间与映像路径（Vista+）。
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let identity = query_identity(handle);
    unsafe {
        let _ = CloseHandle(handle);
    }
    identity
}

fn query_identity(handle: HANDLE) -> Option<ProcessIdentity> {
    unsafe {
        let mut created = FILETIME::default();
        let mut exit_time = FILETIME::default();
        let mut kernel_time = FILETIME::default();
        let mut user_time = FILETIME::default();
        GetProcessTimes(
            handle,
            &mut created,
            &mut exit_time,
            &mut kernel_time,
            &mut user_time,
        )
        .ok()?;
        let raw = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
        let created_unix_ms = filetime_to_unix_ms(raw);

        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR::from_raw(buf.as_mut_ptr()),
            &mut len,
        )
        .ok()?;
        let full_path = String::from_utf16_lossy(&buf[..len as usize]);
        let process_name = full_path
            .rsplit(['\\', '/'])
            .next()
            .filter(|name| !name.is_empty())
            .map(str::to_string)?;
        // 应用名兜底：映像名去扩展名（如 "Notepad.exe" → "Notepad"）。
        let stem = process_name
            .rsplit_once('.')
            .map(|(stem, _)| stem)
            .unwrap_or(&process_name);
        let app_name = if stem.is_empty() {
            process_name.clone()
        } else {
            stem.to_string()
        };
        Some(ProcessIdentity {
            created_unix_ms,
            process_name,
            app_name,
        })
    }
}
