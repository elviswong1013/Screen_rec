//! M11-C P1-5：运行时工具解析（ffmpeg/ffprobe）L1 测试。
//!
//! 覆盖优先级 ①env ②Tauri resource ③exe 同目录 ④开发回退；失败时错误信息逐条列出
//! 尝试路径。目录用临时文件模拟，不启动 ffmpeg。

use std::path::{Path, PathBuf};

use sr_core::tools::{
    dev_fallback_dir, resolve_tool_binary, resolve_with, ToolSearchPaths, ToolSource,
};

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr_m11_tools_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("创建临时目录失败");
    d
}

fn touch(path: &Path) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("创建候选目录失败");
    }
    std::fs::write(path, b"stub").expect("写候选文件失败");
}

#[test]
fn env_override_has_top_priority() {
    let root = temp_dir("env");
    let env = root.join("env-ffmpeg");
    let res = root.join("resources");
    let exe = root.join("app");
    let dev = root.join("dev");
    touch(&env.join("ffmpeg.exe"));
    touch(&env.join("ffprobe.exe"));
    touch(&res.join("ffmpeg/ffmpeg.exe"));
    touch(&exe.join("ffmpeg/ffmpeg.exe"));
    touch(&dev.join("ffmpeg.exe"));
    let paths = ToolSearchPaths {
        env_dir: Some(env.clone()),
        resource_dirs: vec![res],
        exe_dir: Some(exe),
        dev_dirs: vec![dev],
    };
    let got = resolve_with(&paths, "ffmpeg").expect("应命中 env");
    assert_eq!(got.path, env.join("ffmpeg.exe"));
    assert_eq!(got.source, ToolSource::EnvOverride);
    // env 目录也接受 bin/ 子目录。
    let root2 = temp_dir("env-bin");
    touch(&root2.join("env/bin/ffprobe.exe"));
    let paths2 = ToolSearchPaths {
        env_dir: Some(root2.join("env")),
        ..Default::default()
    };
    let got2 = resolve_with(&paths2, "ffprobe").expect("应命中 env/bin");
    assert_eq!(got2.path, root2.join("env/bin/ffprobe.exe"));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&root2);
}

#[test]
fn resource_dir_beats_exe_adjacent_and_dev() {
    let root = temp_dir("resource");
    let res = root.join("resources");
    let exe = root.join("app");
    let dev = root.join("dev");
    touch(&res.join("ffmpeg/ffprobe.exe"));
    touch(&exe.join("ffmpeg/ffprobe.exe"));
    touch(&dev.join("ffprobe.exe"));
    let paths = ToolSearchPaths {
        env_dir: None,
        resource_dirs: vec![res.clone()],
        exe_dir: Some(exe),
        dev_dirs: vec![dev],
    };
    let got = resolve_with(&paths, "ffprobe").expect("应命中 resource");
    assert_eq!(got.path, res.join("ffmpeg/ffprobe.exe"));
    assert_eq!(got.source, ToolSource::ResourceDir);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn exe_adjacent_layout_matches_simulated_install() {
    // ③ 模拟安装目录：<app>/screen-rec.exe + <app>/ffmpeg/{ffmpeg,ffprobe}.exe
    let root = temp_dir("exe");
    let app = root.join("Screen_rec");
    touch(&app.join("ffmpeg/ffmpeg.exe"));
    touch(&app.join("ffmpeg/ffprobe.exe"));
    let paths = ToolSearchPaths {
        env_dir: None,
        resource_dirs: vec![],
        exe_dir: Some(app.clone()),
        dev_dirs: vec![],
    };
    for name in ["ffmpeg", "ffprobe"] {
        let got = resolve_with(&paths, name).expect("应命中 exe 同目录");
        assert_eq!(got.path, app.join(format!("ffmpeg/{name}.exe")));
        assert_eq!(got.source, ToolSource::ExeAdjacent);
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn dev_fallback_is_last_and_reported() {
    let root = temp_dir("dev");
    let dev = root.join("tools-ffmpeg-bin");
    touch(&dev.join("ffmpeg.exe"));
    let paths = ToolSearchPaths {
        env_dir: Some(root.join("missing-env")),
        resource_dirs: vec![root.join("missing-res")],
        exe_dir: Some(root.join("missing-app")),
        dev_dirs: vec![dev.clone()],
    };
    let got = resolve_with(&paths, "ffmpeg").expect("应命中开发回退");
    assert_eq!(got.path, dev.join("ffmpeg.exe"));
    assert_eq!(got.source, ToolSource::DevFallback);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn missing_tool_lists_every_attempted_path() {
    let root = temp_dir("missing");
    let paths = ToolSearchPaths {
        env_dir: Some(root.join("env")),
        resource_dirs: vec![root.join("res")],
        exe_dir: Some(root.join("app")),
        dev_dirs: vec![root.join("dev")],
    };
    let err = resolve_with(&paths, "ffmpeg").expect_err("不存在时应失败");
    let text = err.to_string();
    for needle in [
        "env/ffmpeg.exe",
        "env/bin/ffmpeg.exe",
        "res/ffmpeg.exe",
        "res/ffmpeg/ffmpeg.exe",
        "app/ffmpeg.exe",
        "app/ffmpeg/ffmpeg.exe",
        "dev/ffmpeg.exe",
    ] {
        assert!(
            text.replace('\\', "/").contains(needle),
            "错误信息应列出 {needle}（实际：{text}）"
        );
    }
    assert!(text.starts_with("无法定位 ffmpeg.exe"), "错误前缀：{text}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn repository_dev_fallback_exists_for_both_tools() {
    // 开发机仓库内锁定 n8.1 构建存在；resolve_tool_binary 的 ④ 回退应命中。
    assert!(
        dev_fallback_dir().join("ffmpeg.exe").is_file(),
        "仓库内 ffmpeg 开发回退不存在"
    );
    assert!(
        dev_fallback_dir().join("ffprobe.exe").is_file(),
        "仓库内 ffprobe 开发回退不存在"
    );
    for name in ["ffmpeg", "ffprobe"] {
        let got = resolve_tool_binary(name).unwrap_or_else(|e| panic!("解析 {name} 失败：{e}"));
        assert!(
            matches!(
                got.source,
                ToolSource::DevFallback
                    | ToolSource::ExeAdjacent
                    | ToolSource::ResourceDir
                    | ToolSource::EnvOverride
            ),
            "来源应合法：{:?}",
            got.source
        );
        assert!(got.path.is_file());
    }
}
