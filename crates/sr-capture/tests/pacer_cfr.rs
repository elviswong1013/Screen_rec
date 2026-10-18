//! L1 单元测试：帧槽节奏器（REQ-10，specs/001-screen-recorder.md §2.3 / §3.1 / §4）。
//!
//! 运行方式（L1：随默认 `cargo test` 执行）：
//! ```text
//! cargo test -p sr-capture --test pacer_cfr -- --nocapture
//! ```
//!
//! 接口契约（测试即契约，详见 docs/milestones/m3-red-report.md）：
//! - `sr_capture::pacer::FramePacer::new(fps: u32, qpc_ticks_per_sec: i64) -> Self`
//!   （QPC 频率显式注入——WGC SystemRelativeTime 为 QPC ticks，单位换算必须显式实现
//!   且可确定性单测，REQ-25）
//! - `FramePacer::on_frame(&mut self, source_qpc_ticks: i64) -> FrameAction`
//! - `FrameAction::Write { slot: u64 } | Drop`（derive(Debug, PartialEq)）：
//!   `slot` = CFR 输出槽位 n（相对 T0，n/fps）；首帧建立 T0。
//!
//! 语义钉死（REQ-10 在 M3 的可测部分）：
//! - 槽位是**源时间戳的纯函数**（到达顺序不作时间依据）；同槽第二帧 → Drop（重复）；
//!   已写槽的更早时间戳帧 → Drop（迟到）。
//! - 未写过的槽即使乱序到达也须 Write（frame pool 批量乱序送达不造成错槽/误丢）。
//! - 补帧（间隙槽重复上一帧）与 wall-clock 超前判定属 REQ-25/26 的
//!   `unit_epoch_unit_mapping`/`unit_drift_hysteresis_limits` 范围，本测试不预设其形状。

use sr_capture::pacer::{FrameAction, FramePacer};

/// 30fps + QPC 频率 30,000,000 ticks/s ⇒ 1 槽 = 恰好 1,000,000 ticks（消除除法舍入噪声）。
const FPS: u32 = 30;
const QPC_PER_SEC: i64 = 30_000_000;
const SLOT: i64 = QPC_PER_SEC / FPS as i64; // 1_000_000

fn write(slot: u64) -> FrameAction {
    FrameAction::Write { slot }
}

/// REQ-10：30fps 定速——完美节拍的源时间戳流映射到连续 CFR 槽 0,1,2,…（无跳槽/重复），
/// 全部 Write；时间戳单调 ⇒ 槽位单调。
#[test]
fn unit_pacer_cfr_dup_drop() {
    let mut p = FramePacer::new(FPS, QPC_PER_SEC);

    // ── 1. 定速 30fps：完美节拍 → 槽 0..N 逐帧顺次 Write ──
    for n in 0..10i64 {
        let t = n * SLOT;
        assert_eq!(
            p.on_frame(t),
            write(n as u64),
            "定速流第 {n} 帧应映射到槽 {n}"
        );
    }

    // ── 2. 带抖动的定速（帧内偏移 <1 槽）：仍每帧一槽、顺次不乱 ──
    let mut p2 = FramePacer::new(FPS, QPC_PER_SEC);
    for n in 0..10i64 {
        let t = n * SLOT + SLOT * 3 / 10; // 槽内 +0.3 偏移
        assert_eq!(
            p2.on_frame(t),
            write(n as u64),
            "抖动定速第 {n} 帧应仍在槽 {n}"
        );
    }

    // ── 3. 重复决策：同槽第二帧 Drop（每槽至多一帧）；后续帧不受污染 ──
    let mut p3 = FramePacer::new(FPS, QPC_PER_SEC);
    assert_eq!(
        p3.on_frame(2 * SLOT),
        write(2),
        "首帧建立 T0（本组从槽 2 起算）"
    );
    assert_eq!(
        p3.on_frame(2 * SLOT + SLOT / 5),
        FrameAction::Drop,
        "同槽（slot=2）第二帧应 Drop（重复）"
    );
    assert_eq!(
        p3.on_frame(2 * SLOT - SLOT / 5),
        FrameAction::Drop,
        "已写槽的更近时间戳帧仍应 Drop（该槽已占用）"
    );
    assert_eq!(p3.on_frame(3 * SLOT), write(3), "Drop 之后后续槽正常 Write");

    // ── 4. 迟到：映射到已写槽的更早时间戳帧 → Drop ──
    let mut p4 = FramePacer::new(FPS, QPC_PER_SEC);
    assert_eq!(p4.on_frame(0), write(0));
    assert_eq!(p4.on_frame(SLOT), write(1));
    assert_eq!(
        p4.on_frame(SLOT + SLOT / 2),
        FrameAction::Drop,
        "slot=1 已写：迟到帧 Drop"
    );
    assert_eq!(
        p4.on_frame(SLOT / 2),
        FrameAction::Drop,
        "slot=0 已写：更早帧 Drop"
    );

    // ── 5. 到达顺序不作时间依据（frame pool 批量乱序送达）：
    //      槽指派只看时间戳；乱序到达的未写槽必须 Write，不得错槽或误丢 ──
    let mut p5 = FramePacer::new(FPS, QPC_PER_SEC);
    assert_eq!(p5.on_frame(0), write(0));
    assert_eq!(
        p5.on_frame(2 * SLOT),
        write(2),
        "批量中后帧先到：槽 2 未写 → 应 Write 到槽 2"
    );
    assert_eq!(
        p5.on_frame(SLOT),
        write(1),
        "批量中前帧后到（乱序）：槽 1 未写 → 必须仍 Write 到槽 1（到达顺序不作时间依据，不产生 judder）"
    );
    assert_eq!(
        p5.on_frame(2 * SLOT + 1),
        FrameAction::Drop,
        "槽 2 已写：重复帧 Drop"
    );

    // ── 6. 跨槽大跳（源间隔 >1 槽）：新槽帧不误丢（间隙补帧属 REQ-25/26，不在本测试形状内）──
    let mut p6 = FramePacer::new(FPS, QPC_PER_SEC);
    assert_eq!(p6.on_frame(0), write(0));
    assert_eq!(
        p6.on_frame(5 * SLOT),
        write(5),
        "跳到槽 5 的帧应 Write 到槽 5（补齐 slot1..4 是后续里程碑的补帧契约，不得借此丢帧）"
    );
    assert_eq!(p6.on_frame(6 * SLOT), write(6), "跳槽后继续顺次");
}
