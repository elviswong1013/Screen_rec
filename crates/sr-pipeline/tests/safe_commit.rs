//! L1 单元测试：会话临时目录 + 安全提交（REQ-24/D12，specs/001-screen-recorder.md
//! §2.5 REQ-24、§3.4；与 M3 既有 output_naming.rs 互补——本文件覆盖其未覆盖的
//! 「会话临时目录置于输出卷内」与「安全提交=临时名→验证→rename；禁止覆盖现有
//! 文件」，不重复 sanitize/时间戳/纯分配器面）。
//!
//! 运行方式（L1：随默认 `cargo test` 执行；无设备/交互会话依赖）：
//! ```text
//! cargo test -p sr-pipeline --test safe_commit -- --nocapture
//! ```
//!
//! 接口契约（测试即契约，详见 docs/milestones/m8-red-report.md §3.4）：
//! - `sr_pipeline::naming::SESSION_TMP_DIRNAME: &str = ".screenrec_tmp"`
//!   （REQ-24 默认会话临时目录名；输出目录内、输出卷同卷）。
//! - `sr_pipeline::naming::session_tmp_dir(output_dir: &Path, session_id: &str) -> PathBuf`
//!   = `output_dir/.screenrec_tmp/<session_id>`（**纯函数**：不创建目录、不要求存在）。
//! - `sr_pipeline::naming::safe_commit(tmp_path: &Path, final_path: &Path, verify)
//!    -> Result<(), CommitError>`：临时名→验证→rename 的顺序语义：
//!   ① `final_path` 已存在 → `Err(CommitError::FinalExists{..})`（**verify 不被调用**、
//!      双方文件均不得改动；禁止覆盖现有文件）；
//!   ② `verify(tmp_path)` 返回 Err → `Err(CommitError::VerifyFailed{detail})`
//!      （成品不出现、临时文件保留——可修复/重试）；
//!   ③ 全部通过 → 同卷 rename（临时文件消失、成品出现且字节一致）；
//!      跨卷 rename 失败 → `Err(CommitError::Io{..})`（临时文件保留）；
//!   ④ 失败路径均可重试（修复条件后重调成功）。
//! - `CommitError` 最小变体集：`FinalExists { final_path }` / `VerifyFailed { detail }` /
//!   `Io { detail }`。

use std::fs;
use std::path::{Path, PathBuf};

use sr_pipeline::naming::{
    next_available, safe_commit, session_tmp_dir, CommitError, SESSION_TMP_DIRNAME,
};

fn fresh_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr_m8_red_commit_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("创建临时目录");
    d
}

/// 找一个与 temp 不同卷的已存在卷根（用于跨卷 rename 分支；找不到 → None）。
fn other_volume_root() -> Option<PathBuf> {
    let temp_root = std::env::temp_dir();
    let this_root = temp_root
        .components()
        .next()
        .map(|c| c.as_os_str().to_string_lossy().to_string());
    for letter in b'C'..=b'Z' {
        let root = PathBuf::from(format!("{}:\\", letter as char));
        if !root.exists() {
            continue;
        }
        let same = this_root
            .as_deref()
            .is_some_and(|r| r.eq_ignore_ascii_case(&format!("{}:", letter as char)));
        if !same {
            return Some(root);
        }
    }
    None
}

#[test]
fn unit_session_tmp_dir_contract() {
    // 目录名：REQ-24/D12 默认 `.screenrec_tmp`（输出目录内、同卷）
    assert_eq!(
        SESSION_TMP_DIRNAME, ".screenrec_tmp",
        "会话临时目录名应为 .screenrec_tmp（REQ-24/D12）"
    );

    let out = PathBuf::from(r"C:\Users\demo\Videos\Screen_rec");
    let d = session_tmp_dir(&out, "20261017_101010_abcdef");
    assert_eq!(
        d,
        out.join(SESSION_TMP_DIRNAME).join("20261017_101010_abcdef"),
        "session_tmp_dir = <output_dir>/.screenrec_tmp/<session_id>"
    );
    assert!(!d.exists(), "session_tmp_dir 应为纯函数（不得创建目录）");

    let relative = PathBuf::from("out");
    let d2 = session_tmp_dir(&relative, "sess-1");
    assert!(
        d2.ends_with(Path::new(SESSION_TMP_DIRNAME).join("sess-1")),
        "相对输出目录同样组合出 <out>/.screenrec_tmp/<session_id>（得到 {d2:?}）"
    );
    assert_ne!(
        session_tmp_dir(&relative, "a"),
        session_tmp_dir(&relative, "b"),
        "不同会话应得到不同目录"
    );
}

