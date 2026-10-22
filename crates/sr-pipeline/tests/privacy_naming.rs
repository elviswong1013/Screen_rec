//! M19a 隐私命名最小切片：窗口标题默认不进入输出文件名/manifest。
//!
//! 契约（任务规格）：
//! - `output_target_name(title, seq, false)` → `Window {seq}`（不含标题）；
//! - `output_target_name(title, seq, true)` → 原样返回标题（后续仍由
//!   `sanitize_target_name` 处理）；
//! - 不同序号在默认 false 下产生不同名称（多窗口不冲突）。

use sr_pipeline::naming::{output_target_name, sanitize_target_name};

#[test]
fn output_target_name_false_omits_window_title() {
    let name = output_target_name("Secret Document - Notepad", 7, false);
    assert_eq!(name, "Window 7", "默认 false 应使用通用目标名");
    assert!(
        !name.contains("Secret Document"),
        "默认 false 不得包含窗口标题（得到 {name:?}）"
    );
}

#[test]
fn output_target_name_true_keeps_window_title() {
    let title = "Secret Document - Notepad";
    let name = output_target_name(title, 7, true);
    assert_eq!(name, title, "true 应原样保留窗口标题");
    assert_eq!(
        sanitize_target_name(&name),
        sanitize_target_name(title),
        "true 时标题后续仍由 sanitize_target_name 处理"
    );
}

#[test]
fn output_target_name_false_seq_differs() {
    let a = output_target_name("Same Title", 1, false);
    let b = output_target_name("Same Title", 2, false);
    assert_ne!(a, b, "序号不同的窗口必须得到不同默认名");
    assert_eq!(a, "Window 1");
    assert_eq!(b, "Window 2");
}
