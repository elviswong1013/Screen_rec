//! L1 审计测试：许可与构建（REQ-29，specs/001-screen-recorder.md §2.8 REQ-29、
//! §3.2/§3.4、§4 `license_and_build_audit`、§8 M0 冻结项）。
//!
//! 性质：L1（随默认 `cargo test` 执行；无交互会话/设备依赖）；**对仓库内锁定
//! n8.1 release 构建实际运行**（`-version`/`-buildconf`/`-encoders` + LICENSE.txt
//! 核验），并核验 cargo-deny 配置与许可白名单、工具链锁定、源码获取说明。
//! 工具/构建缺失 → **BLOCKED**（fail，不静默 skip；绿阶段需补齐或由编排者裁定，
//! 见 docs/milestones/m8-red-report.md §8）。
//!
//! 运行方式：
//! ```text
//! cargo test -p sr-pipeline --test license_and_build_audit -- --nocapture
//! ```
//!
//! 接口契约（测试即契约，详见 docs/milestones/m8-red-report.md §3.5）：
//! - `sr_pipeline::sidecar::locked_build_facts() -> &'static LockedBuildFacts`
//!   （M0 冻结构建的锁定事实：版本标记/压缩包名/SHA256/buildconf 必含项/禁用编码器标记）。
//! - `sr_pipeline::sidecar::audit_locked_build(ffmpeg_home: &Path, zip_path: &Path)
//!    -> Result<BuildAudit, String>`：实际运行构建并核验压缩包校验和与许可文本。
//! - `BuildAudit { version, zip_sha256, buildconf_hits, forbidden_hits, license_text_ok }`。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sr_pipeline::sidecar::{audit_locked_build, locked_build_facts, BuildAudit, LockedBuildFacts};

/// §4 环境缺失语义（L1 工具/冻结构建缺失 → BLOCKED fail，不静默 skip）。
fn blocked(msg: &str) -> ! {
    panic!("BLOCKED: {msg}（specs/001-screen-recorder.md §4：审计构建/工具缺失按 BLOCKED 处理，SKIP 不算通过）")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("仓库根 canonicalize")
}

/// 从 deny.toml 文本提取 `[licenses] allow = [...]` 白名单（字符串列表形态；
/// 至 `]` 结束；`#` 注释剥除）。
fn extract_license_allow_list(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut in_list = false;
    for line in text.lines() {
        let l = line.split('#').next().unwrap_or("");
        if !in_list {
            if l.contains("allow") && l.contains('=') {
                in_list = true;
            } else {
                continue;
            }
        }
        for token in l.split('"').skip(1).step_by(2) {
            if !token.is_empty() {
                out.push(token.to_string());
            }
        }
        if l.contains(']') {
            break;
        }
    }
    out
}

/// docs/ 递归中存在记录 `needle` 的文本文件（REQ-29 来源/源码获取说明的仓库内锚点）。
fn docs_contain(dir: &Path, needle: &str) -> bool {
    let needle_l = needle.to_ascii_lowercase();
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if docs_contain(&path, needle) {
                return true;
            }
            continue;
        }
        let ext_ok = path
            .extension()
            .map(|e| {
                matches!(
                    e.to_string_lossy().to_ascii_lowercase().as_str(),
                    "md" | "txt" | "toml" | "json"
                )
            })
            .unwrap_or(false);
        if !ext_ok {
            continue;
        }
        if let Ok(text) = fs::read_to_string(&path) {
            if text.to_ascii_lowercase().contains(&needle_l) {
                return true;
            }
        }
    }
    false
}