#[test]
fn unit_safe_commit_no_overwrite_and_verify() {
    let dir = fresh_dir("sc");

    // ── ① 成功路径：验证通过 → 提交（临时名→验证→rename） ──
    let tmp = dir.join("tmp_a.mkv");
    fs::write(&tmp, b"NEW-CONTENT").expect("写入临时文件");
    let final_ok = dir.join("20261017_101010_Target.mkv");
    let mut verify_saw: Option<PathBuf> = None;
    safe_commit(&tmp, &final_ok, |p: &Path| {
        verify_saw = Some(p.to_path_buf());
        assert_eq!(
            fs::read(p).expect("verify 应能读到临时文件"),
            b"NEW-CONTENT"
        );
        Ok(())
    })
    .expect("验证通过时应提交成功");
    assert_eq!(
        verify_saw.as_deref(),
        Some(tmp.as_path()),
        "verify 应收到临时文件路径（提交前验证）"
    );
    assert!(!tmp.exists(), "提交后临时文件应消失（rename 语义）");
    assert_eq!(
        fs::read(&final_ok).expect("成品应存在"),
        b"NEW-CONTENT",
        "成品字节应与临时文件一致"
    );
    let next = next_available(&dir, "20261017_101010_Target", "mkv");
    assert_eq!(
        next,
        dir.join("20261017_101010_Target_1.mkv"),
        "提交后纯分配器应避开成品名（禁覆盖前提）"
    );

    // ── ② final 已存在 → FinalExists；verify 不被调用；双方内容不变 ──
    let tmp_b = dir.join("tmp_b.mkv");
    fs::write(&tmp_b, b"NEW-B").expect("写入临时文件 B");
    let mut verify_called = false;
    let err = safe_commit(&tmp_b, &final_ok, |_| {
        verify_called = true;
        Ok(())
    })
    .expect_err("final 已存在时必须拒绝（禁止覆盖现有文件）");
    match err {
        CommitError::FinalExists { ref final_path } => {
            assert_eq!(final_path, &final_ok, "FinalExists 应携带目标路径")
        }
        other => panic!("应报 CommitError::FinalExists（得到 {other:?}）"),
    }
    assert!(!verify_called, "final 已存在时应快速拒绝，不应调用 verify");
    assert_eq!(
        fs::read(&final_ok).unwrap(),
        b"NEW-CONTENT",
        "已存在成品不得被覆盖"
    );
    assert_eq!(
        fs::read(&tmp_b).unwrap(),
        b"NEW-B",
        "拒绝后临时文件应原样保留"
    );

    // ── ③ 验证失败 → VerifyFailed；成品不出现；临时文件保留；可重试 ──
    let tmp_c = dir.join("tmp_c.mkv");
    fs::write(&tmp_c, b"NEW-C").expect("写入临时文件 C");
    let final_c = dir.join("20261017_101010_Target2.mkv");
    let err = safe_commit(&tmp_c, &final_c, |_| {
        Err("ffprobe: 成品校验失败（测试注入）".into())
    })
    .expect_err("验证失败时不得提交");
    match err {
        CommitError::VerifyFailed { ref detail } => {
            assert!(
                detail.contains("测试注入"),
                "VerifyFailed.detail 应携带校验失败原因（得到 {detail:?}）"
            )
        }
        other => panic!("应报 CommitError::VerifyFailed（得到 {other:?}）"),
    }
    assert!(!final_c.exists(), "验证失败不得出现成品");
    assert_eq!(
        fs::read(&tmp_c).unwrap(),
        b"NEW-C",
        "验证失败时临时文件应保留（可修复/重试）"
    );
    // 重试：条件修复后（验证通过）应提交成功
    safe_commit(&tmp_c, &final_c, |_| Ok(())).expect("修复后重试应成功");
    assert_eq!(fs::read(&final_c).unwrap(), b"NEW-C");
    assert!(!tmp_c.exists());

    // 冲突消解后重试（②的修复路径）：移除占用后应可提交
    fs::remove_file(&final_ok).expect("移除占用文件");
    safe_commit(&tmp_b, &final_ok, |_| Ok(())).expect("移除冲突后应可提交");
    assert_eq!(fs::read(&final_ok).unwrap(), b"NEW-B");
    assert!(!tmp_b.exists());

    // ── ④ 跨卷 rename：Io 错误；临时文件保留、成品不出现 ──
    if let Some(other_root) = other_volume_root() {
        let tmp_d = dir.join("tmp_d.mkv");
        fs::write(&tmp_d, b"CROSS").expect("写入跨卷临时文件");
        let final_d = other_root.join(format!("sr_m8_red_cross_{}.mkv", std::process::id()));
        let err =
            safe_commit(&tmp_d, &final_d, |_| Ok(())).expect_err("跨卷 rename 应失败（同卷约束）");
        assert!(
            matches!(err, CommitError::Io { .. }),
            "跨卷应报 CommitError::Io（得到 {err:?}）"
        );
        assert!(tmp_d.exists(), "跨卷失败后临时文件应保留");
        assert!(!final_d.exists(), "跨卷失败不得留下成品");
        let _ = fs::remove_file(&final_d);
    } else {
        println!("[note] 单卷环境：跨卷 safe_commit 分支未执行（报告 §8 记录）");
    }

    let _ = fs::remove_dir_all(&dir);
}

