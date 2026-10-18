//! L1 单元测试：样本槽静音补齐与迟到包裁切（REQ-18，specs/001-screen-recorder.md
//! §2.4 / §3.1 / §4；docs/spikes/s3/s3-audio-isolation.md §3.3 静音行为实测依据）。
//!
//! 运行方式（L1：随默认 `cargo test` 执行）：
//! ```text
//! cargo test -p sr-audio --test silence_pad_late_packet -- --nocapture
//! ```
//!
//! 接口契约（测试即契约，详见 docs/milestones/m4-red-report.md §3）：
//! - `sr_audio::slots::SampleSlotPacer::new(sample_rate: u32) -> Self`
//!   （48_000 = 管线固定速率，REQ-18；帧数按「帧」计，交错立体声展开由写入方执行）
//! - `SampleSlotPacer::on_packet(&mut self, ts_100ns: i64, frames: usize, silent: bool)
//!   -> PacketPlan`（WASAPI 设备时间戳为 **100ns 单位**——单位换算显式实现并单测，
//!   REQ-25；`silent` = AUDCLNT_BUFFERFLAGS_SILENT：真实静音仍占时间轴，写入内容
//!   为零由写入方执行，本层不产生额外补齐/裁切差异——区分真实静音/断流）
//! - `PacketPlan { pad_frames: u64, trimmed_frames: usize, start_sample: u64 }`
//!   （derive(Debug, PartialEq)，assert_eq 可用）
//! - `SampleSlotPacer::written_frames(&self) -> u64`（已编排输出总帧数，含补齐）
//!
//! 语义钉死（REQ-18 在 M4 的可测部分）：
//! - 首包建立 T0（样本槽 0；绝对时间戳任意，槽位相对 T0）。
//! - 起始槽 = floor((ts−T0)·rate/10⁷)：100ns→样本显式换算；10ms@48k 恰为 480 帧。
//! - 时间轴空洞（起始槽 > 已写水位）→ 先补 `pad_frames` 帧零样本再写（静音补齐
//!   按样本槽；S3 §3.3：补帧成本极低且把时长钉到请求值）。
//! - 迟到包（起始槽 < 已写水位，含落在补零区）→ 裁切与已写区间重叠的前缀
//!   `trimmed_frames` 帧，余量在水位处**接续**写入；`trimmed_frames == frames`
//!   即整包丢弃——**禁止同一时间段写两次**（REQ-18 红线）。
//! - 整包丢弃不改变已写水位。

use sr_audio::slots::{PacketPlan, SampleSlotPacer};

/// 48kHz（REQ-18 管线固定格式：48k/f32/立体声）。
const RATE: u32 = 48_000;
/// 10ms = 100_000 × 100ns（WASAPI 事件驱动共享模式的典型包节奏，S3 §3.3）。
const TEN_MS_100NS: i64 = 100_000;
/// 10ms @48k = 480 帧（恰好整除：100_000×48_000/10⁷ = 480）。
const FRAMES_10MS: usize = 480;

fn plan(pad: u64, trim: usize, start: u64) -> PacketPlan {
    PacketPlan {
        pad_frames: pad,
        trimmed_frames: trim,
        start_sample: start,
    }
}

