//! 运行时工具定位（M11-C P1-5）：发行包不再依赖开发机源码目录。
//!
//! 统一解析 `ffmpeg.exe` / `ffprobe.exe`，按固定优先级尝试：
//! 1. 环境变量覆盖 `SCREENREC_FFMPEG_DIR`（目录本身，或其 `bin/` 子目录）；
//! 2. Tauri `resource_dir`（安装后；宿主在启动时 [`set_resource_dir`] 注册；
//!    资源目录本身或 `ffmpeg/` 子目录）；
//! 3. 当前 exe 同目录（及其 `ffmpeg/` 子目录；免安装/绿色布局）；
//! 4. 开发回退：`<仓库>/tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin`（
//!    `CARGO_MANIFEST_DIR` 编译期常量，仅开发机存在）。
//!
//! 解析失败返回 [`ToolResolveError`]，其中**逐条列出尝试过的路径**（诊断契约）。
//! 纯函数 [`resolve_with`] 支持注入搜索路径，供 L1 单测覆盖 ③/④ 与 env 覆盖。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// 环境变量覆盖名（指向包含 `ffmpeg.exe`/`ffprobe.exe` 的目录，或该目录的 `bin/`）。
pub const FFMPEG_DIR_ENV: &str = "SCREENREC_FFMPEG_DIR";

/// 解析来源（诊断/自检输出）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolSource {
    EnvOverride,
    ResourceDir,
    ExeAdjacent,
    DevFallback,
}

impl ToolSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::EnvOverride => "env:SCREENREC_FFMPEG_DIR",
            Self::ResourceDir => "tauri-resource-dir",
            Self::ExeAdjacent => "exe-adjacent",
            Self::DevFallback => "dev-fallback",
        }
    }
}

/// 解析结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTool {
    pub path: PathBuf,
    pub source: ToolSource,
}

/// 解析失败：`attempted` 为按优先级尝试过的全部候选路径。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolResolveError {
    pub name: String,
    pub attempted: Vec<PathBuf>,
}

impl std::fmt::Display for ToolResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "无法定位 {}（已尝试：", self.name)?;
        for p in &self.attempted {
            write!(f, "\n  - {}", p.display())?;
        }
        write!(f, "）")
    }
}

impl std::error::Error for ToolResolveError {}

/// 可注入的搜索路径集合（优先级即字段顺序）。
#[derive(Clone, Debug, Default)]
pub struct ToolSearchPaths {
    /// ① 环境变量覆盖目录。
    pub env_dir: Option<PathBuf>,
    /// ② Tauri resource 目录列表（安装后通常一项）。
    pub resource_dirs: Vec<PathBuf>,
    /// ③ 当前 exe 所在目录。
    pub exe_dir: Option<PathBuf>,
    /// ④ 开发回退目录列表（`.../tools/ffmpeg/<build>/bin`）。
    pub dev_dirs: Vec<PathBuf>,
}

fn exe_name(name: &str) -> String {
    if name.to_ascii_lowercase().ends_with(".exe") {
        name.to_string()
    } else {
        format!("{name}.exe")
    }
}

impl ToolSearchPaths {
    /// 按优先级展开候选（`(路径, 来源)`）；目录布局同时接受根目录与 `bin/`/`ffmpeg/`。
    pub fn candidates(&self, name: &str) -> Vec<(PathBuf, ToolSource)> {
        let file = exe_name(name);
        let mut out = Vec::new();
        let mut push = |p: PathBuf, s: ToolSource| {
            if !out.iter().any(|(q, _)| *q == p) {
                out.push((p, s));
            }
        };
        if let Some(dir) = &self.env_dir {
            push(dir.join(&file), ToolSource::EnvOverride);
            push(dir.join("bin").join(&file), ToolSource::EnvOverride);
        }
        for dir in &self.resource_dirs {
            push(dir.join(&file), ToolSource::ResourceDir);
            push(dir.join("ffmpeg").join(&file), ToolSource::ResourceDir);
        }
        if let Some(dir) = &self.exe_dir {
            push(dir.join(&file), ToolSource::ExeAdjacent);
            push(dir.join("ffmpeg").join(&file), ToolSource::ExeAdjacent);
        }
        for dir in &self.dev_dirs {
            push(dir.join(&file), ToolSource::DevFallback);
        }
        out
    }
}

