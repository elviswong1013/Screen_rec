//! L1 单元测试：Channel 订阅语义（REQ-31，specs/001-screen-recorder.md §2.8 / §3.3）。
//!
//! 运行方式（L1：随默认 `cargo test` 执行）：
//! ```text
//! cargo test -p sr-core --test channel_snapshot_rebind -- --nocapture
//! ```
//!
//! 设计说明：不依赖 Tauri 运行时（无需 tauri::test）——REQ-31 的四个可测行为
//! （订阅即快照 / 重绑后旧发送失败容忍 / 告警与电平分通道 / 电平 ≤10Hz 节流）
//! 均为纯逻辑，落在 sr-core 的 events 模块；src-tauri 层后续只做绑定 Tauri Channel
//! 的薄包装。
//!
//! 接口契约（测试即契约，规格未定型处为「测试假定」，见 docs/milestones/m1-red-report.md）：
//! - `sr_core::events::ChannelHub<S, A, L>`：泛型于 状态快照 S / 告警·状态事件 A / 电平事件 L。
//!   - `ChannelHub::new(initial: S) -> Self`
//!   - `update_state(&self, S)`：推进当前状态（供后续订阅快照）
//!   - `subscribe(&self) -> Subscription<S, A, L>`，其中
//!     `Subscription<S, A, L> { pub snapshot: S, pub alerts: Receiver<A>, pub levels: Receiver<L> }`
//!     （假定接收端为同步通道，具备 `recv_timeout(Duration)`/`try_recv`——std mpsc 与
//!     crossbeam channel 均满足）
//!   - `publish_alert(&self, A)` / `publish_level(&self, L)`：发送失败（旧接收端已亡）
//!     在内部被容忍，API 层不可失败（返回 ()，测试以类型断言钉死）
//! - `sr_core::events::LevelThrottle`：电平节流 ≤10Hz 逻辑（显式时间参数，确定性可测）
//!   - `LevelThrottle::new(Duration) -> Self`（最小发射间隔，10Hz ⇒ 100ms）
//!   - `should_emit(&mut self, Instant) -> bool`

use std::time::{Duration, Instant};

use sr_core::events::{ChannelHub, LevelThrottle};

// 测试替身（避免耦合尚不确定的载荷枚举：RouteState/ErrorCode 等由后续里程碑钉死）
#[derive(Clone, PartialEq, Debug)]
struct FakeState {
    revision: u32,
}
#[derive(Clone, PartialEq, Debug)]
struct FakeAlert {
    code: u32,
}
#[derive(Clone, PartialEq, Debug)]
struct FakeLevel {
    rms: f32,
}

#[test]
fn unit_channel_snapshot_rebind() {
    // ── 1. subscribe 返回「当前」状态快照（REQ-31：订阅入口传入 Channel + 当前状态快照）──
    let hub = ChannelHub::<FakeState, FakeAlert, FakeLevel>::new(FakeState { revision: 0 });
    let sub1 = hub.subscribe();
    assert_eq!(
        sub1.snapshot,
        FakeState { revision: 0 },
        "首次订阅应拿到初始状态快照"
    );

    hub.update_state(FakeState { revision: 7 }); // 「录制进行中」状态推进
    let sub2 = hub.subscribe(); // UI 重载 → 重新订阅（重绑）
    assert_eq!(
        sub2.snapshot,
        FakeState { revision: 7 },
        "重绑订阅必须取得最新状态快照（而非初始状态）——UI 重载后能恢复当前录制视图"
    );

    // ── 2. 旧订阅发送失败不得中断录制（REQ-31 失败容忍语义）──
    // 模拟 UI 重载：旧订阅的两个接收端全部失效。
    drop(sub1.alerts);
    drop(sub1.levels);
    // 向已失效订阅发布：不得 panic、不得以错误外溢中断录制。
    // （以类型断言钉死 publish* 返回 ()：发送失败只允许在内部被吞掉。）
    let (): () = hub.publish_alert(FakeAlert { code: 1 });
    let (): () = hub.publish_level(FakeLevel { rms: 0.5 });
    // 「不中断」的可观察证据：新订阅继续收到后续事件。
    hub.publish_alert(FakeAlert { code: 2 });
    assert_eq!(
        sub2.alerts.recv_timeout(Duration::from_secs(1)),
        Ok(FakeAlert { code: 2 }),
        "旧订阅失效后，新订阅必须继续收到事件（录制不中断）"
    );

    // ── 3. 告警与电平分通道（REQ-31/REQ-27：告警不受电平事件流饿死）──
    while sub2.levels.try_recv().is_ok() {} // 清空电平积压
    for i in 0..200 {
        hub.publish_level(FakeLevel {
            rms: i as f32 / 200.0,
        }); // 电平洪泛
    }
    hub.publish_alert(FakeAlert { code: 3 }); // 洪泛期间发布告警
    assert_eq!(
        sub2.alerts.recv_timeout(Duration::from_secs(1)),
        Ok(FakeAlert { code: 3 }),
        "200 条电平洪泛后告警必须照常送达（独立通道/优先级）"
    );

    // ── 4a. 电平节流 ≤10Hz：确定性逻辑（LevelThrottle，显式时间参数）──
    let mut throttle = LevelThrottle::new(Duration::from_millis(100)); // 10 Hz
    let t0 = Instant::now();
    assert!(
        throttle.should_emit(t0),
        "首条电平必须立即放行（用户可感知延迟最小化）"
    );
    assert!(
        !throttle.should_emit(t0 + Duration::from_millis(50)),
        "距上次发射 <100ms 的电平必须被节流（≤10Hz）"
    );
    assert!(
        throttle.should_emit(t0 + Duration::from_millis(100)),
        "距上次发射 ≥100ms 的电平应放行（≤10Hz）"
    );

    // ── 4b. 通道级行为：瞬时 50 条电平，~250ms 窗口内送达数受 ≤10Hz 约束 ──
    while sub2.levels.try_recv().is_ok() {} // 清空 3 中的电平积压
    for i in 0..50 {
        hub.publish_level(FakeLevel {
            rms: i as f32 / 50.0,
        });
    }
    let mut received = 0usize;
    loop {
        match sub2.levels.recv_timeout(Duration::from_millis(250)) {
            Ok(_) => received += 1,
            Err(_) => break,
        }
    }
    assert!(
        received >= 1,
        "节流不得丢弃全部电平（至少送达 1 条；收到 {received}）"
    );
    assert!(
        received <= 10,
        "电平流必须节流至 ≤10Hz（REQ-31）：250ms 窗口收到 {received} 条（未节流应为 50）"
    );
}