/// REQ-18：静音补齐按样本槽 + 迟到包裁切/丢弃（禁止同一时间段写两次）。
#[test]
fn unit_silence_pad_late_packet_drop() {
    // T0 取任意非零绝对时间戳（QPC/设备位置不以 0 起步；槽位须相对 T0）
    let t0: i64 = 73_500_000;

    // ── 1. 定速 10ms 包流：连续槽位顺次写入，无补齐无裁切 ──
    let mut p = SampleSlotPacer::new(RATE);
    for n in 0..10i64 {
        let pl = p.on_packet(t0 + n * TEN_MS_100NS, FRAMES_10MS, false);
        assert_eq!(
            pl,
            plan(0, 0, (n * FRAMES_10MS as i64) as u64),
            "REQ-25/18：定速流第 {n} 包应无补齐/裁切地写入槽 {}",
            n * FRAMES_10MS as i64
        );
    }
    assert_eq!(
        p.written_frames(),
        10 * FRAMES_10MS as u64,
        "定速流水位 = Σ包帧数"
    );

    // ── 2. 100ns→样本换算显式且精确（48k 下 10ms 恰 480 帧）──
    let mut p0 = SampleSlotPacer::new(RATE);
    assert_eq!(
        p0.on_packet(0, FRAMES_10MS, false),
        plan(0, 0, 0),
        "T0=0 首包 → 槽 0"
    );
    assert_eq!(
        p0.on_packet(TEN_MS_100NS, FRAMES_10MS, false),
        plan(0, 0, 480),
        "10ms → 480 帧"
    );

    // ── 3. 断流空洞 → 静音补齐把时间轴接续（pad 后接写，水位连续）──
    let mut p2 = SampleSlotPacer::new(RATE);
    assert_eq!(p2.on_packet(t0, FRAMES_10MS, false), plan(0, 0, 0));
    // 下一包在 30ms 处（空洞 20ms）：起始槽 1440，已写 480 ⇒ 先补 960 帧零
    assert_eq!(
        p2.on_packet(t0 + 3 * TEN_MS_100NS, FRAMES_10MS, false),
        plan(960, 0, 1440),
        "REQ-18：断流区间应按样本槽补零（pad 960 帧 = 20ms）"
    );
    assert_eq!(p2.written_frames(), 1920, "补齐计入水位");

    // ── 4. 补零区间后到达的迟到包（整包落在已写区间）→ 丢弃，水位不回退 ──
    assert_eq!(
        p2.on_packet(t0 + 2 * TEN_MS_100NS, FRAMES_10MS, false),
        plan(0, FRAMES_10MS, 1920),
        "REQ-18：覆盖 [960,1440)（补零区）的迟到包应整包裁切丢弃"
    );
    assert_eq!(
        p2.written_frames(),
        1920,
        "整包丢弃不得改变已写帧数（禁止二次写入）"
    );

    // ── 5. 迟到部分重叠 → 裁去重叠前缀，余量在水位处接续写入 ──
    // ts = 35ms → 起始槽 1680 < 水位 1920 ⇒ 裁 240 帧，余 240 帧写在 1920
    assert_eq!(
        p2.on_packet(t0 + 3 * TEN_MS_100NS + TEN_MS_100NS / 2, FRAMES_10MS, false),
        plan(0, 240, 1920),
        "REQ-18：部分迟到包应裁切重叠前缀后接续写入（不重复写、不留洞）"
    );
    assert_eq!(p2.written_frames(), 2160);

    // ── 6. SILENT 旗标（真实静音）：同样推进时间轴（内容零由写入方执行）──
    // ts = 45ms（= 450_000 × 100ns，45ms 不是 45×10ms）→ 起始槽 2160 == 水位
    // ⇒ 无补齐无裁切，正常占位
    assert_eq!(
        p2.on_packet(t0 + 450_000, FRAMES_10MS, true),
        plan(0, 0, 2160),
        "REQ-18：SILENT 包仍占时间轴（真实静音 ≠ 断流；不触发额外补齐）"
    );
    assert_eq!(p2.written_frames(), 2160 + FRAMES_10MS as u64);

    // ── 7. 迟到判定对「到达顺序」不敏感：早包迟到同样丢弃 ──
    let mut p3 = SampleSlotPacer::new(RATE);
    assert_eq!(
        p3.on_packet(t0 + 2 * TEN_MS_100NS, FRAMES_10MS, false),
        plan(0, 0, 0),
        "首包即 T0（槽 0）"
    );
    assert_eq!(
        p3.on_packet(t0 + 3 * TEN_MS_100NS, FRAMES_10MS, false),
        plan(0, 0, 480)
    );
    assert_eq!(
        p3.on_packet(t0 + TEN_MS_100NS, FRAMES_10MS, false),
        plan(0, FRAMES_10MS, 960),
        "覆盖 [480,960) 的迟到包应整包丢弃（水位 960 不变）"
    );
    assert_eq!(p3.written_frames(), 960);
}