/// 纯解析：按 [`ToolSearchPaths`] 优先级返回首个**存在的普通文件**。
pub fn resolve_with(paths: &ToolSearchPaths, name: &str) -> Result<ResolvedTool, ToolResolveError> {
    let candidates = paths.candidates(name);
    for (path, source) in &candidates {
        if path.is_file() {
            return Ok(ResolvedTool {
                path: path.clone(),
                source: *source,
            });
        }
    }
    Err(ToolResolveError {
        name: exe_name(name),
        attempted: candidates.into_iter().map(|(p, _)| p).collect(),
    })
}

static RESOURCE_DIR: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

fn resource_cell() -> &'static Mutex<Option<PathBuf>> {
    RESOURCE_DIR.get_or_init(|| Mutex::new(None))
}

/// 宿主注册安装后的 Tauri `resource_dir`（重复调用以最后一次为准；空/不存在的
/// 目录也会记录到候选并出现在错误信息中）。
pub fn set_resource_dir(dir: impl Into<PathBuf>) {
    *resource_cell().lock().expect("resource dir 锁") = Some(dir.into());
}

/// 当前已注册的 Tauri resource 目录（未注册 → `None`）。
pub fn registered_resource_dir() -> Option<PathBuf> {
    resource_cell().lock().expect("resource dir 锁").clone()
}

/// 开发回退目录（锁定 n8.1 构建；`CARGO_MANIFEST_DIR` 为编译期仓库内路径）。
pub fn dev_fallback_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin")
}

/// 运行时统一解析（env → Tauri resource → exe 同目录 → 开发回退）。
pub fn resolve_tool_binary(name: &str) -> Result<ResolvedTool, ToolResolveError> {
    resolve_with(&current_search_paths(), name)
}

/// 当前进程实际使用的搜索路径集合（诊断/`--self-check` 展示）。
pub fn current_search_paths() -> ToolSearchPaths {
    let env_dir = std::env::var_os(FFMPEG_DIR_ENV)
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty());
    ToolSearchPaths {
        env_dir,
        resource_dirs: registered_resource_dir().into_iter().collect(),
        exe_dir: std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf)),
        dev_dirs: vec![dev_fallback_dir()],
    }
}

/// 自检摘要（`--self-check` 用）：打印搜索根、逐工具命中来源/路径，并实际执行
/// `<tool> -version` 验证可用性。返回进程退出码（0 = 全部可用）。
pub fn self_check() -> i32 {
    let paths = current_search_paths();
    let show = |label: &str, dirs: &[PathBuf]| {
        if dirs.is_empty() {
            println!("{label}: (未设置)");
        } else {
            for d in dirs {
                println!("{label}: {}", d.display());
            }
        }
    };
    println!("[self-check] 搜索根（按优先级）");
    show(
        "  env  SCREENREC_FFMPEG_DIR",
        &paths.env_dir.iter().cloned().collect::<Vec<_>>(),
    );
    show("  tauri resource_dir", &paths.resource_dirs);
    show(
        "  exe 同目录",
        &paths.exe_dir.iter().cloned().collect::<Vec<_>>(),
    );
    show("  开发回退", &paths.dev_dirs);
    let mut code = 0;
    for name in ["ffmpeg", "ffprobe"] {
        match resolve_tool_binary(name) {
            Ok(t) => {
                println!(
                    "[self-check] {name}: {}（来源：{}）",
                    t.path.display(),
                    t.source.label()
                );
                match std::process::Command::new(&t.path).arg("-version").output() {
                    Ok(out) if out.status.success() => {
                        let first = String::from_utf8_lossy(&out.stdout)
                            .lines()
                            .next()
                            .unwrap_or("")
                            .to_string();
                        println!("[self-check]   -version: {first}");
                    }
                    Ok(out) => {
                        eprintln!("[self-check] {name} -version 失败（exit={}）", out.status);
                        code = 1;
                    }
                    Err(e) => {
                        eprintln!("[self-check] {name} 无法执行（{}）：{e}", t.path.display());
                        code = 1;
                    }
                }
            }
            Err(e) => {
                eprintln!("[self-check] {name}: {e}");
                code = 1;
            }
        }
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_suffix_is_idempotent() {
        assert_eq!(exe_name("ffmpeg"), "ffmpeg.exe");
        assert_eq!(exe_name("ffprobe.exe"), "ffprobe.exe");
    }
}
