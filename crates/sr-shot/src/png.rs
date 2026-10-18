//! PNG 编码与输出落盘（内部）：png crate（RGBA8）。
//! 命名最小实现——REQ-24 全量规则（时间戳格式/sanitize/目录提交）属 M8；此处保证
//! 契约面：输出位于目录内、扩展名 png、多次输出路径互不相同、create_new 禁止覆盖。
//! PNG 实际编码尺寸（IHDR）= 传入元数据宽高（元数据不是空头支票）。

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// 写出失败（路径 + 人类可读原因）。
pub(crate) struct WriteError {
    pub(crate) path: PathBuf,
    pub(crate) detail: String,
}

/// 进程内单调序号：同毫秒多次输出仍互不相同。
static SEQ: AtomicU32 = AtomicU32::new(0);

/// 将 RGBA 帧编码为 PNG 写入 `dir` 内的唯一新文件，返回其路径。
pub(crate) fn write_png_unique(
    dir: &Path,
    kind: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<PathBuf, WriteError> {
    let path = unique_path(dir, kind);
    // create_new：已存在即失败（禁覆盖；SEQ+毫秒使碰撞概率≈0，碰撞时显式报错而非覆盖）
    let file = File::create_new(&path).map_err(|e| WriteError {
        path: path.clone(),
        detail: format!("创建输出文件失败: {e}"),
    })?;
    encode_rgba(file, width, height, rgba).map_err(|detail| WriteError {
        path: path.clone(),
        detail,
    })?;
    Ok(path)
}

fn unique_path(dir: &Path, kind: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    dir.join(format!("shot_{kind}_{millis}_{seq:04}.png"))
}

fn encode_rgba(file: File, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("写 PNG 头失败: {e}"))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| format!("写 PNG 像素数据失败: {e}"))?;
    writer
        .finish()
        .map_err(|e| format!("收尾 PNG 输出失败: {e}"))?;
    Ok(())
}
