//! L1 单元测试：漂移滞回带与速率限幅（REQ-26 慢漂移的参数化决策核，
//! specs/001-screen-recorder.md §2.6 / §4）。
//!
//! 运行方式（L1：随默认 `cargo test` 执行）：
//! ```text
//! cargo test -p sr-audio --test drift_hysteresis -- --nocapture
//! ```
//!
//! 接口契约（测试即契约，详见 docs/milestones/m4-red-report.md §3）：
//! - `sr_audio::resample::DriftCfg { enter_ms: f64, exit_ms: f64, rate_limit_ppm: f64 }`
//!   （**全部参数化记账**，REQ-26；测试用 40/20/500——阶跃阈值起点 40ms 与
//!   速率限幅实验起点 ±500ppm 的锚点值，S6 定值前可调）
//! - `sr_audio::resample::DriftController::new(cfg) -> Self`
//! - `DriftController::on_measurement(&mut self, content_offset_ms: f64) -> DriftAction`
//!   （输入：内容偏差测量——已写样本数/48000 vs 基准（QPC/已写帧数）的偏移；
//!   **正 = 音频内容落后基准**（须加速/丢），负 = 超前（须减速/补））
//! - `DriftAction::Hold | Correct { rate_ppm: f64 }`（derive(Debug, PartialEq)；
//!   `rate_ppm` 符号与偏差同号，|rate_ppm| ≤ rate_limit_ppm）
//!
//! 语义钉死（REQ-26 在 M4 的可测部分；与阶跃校正分开记账）：
//! - 滞回带：|offset| < enter ⇒ Hold；越 enter 进入校正；校正态回到 (exit, enter)
//!   区间**维持**校正；|offset| ≤ exit 才退出；退出后不得自行再进入。
//! - 速率限幅：校正速率恒 ≤ ±rate_limit_ppm（实验起点 ±500ppm），巨大偏差饱和于限幅。
//! - 校正非零：处于校正态时 rate_ppm ≠ 0（符号与偏差一致）。
//! - 增益（offset→rate 的比例系数）**不钉死**（测量窗口/滤波/恢复条件属实现记账，
//!   S6 定值；测试只钉方向、限幅、滞回形状）。

use sr_audio::resample::{DriftAction, DriftCfg, DriftController};

fn cfg_default() -> DriftCfg {
    DriftCfg {
        enter_ms: 40.0,
        exit_ms: 20.0,
        rate_limit_ppm: 500.0,
    }
}

/// REQ-26：滞回带 + 速率限幅（±500ppm 实验起点）+ 参数化。
#[test]
fn unit_drift_hysteresis_limits() {
    // ── 1. 带内（|offset| < enter，含边界内侧）⇒ Hold ──
    let mut d = DriftController::new(cfg_default());
    for off in [0.0, 5.0, -15.0, 39.9, -39.9] {
        assert_eq!(
            d.on_measurement(off),
            DriftAction::Hold,
            "REQ-26：|{off}ms| < 进入阈值 40ms 应 Hold（带内不校正）"
        );
    }

    // ── 2. 越带进入校正：方向与偏差同号、非零、不超限幅 ──
    for (off, sign) in [(40.0f64, 1.0f64), (120.0, 1.0), (-60.0, -1.0)] {
        match d.on_measurement(off) {
            DriftAction::Correct { rate_ppm } => {
                assert!(
                    rate_ppm * sign > 0.0,
                    "REQ-26：offset={off}ms 的校正方向应与偏差同号（得到 {rate_ppm}ppm）"
                );
                assert!(
                    rate_ppm.abs() <= 500.0 + 1e-9,
                    "REQ-26：校正速率不得超限幅 ±500ppm（得到 {rate_ppm}）"
                );
            }
            DriftAction::Hold => panic!("REQ-26：offset={off}ms 已越进入阈值 40ms，应进入校正"),
        }
    }

    // ── 3. 巨大偏差饱和于限幅（恢复需求再大也恒 ≤ ±500ppm）──
    match d.on_measurement(10_000.0) {
        DriftAction::Correct { rate_ppm } => assert!(
            (rate_ppm - 500.0).abs() <= 1e-6,
            "REQ-26：+10s 偏差应饱和在 +500ppm（得到 {rate_ppm}）"
        ),
        DriftAction::Hold => panic!("REQ-26：+10s 偏差应处于校正态"),
    }
    match d.on_measurement(-10_000.0) {
        DriftAction::Correct { rate_ppm } => assert!(
            (rate_ppm + 500.0).abs() <= 1e-6,
            "REQ-26：−10s 偏差应饱和在 −500ppm（得到 {rate_ppm}）"
        ),
        DriftAction::Hold => panic!("REQ-26：−10s 偏差应处于校正态"),
    }

    // ── 4. 滞回：校正态在 (exit, enter) 区间维持；≤ exit 才退出；退出后不自启 ──
    let mut d2 = DriftController::new(cfg_default());
    assert_eq!(
        d2.on_measurement(30.0),
        DriftAction::Hold,
        "30ms 未越进入阈值（滞回带内静默）"
    );
    match d2.on_measurement(50.0) {
        DriftAction::Correct { .. } => {}
        DriftAction::Hold => panic!("50ms 应进入校正"),
    }
    match d2.on_measurement(30.0) {
        DriftAction::Correct { rate_ppm } => assert!(
            rate_ppm > 0.0 && rate_ppm <= 500.0 + 1e-9,
            "滞回：校正态回到 30ms（> 退出阈值 20ms）应维持校正"
        ),
        DriftAction::Hold => panic!("滞回：校正态回到 30ms 应维持校正（须 ≤20ms 才退出）"),
    }
    assert_eq!(
        d2.on_measurement(20.0),
        DriftAction::Hold,
        "REQ-26：|offset| 回落到退出阈值 20ms 应退出校正（滞回带）"
    );
    assert_eq!(
        d2.on_measurement(30.0),
        DriftAction::Hold,
        "退出后 30ms（< 进入阈值）不得自行再进入校正"
    );

    // ── 5. 参数化：行为随 cfg 收缩（阈值与限幅都是注入参数，非硬编码）──
    let mut d3 = DriftController::new(DriftCfg {
        enter_ms: 10.0,
        exit_ms: 5.0,
        rate_limit_ppm: 100.0,
    });
    assert_eq!(
        d3.on_measurement(9.9),
        DriftAction::Hold,
        "enter=10：9.9ms 应 Hold"
    );
    match d3.on_measurement(12.0) {
        DriftAction::Correct { rate_ppm } => assert!(
            rate_ppm > 0.0 && rate_ppm <= 100.0 + 1e-9,
            "限幅随配置收缩到 ±100ppm（得到 {rate_ppm}）"
        ),
        DriftAction::Hold => panic!("enter=10：12ms 应进入校正"),
    }
}
