//! L1 回归：墙钟续流的触发条件（REQ-18「流不断」的边界；M13 修复）。
//!
//! 背景（docs/milestones/m12-h-report.md §200–207、m13-report.md）：采集核曾有
//! 「每轮 drain 后无条件 `pad_to_ts(QPC now)`」的行为。loopback 包时间戳天然落后
//! 当前 QPC 一个端点延迟（实测 0.3–0.7ms，调度抖动下可达数十 ms），于是每轮补少量
//! 静音、下一包被等量迟到裁切——时间轴不变，但真实载荷被逐段替换为静音。
//!
//! 契约（本文件钉住）：
//! - 正常端点延迟/抖动（含批量 drain）→ **0 帧补齐、0 帧裁切**；
//! - 真实停滞（距上一真实包 ≥ 阈值）→ 补齐到当前 QPC，且阈值之前不补；
//! - 阈值边界为「≥」（恰达阈值触发）。
//!
//! 层级选择：纯逻辑（`SampleSlotPacer`，无 COM/音频端点/真实时钟），确定性、微秒级，
//! 与既有 `silence_pad_late_packet.rs` 同层，默认 `cargo test`（L1）即执行。
//!
//! 运行：`cargo test -p sr-audio --test stall_pad_policy -- --nocapture`

use sr_audio::slots::{PacketPlan, SampleSlotPacer};

/// 48kHz（REQ-18 管线固定格式）。
const RATE: u32 = 48_000;
/// 1ms = 10_000 × 100ns。
const MS_100NS: i64 = 10_000;
/// 10ms 包 = 100_000 × 100ns（WASAPI 共享模式典型周期）。
const TEN_MS_100NS: i64 = 10 * MS_100NS;
/// 10ms @48k = 480 帧。
const FRAMES_10MS: usize = 480;
/// 48 帧/ms（48k）。
const FRAMES_PER_MS: u64 = 48;

fn plan(pad: u64, trim: usize, start: u64) -> PacketPlan {
    PacketPlan {
        pad_frames: pad,
        trimmed_frames: trim,
        start_sample: start,
    }
}

/// (a) 正常端点延迟 + 抖动 + 批量 drain：不得补齐、不得裁切（0 样本被静音替换）。
///
/// 修复前行为：每轮 `pad_to_ts(now)` 会把 `latency` 补成静音，下一包再裁掉等量
/// 载荷——本用例的 `padded/trimmed == 0` 断言即该缺陷的回归闸门。
#[test]
fn unit_normal_endpoint_delay_never_replaces_payload() {
    let t0 = 5_000_000_000i64; // 任意 QPC 起点（T0 由首包建立）
    let mut pacer = SampleSlotPacer::new(RATE);
    // 端点延迟序列（100ns）：0.2–8ms，覆盖实测 0.3–0.7ms 与调度抖动尾部。
    let latency = [
        2_000, 7_000, 12_000, 5_000, 30_000, 80_000, 15_000, 4_000, 25_000, 6_000,
    ];
    let (mut padded, mut trimmed) = (0u64, 0usize);
    let mut packets = 0u64;
    let mut content = 0i64; // 内容时间（100ns，相对 T0）
    let mut round = 0usize;
    while packets < 600 {
        // 每 7 轮模拟一次「调度抖动后一次 drain 取到 2 个包」的批量送达。
        let batch = if round % 7 == 6 { 2 } else { 1 };
        for _ in 0..batch {
            let pl = pacer.on_packet(t0 + content, FRAMES_10MS, false);
            padded += pl.pad_frames;
            trimmed += pl.trimmed_frames;
            content += TEN_MS_100NS;
            packets += 1;
        }
        // drain 结束时 QPC = 最新包结束 + 端点延迟（恒 < 50ms 停滞阈值）。
        let now = t0 + content + latency[round % latency.len()];
        padded += pacer.pad_to_wall_clock(now);
        round += 1;
    }
    assert_eq!(
        padded, 0,
        "正常端点延迟不得补静音（M12-H 互抵缺陷：补齐会被下一包等量裁掉）"
    );
    assert_eq!(trimmed, 0, "正常端点延迟不得裁切真实载荷");
    assert_eq!(
        pacer.written_frames(),
        600 * FRAMES_10MS as u64,
        "输出帧数 = 真实载荷帧数（无补零占位）"
    );
}

/// (b) 真实停滞（> 阈值）：阈值之前不补；到阈值后补齐到当前 QPC；恢复后严丝合缝。
#[test]
fn unit_real_stall_pads_only_after_threshold() {
    let t0 = 1_000_000_000i64;
    let mut pacer = SampleSlotPacer::new(RATE);
    // 1s 连续包（100 × 10ms）。
    for n in 0..100i64 {
        assert_eq!(
            pacer.on_packet(t0 + n * TEN_MS_100NS, FRAMES_10MS, false),
            plan(0, 0, (n as u64) * FRAMES_10MS as u64),
            "连续包 {n} 应无补齐/无裁切"
        );
    }
    let last_end = t0 + 100 * TEN_MS_100NS; // 内容 1s

    // 停滞 5..45ms：未达 50ms 阈值 → 一律不补（正常抖动区）。
    let mut early = 0u64;
    for step in 1..=9i64 {
        early += pacer.pad_to_wall_clock(last_end + step * 5 * MS_100NS);
    }
    assert_eq!(early, 0, "阈值（50ms）之前不得补静音");

    // 50..300ms：首次达阈值补到 now（含前 50ms），其后每 5ms 递增。
    let mut padded = 0u64;
    for step in 10..=60i64 {
        padded += pacer.pad_to_wall_clock(last_end + step * 5 * MS_100NS);
    }
    assert_eq!(
        padded,
        300 * FRAMES_PER_MS,
        "停滞 300ms 应补 300ms 静音（墙钟对齐）"
    );
    assert_eq!(
        pacer.written_frames(),
        (1000 + 300) * FRAMES_PER_MS,
        "停滞后的水位 = 1s 内容 + 300ms 补齐"
    );

    // 源恢复（内容时钟继续）：起始槽 = 水位，无重复写入（REQ-18 红线）。
    assert_eq!(
        pacer.on_packet(last_end + 300 * MS_100NS, FRAMES_10MS, false),
        plan(0, 0, 62_400),
        "恢复包应从补齐后的水位接续"
    );
    assert_eq!(pacer.written_frames(), 62_400 + FRAMES_10MS as u64);
}

/// (c) 阈值边界：恰达阈值触发（≥），差 1 个 100ns 不触发；阈值可由构造注入。
#[test]
fn unit_stall_threshold_boundary_is_inclusive() {
    let mut pacer = SampleSlotPacer::with_stall_threshold(RATE, 50 * MS_100NS);
    assert_eq!(pacer.on_packet(0, FRAMES_10MS, false), plan(0, 0, 0));
    let last_end = TEN_MS_100NS;
    assert_eq!(
        pacer.pad_to_wall_clock(last_end + 50 * MS_100NS - 1),
        0,
        "差 1 个 100ns 未达阈值：不得补"
    );
    assert_eq!(
        pacer.pad_to_wall_clock(last_end + 50 * MS_100NS),
        50 * FRAMES_PER_MS,
        "恰达阈值：补到 now（50ms）"
    );
}
