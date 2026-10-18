//! 帧槽节奏器（REQ-10/25）：WGC 源时间戳 → CFR 输出槽决策。
//!
//! 契约（测试即契约，docs/milestones/m3-red-report.md §3.1 + pacer_cfr.rs）：
//! - 槽位是**源时间戳的纯函数**：`slot = floor(source_ticks · fps / qpc_ticks_per_sec)`
//!   （源时间域绝对槽；WGC SystemRelativeTime 以固定系统零点起算）；
//! - **首帧建立 T0**：更早（`t < T0`）的帧视为迟到/无效 → Drop；
//! - 每槽至多一帧：该槽已写过 → Drop（重复/迟到，到达顺序不作时间依据）；
//! - 乱序批量送达：未写过的槽（即使槽号低于已写最大槽）必须 Write，不得错槽/误丢
//!   （frame pool 批量送达不造成 judder）；
//! - 跨槽大跳不误丢（间隙补帧属 REQ-25/26 后续契约，本模块不补帧）。

use std::collections::HashSet;

/// 单帧的槽决策。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameAction {
    /// 写入该 CFR 输出槽（n 相对源时间零点，1 槽 = 1/fps 秒）。
    Write { slot: u64 },
    /// 重复/迟到/无效帧：丢弃。
    Drop,
}

/// 帧槽节奏器（每路一个；非线程安全，须在帧回调线程内使用）。
pub struct FramePacer {
    fps: u32,
    qpc_ticks_per_sec: i64,
    /// 首帧源时间戳（T0；更早的帧 Drop）。
    t0: Option<i64>,
    /// 已写槽集合（乱序写允许低于高水位的未写槽）。
    written: HashSet<u64>,
}

impl FramePacer {
    /// `fps` 与 `qpc_ticks_per_sec` 显式注入（单位换算可确定性单测，REQ-25 钩子）。
    pub fn new(fps: u32, qpc_ticks_per_sec: i64) -> Self {
        assert!(fps > 0, "fps 必须为正");
        assert!(qpc_ticks_per_sec > 0, "QPC 频率必须为正");
        Self {
            fps,
            qpc_ticks_per_sec,
            t0: None,
            written: HashSet::new(),
        }
    }

    /// 源时间戳（WGC SystemRelativeTime，QPC 域 ticks）→ 槽决策。
    pub fn on_frame(&mut self, source_qpc_ticks: i64) -> FrameAction {
        if self.t0.is_none() {
            self.t0 = Some(source_qpc_ticks);
        }
        let t0 = self.t0.expect("T0 已建立");
        if source_qpc_ticks < t0 {
            // T0 之前的帧：迟到/无效（不建立槽位，不得污染已写集合）。
            return FrameAction::Drop;
        }
        // 槽位 = 源时间戳纯函数（i128 中间量防乘法溢出；t ≥ T0 ≥ 0 保证非负）。
        let slot = ((source_qpc_ticks as i128) * (self.fps as i128)
            / (self.qpc_ticks_per_sec as i128)) as u64;
        // 每槽至多一帧：已写过 → Drop；未写过（含乱序补入的空洞槽）→ Write。
        if !self.written.insert(slot) {
            return FrameAction::Drop;
        }
        FrameAction::Write { slot }
    }
}
