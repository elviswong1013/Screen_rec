//! 边界层日志/错误脱敏原语（M19c REQ-1，specs/005-log-redaction.md）。
//!
//! 默认 stderr 与用户可见错误不得携带完整路径、盘符、UNC 前缀或设备 ID；
//! 仅保留 basename 与 `.screenrec_tmp` 会话后缀以维持可诊断性。完整路径只能
//! 通过 `SR_DIAG_PATHS=1|true` 显式开启（REQ-4）。本模块不新增依赖；
//! **不**修改 [`ToolResolveError`](crate::tools::ToolResolveError) 的 `Display`
//! （`--self-check` 与既有工具解析测试依赖完整路径，REQ-5）。

use std::ffi::OsStr;
use std::path::Path;

use crate::tools::ToolResolveError;

/// 会话临时目录组件名：含该组件的路径保留从该组件起的后缀。
const SESSION_TMP_COMPONENT: &str = ".screenrec_tmp";

/// 通用脱敏占位（无 basename/空/根路径）。
const REDACTED: &str = "<redacted>";

/// 路径诊断开关环境变量（REQ-4）。
const SR_DIAG_PATHS_ENV: &str = "SR_DIAG_PATHS";

/// 纯函数：仅 `1` 或 ASCII 大小写不敏感的 `true` 视为开启；其余（含 `None`）关闭。
/// 独立于 [`paths_verbose`] 以便测试不触碰进程级环境变量。
pub fn paths_verbose_from(value: Option<&str>) -> bool {
    matches!(value, Some(v) if v == "1" || v.eq_ignore_ascii_case("true"))
}

/// 运行期包装：读取环境变量 `SR_DIAG_PATHS` 后委托 [`paths_verbose_from`]。
pub fn paths_verbose() -> bool {
    paths_verbose_from(std::env::var(SR_DIAG_PATHS_ENV).ok().as_deref())
}

