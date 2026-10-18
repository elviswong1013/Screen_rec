//! 输入前置校验（REQ-23 §3.5：合并输入不可读/不被 MKV 接纳 → 拒绝并说明
//! （E_MERGE_INCOMPATIBLE）；**异构（分辨率/帧率/时长/轨数不同）一律放行**，不得误拒）。
//!
//! 校验口径（轻量预检，避免把不可接纳输入送进 sidecar 后误报为执行失败）：
//! 存在且为常规文件、可读、非空、容器可接纳——Matroska（EBML 存续标记 0x1A45DFA3）
//! 或 ISO BMFF（MP4 族：offset 4 处 `ftyp` box；M17/REQ-18 支持本产品 MP4 参与合并）。
//! 合并产物仍固定为 MKV（多视频轨）。

use std::io::Read;
use std::path::Path;

/// Matroska/EBML 文件头存续标记（前 4 字节）。
const EBML_MAGIC: [u8; 4] = [0x1A, 0x45, 0xDF, 0xA3];
/// ISO BMFF `ftyp` box 类型（bytes 4..8；MP4/M4A/MOV 等容器族）。
const FTYP_BOX: [u8; 4] = [b'f', b't', b'y', b'p'];

/// 前置校验单条输入；`Err(detail)` = 拒绝原因（E_MERGE_INCOMPATIBLE 语义）。
pub fn check_input(path: &Path) -> Result<(), String> {
    let meta = std::fs::metadata(path)
        .map_err(|e| format!("输入不存在/不可读（{}）：{e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("输入不是常规文件（{}）", path.display()));
    }
    if meta.len() == 0 {
        return Err(format!("输入为空文件（{}）", path.display()));
    }
    let mut f = std::fs::File::open(path)
        .map_err(|e| format!("输入不可打开（{}）：{e}", path.display()))?;
    let mut probe = [0u8; 12];
    f.read_exact(&mut probe)
        .map_err(|e| format!("输入读取失败（{}）：{e}", path.display()))?;
    let is_matroska = probe[0..4] == EBML_MAGIC;
    let is_iso_bmff = probe[4..8] == FTYP_BOX;
    if !is_matroska && !is_iso_bmff {
        return Err(format!(
            "输入未被 MKV/MP4 接纳（EBML/ftyp 标记缺失，前 12 字节 {probe:02X?}；{}）",
            path.display()
        ));
    }
    Ok(())
}
