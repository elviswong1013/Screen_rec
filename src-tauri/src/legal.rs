//! M19d1：About/许可证入口（固定 allowlist；不读取任意路径）。
//!
//! 只暴露已随包的 5 份 FFmpeg 文档。UI 只传固定 id；后端从资源目录/exe 同目录/
//! 开发回退解析，canonicalize 后校验仍位于允许根，限制 4 MiB，UTF-8 失败返回通用错误。

use std::path::{Path, PathBuf};

use serde::Serialize;

const MAX_DOC_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Copy)]
struct DocDef {
    id: &'static str,
    file: &'static str,
    title: &'static str,
}

const DOCS: [DocDef; 5] = [
    DocDef {
        id: "ffmpeg_license",
        file: "FFmpeg-LICENSE.txt",
        title: "FFmpeg LGPLv3 许可证",
    },
    DocDef {
        id: "gplv3",
        file: "GPLv3.txt",
        title: "GNU GPLv3 文本",
    },
    DocDef {
        id: "ffmpeg_source_offer",
        file: "ffmpeg-source-offer.txt",
        title: "FFmpeg 对应源码说明",
    },
    DocDef {
        id: "ffmpeg_build_info",
        file: "ffmpeg-build-info.txt",
        title: "FFmpeg 构建信息",
    },
    DocDef {
        id: "ffmpeg_build_script",
        file: "ffmpeg-build-script.sh",
        title: "FFmpeg 构建脚本",
    },
];

#[derive(Clone, Serialize)]
pub struct LegalDocInfo {
    pub id: String,
    pub title: String,
    pub status: String,
    pub bytes: u64,
}

#[derive(Clone, Serialize)]
pub struct AppInfoOut {
    pub version: String,
    pub ffmpeg_build: String,
    pub docs: Vec<LegalDocInfo>,
}

#[derive(Clone, Serialize)]
pub struct LegalDocOut {
    pub id: String,
    pub text: String,
    pub bytes: u64,
}

fn doc_by_id(id: &str) -> Option<&'static DocDef> {
    DOCS.iter().find(|d| d.id == id)
}

/// 资源根顺序：安装后的 resource_dir → exe 同目录 → 开发回退。
/// 同时尝试根目录与 `licenses/` 子目录；文件名仍来自固定 allowlist。
fn candidate_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut push = |p: PathBuf| {
        if !roots.iter().any(|q| *q == p) {
            roots.push(p);
        }
    };
    if let Some(dir) = sr_core::tools::registered_resource_dir() {
        push(dir.join("licenses"));
        push(dir);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            push(dir.join("licenses"));
            push(dir.to_path_buf());
        }
    }
    push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/licenses"));
    roots
}

fn read_doc_in_roots(def: &DocDef, roots: &[PathBuf]) -> Result<(String, u64), String> {
    for root in roots {
        let candidate = root.join(def.file);
        if !candidate.is_file() {
            continue;
        }
        let root = std::fs::canonicalize(root).map_err(|_| "legal document unavailable")?;
        let path = std::fs::canonicalize(&candidate).map_err(|_| "legal document unavailable")?;
        if !path.starts_with(&root) {
            return Err("legal document unavailable".into());
        }
        let meta = std::fs::metadata(&path).map_err(|_| "legal document unavailable")?;
        if meta.len() > MAX_DOC_BYTES {
            return Err("legal document too large".into());
        }
        let text = std::fs::read_to_string(&path).map_err(|_| "legal document unreadable")?;
        return Ok((text, meta.len()));
    }
    Err("legal document unavailable".into())
}

fn parse_ffmpeg_build(text: &str) -> String {
    let commit = text
        .lines()
        .find_map(|l| l.strip_prefix("FFmpeg commit:").map(str::trim))
        .filter(|s| !s.is_empty());
    let date = text
        .lines()
        .find_map(|l| l.strip_prefix("Build date:").map(str::trim))
        .filter(|s| !s.is_empty());
    match (commit, date) {
        (Some(c), Some(d)) => format!("{c} / {d}"),
        (Some(c), None) => c.to_string(),
        (None, Some(d)) => format!("unknown / {d}"),
        (None, None) => "unknown".into(),
    }
}