/// P1-B 回归（确定性竞态）：verify 通过后、提交之前目标被外部创建。
///
/// 注入点 = verify 闭包内写入目标文件（与外部进程在同一时间窗口创建同名文件等价）。
/// 期望：提交失败（`FinalExists`）且**目标字节保持外部内容**（原子禁止替换），
/// 临时文件保留；移除冲突后重试成功（失败路径可重试）。
#[test]
fn unit_safe_commit_rejects_target_created_after_verify() {
    let dir = fresh_dir("race");
    let tmp = dir.join("tmp_race.mkv");
    fs::write(&tmp, b"NEW-RACE").expect("写入临时文件");
    let final_path = dir.join("20261017_101010_Race.mkv");

    let err = safe_commit(&tmp, &final_path, |p: &Path| {
        assert_eq!(fs::read(p).expect("verify 应能读到临时文件"), b"NEW-RACE");
        fs::write(&final_path, b"EXTERNAL-RACE").expect("模拟 verify 后外部创建同名目标");
        Ok(())
    })
    .expect_err("提交阶段目标已存在时必须失败（原子禁止替换）");

    match err {
        CommitError::FinalExists {
            final_path: reported,
        } => {
            assert_eq!(
                reported, final_path,
                "竞态失败应报 FinalExists 并携带目标路径"
            );
        }
        other => panic!("应报 CommitError::FinalExists（得到 {other:?}）"),
    }
    assert_eq!(
        fs::read(&final_path).expect("外部创建的目标应存在"),
        b"EXTERNAL-RACE",
        "已存在的目标字节不得被提交替换"
    );
    assert_eq!(
        fs::read(&tmp).expect("临时文件应保留"),
        b"NEW-RACE",
        "失败时临时文件应原样保留"
    );

    // 移除冲突后重试：失败路径未污染临时文件，可完成提交。
    fs::remove_file(&final_path).expect("移除竞态目标");
    safe_commit(&tmp, &final_path, |_| Ok(())).expect("移除冲突后重试应成功");
    assert_eq!(fs::read(&final_path).unwrap(), b"NEW-RACE");
    assert!(!tmp.exists());

    let _ = fs::remove_dir_all(&dir);
}
