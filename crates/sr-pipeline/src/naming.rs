//! 输出命名（REQ-24/D12）：sanitize、时间戳、纯分配器、会话临时目录、安全提交。
//!
//! 契约（测试即契约，docs/milestones/m3-red-report.md §3.3 + output_naming.rs；
//! M8 面见 docs/milestones/m8-red-report.md §3.4 + safe_commit.rs）：
//! - `sanitize_target_name`：非法字符 `\/:*?"<>|` 与控制字符替换；安全字符（含
//!   Unicode）原样；结尾 `.`/空格 修剪；保留名（CON/PRN/AUX/NUL/COM1-9/LPT1-9，
//!   忽略大小写）转义为非保留且非空；
//! - `format_timestamp`：`yyyyMMdd_HHmmss`（15 字符）；偏移显式注入（分钟）；
//! - `next_available`：纯分配器（不创建文件）；无碰撞原名、碰撞 `_1`/`_2`…；
//!   返回时目标不存在（禁止覆盖）；目录状态纯函数。
//! - `session_tmp_dir`：`<output_dir>/.screenrec_tmp/<session_id>`（纯函数）；
//! - `safe_commit`：临时名→验证→原子「禁止替换」提交（Windows `MoveFileExW` 不带
//!   `MOVEFILE_REPLACE_EXISTING`）；禁止覆盖现有文件；失败保留临时文件。

use std::path::{Path, PathBuf};

/// Windows 保留设备名（大小写不敏感；含尾缀数字族）。
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn is_reserved(s: &str) -> bool {
    RESERVED.iter().any(|r| s.eq_ignore_ascii_case(r))
}

/// 目标名 → 安全文件名 stem 片段。
pub fn sanitize_target_name(raw: &str) -> String {
    // 1) 非法字符/控制字符 → '_'（保留安全片段，含 Unicode 原样）。
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if matches!(ch, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || ch.is_control() {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    // 2) 结尾 '.'/' ' 修剪（Windows 路径元素约束）。
    let trimmed = out.trim_end_matches(['.', ' ']);
    let mut s = trimmed.to_string();
    // 3) 保留名转义（大小写不敏感）与空结果兜底。
    if is_reserved(&s) {
        s.insert(0, '_');
    }
    if s.is_empty() {
        s.push_str("target");
    }
    s
}

/// UNIX 毫秒时间戳（显式 UTC 偏移，分钟）→ `yyyyMMdd_HHmmss`（15 字符）。
pub fn format_timestamp(unix_ms: u64, utc_offset_minutes: i32) -> String {
    // 秒级换算（i64 中间量；偏移可负，跨日回退由 civil 算法处理）。
    let total_secs = (unix_ms / 1000) as i64 + utc_offset_minutes as i64 * 60;
    let days = total_secs.div_euclid(86_400);
    let secs_of_day = total_secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hh, mm, ss) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );
    format!("{year:04}{month:02}{day:02}_{hh:02}{mm:02}{ss:02}")
}

/// `next_available(dir, stem, ext)`：碰撞即加 `_N` 序号（N≥1），永不覆盖。
pub fn next_available(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut n: u32 = 0;
    loop {
        let candidate = if n == 0 {
            dir.join(format!("{stem}.{ext}"))
        } else {
            dir.join(format!("{stem}_{n}.{ext}"))
        };
        if !candidate.exists() {
            return candidate;
        }
        n = n.checked_add(1).expect("序号溢出：目录内同名条目过多");
    }
}

/// Hinnant civil_from_days（天数自 1970-01-01 → (y, m, d)；公制历法，闰年正确）。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

// ─────────────────────────── 会话临时目录 + 安全提交（M8/REQ-24、D12） ───────────────────────────

/// REQ-24/D12：默认会话临时目录名（输出目录内、与输出同卷）。
pub const SESSION_TMP_DIRNAME: &str = ".screenrec_tmp";

/// REQ-24/D12：`<output_dir>/.screenrec_tmp/<session_id>`（**纯函数**：不创建目录、
/// 不要求存在）。
pub fn session_tmp_dir(output_dir: &Path, session_id: &str) -> PathBuf {
    output_dir.join(SESSION_TMP_DIRNAME).join(session_id)
}

/// 安全提交失败（REQ-24：临时名→验证→原子提交的顺序语义）。
#[derive(Debug)]
pub enum CommitError {
    /// 禁止覆盖现有文件：预先存在（verify 不被调用）或提交阶段被外部创建
    /// （P1-B 竞态；已存在目标字节保持不变）。
    FinalExists { final_path: PathBuf },
    /// 验证失败：成品不出现、临时文件保留（可修复/重试）。
    VerifyFailed { detail: String },
    /// rename/IO 失败（跨卷等）；临时文件保留。
    Io { detail: String },
}

/// REQ-24 安全提交：`final_path` 已存在 → [`CommitError::FinalExists`]（快速拒绝，
/// 不调用 verify）；否则 `verify(tmp_path)` → 同卷**原子禁止替换**提交 → 成品出现且
/// 字节一致。失败路径不触碰临时文件（保留可修复/重试）。
///
/// P1-B：`exists()` 只用于快速拒绝，真正的「不覆盖」由提交 API 原子裁决——
/// verify 与提交之间即使被外部创建同名文件，提交也会失败且目标字节不变。
pub fn safe_commit<F>(tmp_path: &Path, final_path: &Path, verify: F) -> Result<(), CommitError>
where
    F: FnOnce(&Path) -> Result<(), String>,
{
    if final_path.exists() {
        return Err(CommitError::FinalExists {
            final_path: final_path.to_path_buf(),
        });
    }
    verify(tmp_path).map_err(|detail| CommitError::VerifyFailed { detail })?;
    match rename_no_replace(tmp_path, final_path) {
        Ok(()) => Ok(()),
        // 提交阶段发现目标：外部在 verify 之后创建；OS 拒绝替换，目标字节未变。
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(CommitError::FinalExists {
            final_path: final_path.to_path_buf(),
        }),
        Err(e) => Err(CommitError::Io {
            detail: e.to_string(),
        }),
    }
}

/// 同卷原子提交（P1-B）：Windows `MoveFileExW` **不带** `MOVEFILE_REPLACE_EXISTING`。
///
/// 目标存在 → 失败（Win32 `ERROR_ALREADY_EXISTS`/`ERROR_FILE_EXISTS`，映射为
/// [`std::io::ErrorKind::AlreadyExists`]），不替换目标字节；跨卷 → `ERROR_NOT_SAME_DEVICE`
/// （`Io`）。检查与移动由文件系统在同一次操作中完成，不存在调用方可插入的竞态窗口。
fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::MoveFileExW;

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }
    let from_w = wide(from);
    let to_w = wide(to);
    unsafe {
        MoveFileExW(
            PCWSTR::from_raw(from_w.as_ptr()),
            PCWSTR::from_raw(to_w.as_ptr()),
            Default::default(),
        )
    }
    // `MoveFileExW` 失败来自 GetLastError：HRESULT 低 16 位即 Win32 错误码，
    // 还原为 io::Error 以复用 std 的 ErrorKind 映射（AlreadyExists / 跨卷等）。
    .map_err(|e| std::io::Error::from_raw_os_error(e.code().0 & 0xFFFF))
}