fn app_info(app: &tauri::AppHandle) -> AppInfoOut {
    let roots = candidate_roots();
    let ffmpeg_build = doc_by_id("ffmpeg_build_info")
        .and_then(|d| read_doc_in_roots(d, &roots).ok())
        .map(|(text, _)| parse_ffmpeg_build(&text))
        .unwrap_or_else(|| "unknown".into());
    let docs = DOCS
        .iter()
        .map(|d| {
            let bytes = roots
                .iter()
                .find_map(|root| {
                    let p = root.join(d.file);
                    p.is_file()
                        .then(|| std::fs::metadata(&p).ok().map(|m| m.len()))
                        .flatten()
                })
                .unwrap_or(0);
            LegalDocInfo {
                id: d.id.into(),
                title: d.title.into(),
                status: if bytes > 0 { "shipped" } else { "missing" }.into(),
                bytes,
            }
        })
        .collect();
    AppInfoOut {
        version: app.package_info().version.to_string(),
        ffmpeg_build,
        docs,
    }
}

#[tauri::command]
pub fn get_app_info(app: tauri::AppHandle) -> Result<AppInfoOut, String> {
    Ok(app_info(&app))
}

#[tauri::command]
pub fn read_legal_doc(app: tauri::AppHandle, id: String) -> Result<LegalDocOut, String> {
    let _ = app;
    let def = doc_by_id(&id).ok_or_else(|| "invalid legal document".to_string())?;
    // 先做 ID 形态校验，防御未来 allowlist 误加带路径的文件名。
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id == "."
        || id == ".."
        || Path::new(&id).is_absolute()
    {
        return Err("invalid legal document".into());
    }
    let (text, bytes) = read_doc_in_roots(def, &candidate_roots())?;
    Ok(LegalDocOut {
        id: def.id.into(),
        text,
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sr_m19d_legal_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn docs_are_unique_and_fixed() {
        let ids: Vec<_> = DOCS.iter().map(|d| d.id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), DOCS.len(), "文档 id 必须唯一");
        assert_eq!(DOCS.len(), 5);
        for d in DOCS {
            assert!(!d.file.contains('/') && !d.file.contains('\\'));
            assert!(d.file.ends_with(".txt") || d.file.ends_with(".sh"));
        }
    }

    #[test]
    fn unknown_and_path_ids_rejected() {
        assert!(doc_by_id("").is_none());
        assert!(doc_by_id("../FFmpeg-LICENSE.txt").is_none());
        assert!(doc_by_id("ffmpeg_license/..").is_none());
        assert!(doc_by_id("ffmpeg_license").is_some());
    }

    #[test]
    fn read_doc_in_root_and_missing_are_safe() {
        let root = temp_root("read");
        fs::write(root.join("GPLv3.txt"), "GPL text").unwrap();
        let def = doc_by_id("gplv3").unwrap();
        let (text, bytes) = read_doc_in_roots(def, &[root.clone()]).unwrap();
        assert_eq!(text, "GPL text");
        assert_eq!(bytes, 8);
        let missing = doc_by_id("ffmpeg_license").unwrap();
        let err = read_doc_in_roots(missing, &[root.clone()]).unwrap_err();
        assert_eq!(err, "legal document unavailable");
        assert!(!err.contains(&root.display().to_string()));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn oversized_doc_rejected() {
        let root = temp_root("oversize");
        let f = fs::File::create(root.join("GPLv3.txt")).unwrap();
        f.set_len(MAX_DOC_BYTES + 1).unwrap();
        let def = doc_by_id("gplv3").unwrap();
        let err = read_doc_in_roots(def, &[root.clone()]).unwrap_err();
        assert_eq!(err, "legal document too large");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ffmpeg_build_parser_keeps_commit_and_date() {
        let text =
            "Screen_rec custom FFmpeg build\nFFmpeg commit: 46d899d7\nBuild date: 20261019\n";
        assert_eq!(parse_ffmpeg_build(text), "46d899d7 / 20261019");
        assert_eq!(parse_ffmpeg_build("nothing"), "unknown");
    }

    #[test]
    fn version_matches_tauri_config_and_cargo() {
        let conf = include_str!("../tauri.conf.json");
        let json: serde_json::Value = serde_json::from_str(conf).unwrap();
        assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    }
}
