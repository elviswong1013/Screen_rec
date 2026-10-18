//! L1 单元测试：时间轴单位换算（REQ-25 统一 epoch；specs/001-screen-recorder.md
//! §2.6 REQ-25、§3.3 命名管道/时间轴、§3.4 SessionManifest.start_qpc、§4 REQ-25 行）。
//!
//! 运行方式（L1：随默认 `cargo test` 执行；无设备/交互会话依赖）：
//! ```text
//! cargo test -p sr-pipeline --test epoch_mapping -- --nocapture
//! ```
//!
//! 接口契约（测试即契约）：
//! - `sr_pipeline::epoch::{HNS_PER_SEC, HNS_PER_MS}`：100ns 单位常量（REQ-18/25）。
//! - `qpc_ticks_to_hns(ticks, freq) -> Option<i64>` / `hns_to_qpc_ticks(hns, freq) -> Option<i64>`：
//!   QPC ticks ↔ 100ns，**round-half-away-from-zero**、i128 中间量、溢出饱和；`freq <= 0` → `None`
//!   （不猜默认频率）。
//! - `unix_ms_to_hns(ms) -> i64`（饱和）/ `hns_to_unix_ms(hns) -> u64`（向下取整；epoch 前 → 0）。
//! - `Timeline::new(freq, qpc_t0, unix_t0_ms) -> Option<Timeline>`：公共 T0 时间轴；
//!   `qpc_to_rel_hns` / `rel_hns_to_qpc` / `hns_to_rel_hns` / `rel_hns_to_unix_ms` /
//!   `qpc_to_unix_ms` / `qpc_to_rel_secs`。负值（屏障前）合法、跨日由整数运算自然处理。
//!
//! 覆盖维度：单位恒等/典型 QPC 频率/单调性/往返误差/负数对称/非法频率/饱和/向下取整/
//! 跨日时间戳/时间轴 T0 与 epoch 回算/QPC 域与 WASAPI 100ns 域一致性。

use sr_pipeline::epoch::{
    hns_to_qpc_ticks, hns_to_unix_ms, qpc_ticks_to_hns, unix_ms_to_hns, Timeline, HNS_PER_MS,
    HNS_PER_SEC,
};

/// 本机实测级典型 QPC 频率（3.579545 MHz；Windows 常见值之一）。
const QPC_FREQ_TYPICAL: i64 = 3_579_545;

