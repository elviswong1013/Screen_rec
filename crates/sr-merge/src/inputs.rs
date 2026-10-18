//! 输入前置校验（REQ-23 §3.5：合并输入不可读/不被 MKV 接纳 → 拒绝并说明
//! （E_MERGE_INCOMPATIBLE）；**异构（分辨率/帧率/时长/轨数不同）一律放行**，不得误拒）。
//!
//! 校验口径（轻量预检，避免把不可接纳输入送进 sidecar 后误报为执行失败）：
//! 存在且为常规文件、可读、非空、Matroska（EBML 存续标记 0x1A45DFA3）。

use std::io::Read;
use std::path::Path;

/// Matroska/EBML 文件头存续标记（前 4 字节）。
const EBML_MAGIC: [u8; 4] = [0x1A, 0x45, 0xDF, 0xA3];

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
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic)
        .map_err(|e| format!("输入读取失败（{}）：{e}", path.display()))?;
    if magic != EBML_MAGIC {
        return Err(format!(
            "输入未被 Matroska 接纳（EBML 存续标记缺失，前 4 字节 {magic:02X?}；{}）",
            path.display()
        ));
    }
    Ok(())
}