#[test]
fn license_and_build_audit() {
    let root = repo_root();
    println!("[audit] 仓库根：{}", root.display());

    // ── 1. Rust 工具链锁定（REQ-29：rust-toolchain.toml；MSRV 按 tauri 2.12.4） ──
    let tc = fs::read_to_string(root.join("rust-toolchain.toml"))
        .expect("rust-toolchain.toml 缺失（REQ-29：Rust 工具链锁定）");
    assert!(
        tc.contains("channel = \"1.99.0\""),
        "工具链应锁定 channel = \"1.99.0\"（得到：{tc}）"
    );

    // ── 2. cargo-deny 配置与许可白名单（REQ-29：cargo deny 仅覆盖 Rust 依赖） ──
    let deny = fs::read_to_string(root.join("deny.toml"))
        .expect("deny.toml 缺失（REQ-29：cargo deny 配置；白名单须覆盖工作区许可并排除 GPL 系）");
    let allow = extract_license_allow_list(&deny);
    assert!(
        allow.len() >= 2,
        "deny.toml [licenses] allow 白名单解析失败（得到 {allow:?}）"
    );
    for required in ["MIT", "Apache-2.0"] {
        assert!(
            allow.iter().any(|a| a == required),
            "白名单应含 {required}（工作区声明 MIT OR Apache-2.0；得到 {allow:?}）"
        );
    }
    assert!(
        !allow.iter().any(|a| a.to_ascii_uppercase().contains("GPL")),
        "GPL 系（GPL/AGPL）不得进入白名单（spec §1：GPL 系禁入发布物）（得到 {allow:?}）"
    );
    let ws = fs::read_to_string(root.join("Cargo.toml")).expect("根 Cargo.toml 读取失败");
    assert!(
        ws.contains("license = \"MIT OR Apache-2.0\""),
        "workspace.package.license 应为 MIT OR Apache-2.0（REQ-29/许可筛查基线）"
    );
    println!("[audit] deny.toml 白名单：{allow:?}");

    // ── 3. 锁定 ffmpeg 构建实际审计（REQ-29：审计实际二进制） ──
    let facts: &LockedBuildFacts = locked_build_facts();
    assert_eq!(
        facts.zip_sha256.len(),
        64,
        "锁定压缩包 SHA256 应为 64 位十六进制"
    );
    assert!(
        facts.zip_sha256.chars().all(|c| c.is_ascii_hexdigit()),
        "锁定压缩包 SHA256 应仅含十六进制字符（得到 {})",
        facts.zip_sha256
    );
    assert!(
        !facts.version_marker.is_empty(),
        "版本标记不得为空（版本串核验锚点）"
    );

    let ff_home = root.join("tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1");
    let zip = root.join("tools/ffmpeg").join(facts.zip_file_name);
    if !ff_home.join("bin/ffmpeg.exe").exists() || !zip.exists() {
        blocked(&format!(
            "锁定 n8.1 构建或压缩包缺失：{} / {}（spec §8 冻结；REQ-29 审计对象）",
            ff_home.display(),
            zip.display()
        ));
    }
    let audit: BuildAudit = audit_locked_build(&ff_home, &zip).unwrap_or_else(|e| {
        panic!("锁定构建审计执行失败（REQ-29：buildconf/校验和/许可文本）：{e}")
    });
    println!("[audit] 版本：{}", audit.version);
    println!("[audit] 压缩包 SHA256：{}", audit.zip_sha256);
    assert!(
        audit.version.contains(facts.version_marker),
        "版本串应含锁定标记 {:?}（得到 {:?}）",
        facts.version_marker,
        audit.version
    );
    assert_eq!(
        audit.zip_sha256.to_ascii_uppercase(),
        facts.zip_sha256.to_ascii_uppercase(),
        "压缩包 SHA256 应与锁定事实一致（REQ-29：固定版本+来源+校验和）"
    );
    for (marker, hit) in &audit.buildconf_hits {
        assert!(*hit, "buildconf 应含 {marker}（LGPL/许可锚点；实际缺失）");
    }
    assert!(
        audit.forbidden_hits.is_empty(),
        "禁用/GPL 组件标记不得出现在构建内（得到 {:?}；spec §1 GPL 禁入）",
        audit.forbidden_hits
    );
    assert!(
        audit.license_text_ok,
        "随包 LICENSE.txt 应为 LGPL v3 文本（REQ-29：许可文本核验）"
    );

    // 来源/源码获取说明：锁定压缩包 SHA256 必须被仓库文档记录
    assert!(
        docs_contain(&root.join("docs"), &facts.zip_sha256),
        "docs/ 下应记录锁定压缩包 SHA256（REQ-29：来源/源码获取说明）：{}",
        facts.zip_sha256
    );

    // ── 4. cargo deny 实际执行（工具存在时必过；缺失 → BLOCKED） ──
    match Command::new("cargo").args(["deny", "--version"]).output() {
        Ok(o) if o.status.success() => {
            let out = Command::new("cargo")
                .args(["deny", "check", "licenses"])
                .current_dir(&root)
                .output()
                .expect("spawn cargo deny check licenses");
            let stdout_tail: String = String::from_utf8_lossy(&out.stdout)
                .lines()
                .rev()
                .take(15)
                .collect::<Vec<_>>()
                .join(" | ");
            let stderr_tail: String = String::from_utf8_lossy(&out.stderr)
                .lines()
                .rev()
                .take(15)
                .collect::<Vec<_>>()
                .join(" | ");
            println!("[audit] cargo deny check licenses exit={}（stdout 尾部：{stdout_tail}；stderr 尾部：{stderr_tail}）", out.status);
            assert!(
                out.status.success(),
                "cargo deny check licenses 应通过（REQ-29：cargo deny 覆盖 Rust 依赖）"
            );
        }
        _ => blocked(
            "cargo-deny 未安装（REQ-29 要求 `cargo deny` 覆盖 Rust 依赖；工具缺失按 BLOCKED 处理——\
             绿阶段需 `cargo install cargo-deny` 或由编排者裁定豁免口径）",
        ),
    }

    println!("[audit] 许可与构建审计通过：工具链锁定 + deny 白名单 + 锁定构建（buildconf/校验和/许可文本）+ 来源记录");
}