#[test]
fn unit_epoch_unit_mapping() {
    // ── 1) 10MHz 下 ticks 与 100ns 恒等（含 0/1/负值语义在下文单列） ──
    for t in [0i64, 1, 9_999, 123_456_789, HNS_PER_SEC] {
        assert_eq!(
            qpc_ticks_to_hns(t, HNS_PER_SEC),
            Some(t),
            "freq=1e7 时 ticks→hns 应恒等（t={t}）"
        );
        assert_eq!(
            hns_to_qpc_ticks(t, HNS_PER_SEC),
            Some(t),
            "freq=1e7 时 hns→ticks 应恒等（t={t}）"
        );
    }

    // ── 2) 典型 QPC 频率：1 秒精确、逐 tick 单调、往返误差 ≤1 tick ──
    assert_eq!(
        qpc_ticks_to_hns(QPC_FREQ_TYPICAL, QPC_FREQ_TYPICAL),
        Some(HNS_PER_SEC),
        "1 秒对应的 ticks 应精确映射为 1e7 hns"
    );
    assert_eq!(
        hns_to_qpc_ticks(HNS_PER_SEC, QPC_FREQ_TYPICAL),
        Some(QPC_FREQ_TYPICAL),
        "1e7 hns 应精确映射回 1 秒 ticks"
    );
    // 1 tick = 1e7/3579545 ≈ 2.7936 hns → round-half-away-from-zero = 3
    assert_eq!(
        qpc_ticks_to_hns(1, QPC_FREQ_TYPICAL),
        Some(3),
        "1 tick 应四舍五入为 3 hns"
    );
    let mut prev = i64::MIN;
    for t in 0..10_000i64 {
        let h = qpc_ticks_to_hns(t, QPC_FREQ_TYPICAL).expect("合法频率");
        assert!(h >= prev, "ticks→hns 应单调不减（t={t}，{h} < {prev}）");
        prev = h;
    }
    for t in [
        -1_000_000i64,
        -1,
        0,
        1,
        999_999,
        QPC_FREQ_TYPICAL,
        35_795_450,
    ] {
        let h = qpc_ticks_to_hns(t, QPC_FREQ_TYPICAL).expect("合法频率");
        let back = hns_to_qpc_ticks(h, QPC_FREQ_TYPICAL).expect("合法频率");
        assert!(
            (back - t).abs() <= 1,
            "ticks→hns→ticks 往返误差应 ≤1 tick（t={t} 得到 {back}）"
        );
    }

    // ── 3) 负数对称（round-half-away-from-zero；屏障前数据为负） ──
    for t in [1i64, 7, QPC_FREQ_TYPICAL, 123_456_789] {
        assert_eq!(
            qpc_ticks_to_hns(-t, QPC_FREQ_TYPICAL),
            qpc_ticks_to_hns(t, QPC_FREQ_TYPICAL).map(|v| -v),
            "ticks→hns 负数舍入应对称（t={t}）"
        );
        assert_eq!(
            hns_to_qpc_ticks(-t, QPC_FREQ_TYPICAL),
            hns_to_qpc_ticks(t, QPC_FREQ_TYPICAL).map(|v| -v),
            "hns→ticks 负数舍入应对称（t={t}）"
        );
    }

    // ── 4) 非法频率：显式 None（不得静默按默认频率换算） ──
    assert_eq!(qpc_ticks_to_hns(1, 0), None, "freq=0 应显式失败");
    assert_eq!(qpc_ticks_to_hns(1, -5), None, "freq<0 应显式失败");
    assert_eq!(hns_to_qpc_ticks(1, 0), None, "hns→ticks freq=0 应显式失败");
    assert!(Timeline::new(0, 0, 0).is_none(), "Timeline 应拒绝 freq=0");

    // ── 5) 饱和（i128 中间量不得回绕） ──
    assert_eq!(qpc_ticks_to_hns(i64::MAX, 1), Some(i64::MAX), "上溢应饱和");
    assert_eq!(qpc_ticks_to_hns(i64::MIN, 1), Some(i64::MIN), "下溢应饱和");

    // ── 6) UNIX 毫秒 ↔ 100ns：恒等/取整/epoch 之前夹到 0/饱和 ──
    assert_eq!(unix_ms_to_hns(0), 0);
    assert_eq!(unix_ms_to_hns(1), HNS_PER_MS);
    assert_eq!(unix_ms_to_hns(1_700_000_000_000), 17_000_000_000_000_000);
    assert_eq!(hns_to_unix_ms(17_000_000_000_000_000), 1_700_000_000_000);
    assert_eq!(hns_to_unix_ms(0), 0);
    assert_eq!(hns_to_unix_ms(-1), 0, "epoch 之前的 100ns 应夹到 0");
    assert_eq!(hns_to_unix_ms(HNS_PER_MS - 1), 0, "毫秒换算应向下取整");
    assert_eq!(hns_to_unix_ms(HNS_PER_MS), 1);
    assert_eq!(unix_ms_to_hns(u64::MAX), i64::MAX, "毫秒→100ns 应饱和");
    assert_eq!(hns_to_unix_ms(i64::MAX), (i64::MAX / HNS_PER_MS) as u64);

    // ── 7) 跨日：时间戳算术在日界前后可逆 ──
    const DAY_MS: u64 = 86_400_000;
    for ms in [DAY_MS - 1, DAY_MS, DAY_MS + 1, 20_000 * DAY_MS + 12_345] {
        assert_eq!(
            hns_to_unix_ms(unix_ms_to_hns(ms)),
            ms,
            "毫秒→100ns→毫秒往返应恒等（ms={ms}）"
        );
    }

    // ── 8) Timeline：T0 零点 / 1s / 负值 / epoch 回算 / QPC 反算 ──
    let freq = HNS_PER_SEC;
    let tl = Timeline::new(freq, 5_000_000, 1_700_000_000_000).expect("合法 Timeline");
    assert_eq!(tl.freq(), freq);
    assert_eq!(tl.qpc_t0(), 5_000_000);
    assert_eq!(tl.unix_t0_ms(), 1_700_000_000_000);
    assert_eq!(tl.qpc_to_rel_hns(5_000_000), 0, "T0 时刻相对时间应为 0");
    assert_eq!(
        tl.qpc_to_rel_hns(5_000_000 + freq),
        HNS_PER_SEC,
        "T0+1s 应为 1e7 hns"
    );
    assert_eq!(
        tl.qpc_to_rel_hns(5_000_000 - 1),
        -1,
        "屏障前 1 tick（10MHz 下 1 tick = 100ns = 1 hns）应为 -1 hns"
    );
    assert_eq!(tl.qpc_to_rel_secs(5_000_000 + freq), 1.0);
    assert!(
        (tl.qpc_to_rel_secs(5_000_000 - 1) + 1e-7).abs() < 1e-12,
        "负相对秒应为 -1e-7"
    );
    assert_eq!(tl.hns_to_rel_hns(5_000_000), 0, "设备位置 100ns 域同源减法");
    assert_eq!(tl.hns_to_rel_hns(5_500_000), 500_000, "50ms 应为 5e5 hns");
    assert_eq!(tl.rel_hns_to_unix_ms(0), 1_700_000_000_000);
    assert_eq!(tl.rel_hns_to_unix_ms(HNS_PER_SEC), 1_700_000_001_000);
    assert_eq!(
        tl.rel_hns_to_unix_ms(-HNS_PER_MS),
        1_699_999_999_999,
        "T0 前 1ms 应回算为 epoch-1ms"
    );
    assert_eq!(tl.qpc_to_unix_ms(5_000_000 + freq), 1_700_000_001_000);
    assert_eq!(tl.rel_hns_to_qpc(0), 5_000_000);
    assert_eq!(tl.rel_hns_to_qpc(HNS_PER_SEC), 5_000_000 + freq);
    assert_eq!(
        tl.rel_hns_to_qpc(tl.qpc_to_rel_hns(5_123_456)),
        5_123_456,
        "QPC → 相对 → QPC 应可逆"
    );

    // ── 9) 跨日 T0：23:59:59.5 起 1s → 次日 00:00:00.5 ──
    let tl2 = Timeline::new(freq, 0, DAY_MS - 500).expect("合法 Timeline");
    assert_eq!(
        tl2.rel_hns_to_unix_ms(HNS_PER_SEC),
        DAY_MS + 500,
        "跨日回算应进位到次日"
    );

    // ── 10) QPC 域与 WASAPI 100ns 域一致性（WGC ticks 与设备位置同源） ──
    for t in [5_000_000i64, 5_000_001, 5_123_456] {
        let h = qpc_ticks_to_hns(t, freq).expect("合法频率");
        assert_eq!(
            tl.qpc_to_rel_hns(t),
            tl.hns_to_rel_hns(h),
            "同一时刻经 QPC ticks 与 100ns 两条路径应得同一相对时间（t={t}）"
        );
    }
}