/// 脱敏单个路径：
/// - 任一组件为 `.screenrec_tmp` 时，保留从该组件起的后缀（如 `.screenrec_tmp\<session>\seg.mkv`）；
/// - 否则有 basename 时返回 `<redacted>\<basename>`；
/// - 无 basename（空/根/纯父目录引用）返回 `<redacted>`。
///
/// 非 UTF-8 组件按 lossy 转换；绝不返回盘符、UNC 前缀或父目录；对任何输入不 panic。
pub fn redact_path(path: &Path) -> String {
    let components: Vec<_> = path.components().collect();
    if let Some(pos) = components
        .iter()
        .position(|c| c.as_os_str() == OsStr::new(SESSION_TMP_COMPONENT))
    {
        // 统一以 `\` 连接后缀，与输入分隔符风格无关（正斜杠输入输出同样形态）。
        return components[pos..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("\\");
    }
    match path.file_name() {
        Some(name) => format!("{REDACTED}\\{}", name.to_string_lossy()),
        None => REDACTED.to_string(),
    }
}

/// 将 `text` 中出现的已知路径替换为 [`redact_path`] 结果：
/// - 每个已知路径同时匹配 `Path::display()` 与正斜杠两种形式；
/// - 最长 needle 优先、ASCII 大小写不敏感；
/// - 空（或等价于空串的）已知路径忽略；替换产物不再参与后续匹配，无部分重叠残留；
/// - 不改动错误关键词/退出码等其余文本；对任何输入不 panic。
pub fn redact_text_known_paths(text: &str, known: &[&Path]) -> String {
    // (needle, replacement) 列表：display 与正斜杠两种形式，去重并忽略空 needle。
    let mut needles: Vec<(String, String)> = Vec::new();
    for path in known {
        let replacement = redact_path(path);
        let display = path.to_string_lossy().into_owned();
        let forward = display.replace('\\', "/");
        for needle in [display, forward] {
            if !needle.is_empty() && !needles.iter().any(|(n, _)| *n == needle) {
                needles.push((needle, replacement.clone()));
            }
        }
    }
    if needles.is_empty() {
        return text.to_string();
    }
    // 最长优先（稳定排序保持插入顺序）。
    needles.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

    let hay = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut emitted_to = 0usize; // 已原样输出的字节区间终点
    let mut i = 0usize;
    while i < hay.len() {
        // 仅在 UTF-8 字符边界尝试匹配，避免把多字节字符切坏（needle 经 lossy 可能含非 ASCII）。
        if text.is_char_boundary(i) {
            let mut hit: Option<(usize, &String)> = None;
            for (needle, replacement) in &needles {
                let pat = needle.as_bytes();
                if i + pat.len() <= hay.len() && hay[i..i + pat.len()].eq_ignore_ascii_case(pat) {
                    hit = Some((pat.len(), replacement));
                    break;
                }
            }
            if let Some((len, replacement)) = hit {
                out.push_str(&text[emitted_to..i]);
                out.push_str(replacement);
                i += len;
                emitted_to = i;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&text[emitted_to..]);
    out
}

/// 设备 ID 脱敏：默认错误/状态统一返回 `<device>`（不用于设备枚举 DTO，REQ-3）。
pub fn redact_device_id(_id: &str) -> &'static str {
    "<device>"
}

/// [`ToolResolveError`] 的脱敏渲染：保留工具名、尝试顺序与错误类别，
/// 路径替换为 [`redact_path`] 结果；**不**修改 [`ToolResolveError`] 的 `Display`。
pub fn redact_tool_error(err: &ToolResolveError) -> String {
    let mut out = format!("无法定位 {}（已尝试：", err.name);
    for p in &err.attempted {
        out.push_str("\n  - ");
        out.push_str(&redact_path(p));
    }
    out.push('）');
    out
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use super::*;

    /// 串行化涉及 `SR_DIAG_PATHS` 的测试，避免并行测试间的环境变量竞态。
    static SR_DIAG_PATHS_LOCK: Mutex<()> = Mutex::new(());

    /// 环境变量快照守卫：无论测试是否 panic 都恢复原值，避免泄漏状态。
    struct EnvSnapshot {
        saved: Option<std::ffi::OsString>,
    }

    impl EnvSnapshot {
        fn take() -> Self {
            Self {
                saved: std::env::var_os(SR_DIAG_PATHS_ENV),
            }
        }
    }

    impl Drop for EnvSnapshot {
        fn drop(&mut self) {
            match &self.saved {
                Some(v) => std::env::set_var(SR_DIAG_PATHS_ENV, v),
                None => std::env::remove_var(SR_DIAG_PATHS_ENV),
            }
        }
    }

    #[test]
    fn redact_path_drive_keeps_only_basename() {
        let p = PathBuf::from(r"C:\Users\elvis\Videos\session\seg.mkv");
        assert_eq!(redact_path(&p), r"<redacted>\seg.mkv");
    }

    #[test]
    fn redact_path_unc_and_verbatim() {
        assert_eq!(
            redact_path(Path::new(r"\\server\share\seg.mkv")),
            r"<redacted>\seg.mkv"
        );
        assert_eq!(
            redact_path(Path::new(r"\\?\C:\Users\x\seg.mkv")),
            r"<redacted>\seg.mkv"
        );
    }

    #[test]
    fn redact_path_forward_slash() {
        assert_eq!(
            redact_path(Path::new("C:/Users/elvis/seg.mkv")),
            r"<redacted>\seg.mkv"
        );
    }

    #[test]
    fn redact_path_relative() {
        assert_eq!(redact_path(Path::new("out/seg.mkv")), r"<redacted>\seg.mkv");
        assert_eq!(redact_path(Path::new("seg.mkv")), r"<redacted>\seg.mkv");
    }

    #[test]
    fn redact_path_empty_root_and_parent() {
        assert_eq!(redact_path(Path::new("")), "<redacted>");
        assert_eq!(redact_path(Path::new(r"C:\")), "<redacted>");
        assert_eq!(redact_path(Path::new(r"\")), "<redacted>");
        assert_eq!(redact_path(Path::new("..")), "<redacted>");
    }

    #[test]
    fn redact_path_screenrec_tmp_keeps_suffix() {
        assert_eq!(
            redact_path(Path::new(
                r"C:\Users\elvis\AppData\Local\Temp\.screenrec_tmp\s-123\seg.mkv"
            )),
            r".screenrec_tmp\s-123\seg.mkv"
        );
        // 正斜杠输入同样输出反斜杠后缀。
        assert_eq!(
            redact_path(Path::new("C:/Users/x/.screenrec_tmp/s-123/seg.mkv")),
            r".screenrec_tmp\s-123\seg.mkv"
        );
        // 后缀从 `.screenrec_tmp` 组件本身开始。
        assert_eq!(
            redact_path(Path::new(r"out\.screenrec_tmp")),
            ".screenrec_tmp"
        );
    }

    #[cfg(windows)]
    #[test]
    fn redact_path_non_utf8_is_lossy_without_panic() {
        use std::os::windows::ffi::OsStringExt;
        // 未配对代理项（非法 UTF-16）：lossy → U+FFFD；仅保留 basename，不 panic。
        let os = std::ffi::OsString::from_wide(&[
            0x0043, 0x003A, 0x005C, 0x0061, 0xD800, 0x002E, 0x006D, 0x006B, 0x0076,
        ]);
        assert_eq!(redact_path(Path::new(&os)), "<redacted>\\a\u{FFFD}.mkv");
    }

    #[test]
    fn redact_text_known_paths_replaces_longest_first() {
        let out_dir = PathBuf::from(r"C:\Users\elvis\Videos\out");
        let seg = out_dir.join("seg.mkv");
        let text = format!("E_DISK_FULL: 无法写入 {}（exit code 1）", seg.display());
        let redacted = redact_text_known_paths(&text, &[&out_dir, &seg]);
        // 最长优先：段路径整体替换，不留 `<redacted>\out\seg.mkv` 之类的部分残留。
        assert!(redacted.contains(r"<redacted>\seg.mkv"), "{redacted}");
        assert!(!redacted.contains(r"<redacted>\out"), "{redacted}");
        assert!(!redacted.contains(r"C:\Users\"), "{redacted}");
        // 错误关键词/退出码保留。
        assert!(redacted.contains("E_DISK_FULL"), "{redacted}");
        assert!(redacted.contains("exit code 1"), "{redacted}");
    }

    #[test]
    fn redact_text_known_paths_is_ascii_case_insensitive() {
        let known = PathBuf::from(r"C:\Users\elvis\Videos\out");
        let text = String::from("error: c:\\users\\elvis\\videos\\out\\seg.mkv denied");
        let redacted = redact_text_known_paths(&text, &[&known]);
        assert!(
            !redacted.to_ascii_lowercase().contains("users"),
            "{redacted}"
        );
        assert!(redacted.contains("denied"), "{redacted}");
    }

    #[test]
    fn redact_text_known_paths_matches_forward_slash_form() {
        let out_dir = PathBuf::from(r"C:\Users\elvis\Videos\out");
        let seg = out_dir.join("seg.mkv");
        let text = String::from("ffmpeg: C:/Users/elvis/Videos/out/seg.mkv: Permission denied");
        let redacted = redact_text_known_paths(&text, &[&out_dir, &seg]);
        assert!(!redacted.contains("C:/Users/"), "{redacted}");
        assert!(!redacted.contains(r"C:\Users\"), "{redacted}");
        assert!(redacted.contains(r"<redacted>\seg.mkv"), "{redacted}");
        assert!(redacted.contains("Permission denied"), "{redacted}");
    }

    #[test]
    fn redact_text_known_paths_tolerates_empty_inputs() {
        assert_eq!(redact_text_known_paths("", &[Path::new("a")]), "");
        assert_eq!(
            redact_text_known_paths("普通错误（exit 3）", &[Path::new("")]),
            "普通错误（exit 3）"
        );
        assert_eq!(
            redact_text_known_paths("abc", &[Path::new(""), Path::new("abc")]),
            "<redacted>\\abc"
        );
    }

    #[test]
    fn paths_verbose_from_only_accepts_one_or_true() {
        assert!(paths_verbose_from(Some("1")));
        assert!(paths_verbose_from(Some("true")));
        assert!(paths_verbose_from(Some("TRUE")));
        assert!(paths_verbose_from(Some("True")));
        assert!(!paths_verbose_from(None));
        assert!(!paths_verbose_from(Some("")));
        assert!(!paths_verbose_from(Some("0")));
        assert!(!paths_verbose_from(Some("yes")));
        assert!(!paths_verbose_from(Some(" false")));
        assert!(!paths_verbose_from(Some("1 ")));
    }

    #[test]
    fn paths_verbose_reads_sr_diag_paths() {
        let _lock = SR_DIAG_PATHS_LOCK.lock().expect("SR_DIAG_PATHS 锁");
        let _snapshot = EnvSnapshot::take();

        std::env::remove_var(SR_DIAG_PATHS_ENV);
        assert!(!paths_verbose());

        std::env::set_var(SR_DIAG_PATHS_ENV, "1");
        assert!(paths_verbose());

        std::env::set_var(SR_DIAG_PATHS_ENV, "TRUE");
        assert!(paths_verbose());

        std::env::set_var(SR_DIAG_PATHS_ENV, "0");
        assert!(!paths_verbose());

        std::env::set_var(SR_DIAG_PATHS_ENV, "yes");
        assert!(!paths_verbose());
    }

    #[test]
    fn redact_device_id_is_static_placeholder() {
        assert_eq!(redact_device_id(r"\\?\USB#VID_1234"), "<device>");
        assert_eq!(redact_device_id(""), "<device>");
    }

    #[test]
    fn redact_tool_error_keeps_name_order_and_redacts_paths() {
        let err = ToolResolveError {
            name: "ffprobe".to_string(),
            attempted: vec![
                PathBuf::from(r"C:\Users\elvis\app\bin\ffprobe.exe"),
                PathBuf::from(
                    r"C:\Users\elvis\AppData\Local\Temp\.screenrec_tmp\s-1\bin\ffprobe.exe",
                ),
            ],
        };
        let msg = redact_tool_error(&err);
        assert!(msg.starts_with("无法定位 ffprobe（已尝试："), "{msg}");
        assert!(msg.contains("<redacted>"), "{msg}");
        assert!(msg.contains("ffprobe.exe"), "{msg}");
        assert!(!msg.contains(r"C:\"), "{msg}");
        // 顺序保留：第一行仅 `<redacted>`，第二行保留 `.screenrec_tmp` 会话后缀。
        let first = msg.find("<redacted>").expect("<redacted> 出现");
        let second = msg.find(".screenrec_tmp").expect("会话后缀出现");
        assert!(first < second, "{msg}");
        assert!(msg.contains(r".screenrec_tmp\s-1\bin\ffprobe.exe"), "{msg}");
    }

    #[test]
    fn redact_tool_error_with_no_attempts() {
        let err = ToolResolveError {
            name: "ffmpeg".to_string(),
            attempted: vec![],
        };
        assert_eq!(redact_tool_error(&err), "无法定位 ffmpeg（已尝试：）");
    }
}
