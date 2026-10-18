//! sr-merge：合并（异构多轨）。
//!
//! 职责（specs/001-screen-recorder.md §3.2）：REQ-23。
//! M7 实现：[`inputs`]（输入前置校验：不可读/非 Matroska 拒绝；异构一律放行）、
//! [`merge`]（`-c copy` 流拷贝合并：显式逐输入全轨映射/多视频轨并存/不覆盖输入/
//! 不用 `-shortest`/失败与取消不提交）。

/// 输入校验（REQ-23：仅拒绝不可读/未被 MKV 接纳的输入（E_MERGE_INCOMPATIBLE）；
/// 允许异构（分辨率/帧率/轨数不同），不得误拒）。
pub mod inputs;

/// 合并执行（REQ-23：`-map 0 -map 1 … -c copy` 全轨保留（多视频轨并存）；
/// 不覆盖输入；不用 `-shortest`；各文件按本地零点合并；
/// 取消/失败保留输入并清理临时物）。
pub mod merge;
