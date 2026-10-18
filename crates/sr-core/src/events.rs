//! Channel 订阅纯逻辑（REQ-31，specs/001-screen-recorder.md §2.8/§3.3）。
//!
//! - [`ChannelHub`]`<S, A, L>`：状态快照 S / 告警·状态事件 A / 电平事件 L 三类载荷分离。
//!   - 订阅入口 [`ChannelHub::subscribe`] 返回**当前**状态快照（UI 重载重绑后可恢复视图）；
//!   - publish 为事务式投递：存在失效接收端（重载遗留的旧订阅）时丢弃本条事件并回收该订阅，
//!     发布方永不阻塞、不感知失败（REQ-31：发送失败不得中断录制）；重绑端以快照恢复；
//!   - 告警与电平分通道，电平洪泛不得饿死告警（REQ-27 关联）；
//!   - 电平按订阅 ≤10Hz 节流（REQ-31）：首条立即放行，节流区间内保留最新值，
//!     区间结束时由内部线程送达——保证任意区间至少送达 1 条，而非整段丢弃。
//! - [`LevelThrottle`]：节流的确定性核心（显式时间参数，可注入时钟单测）。
//!
//! 接收端为同步通道（[`Receiver`]：`recv_timeout`/`try_recv`），不依赖异步运行时；
//! src-tauri 后续仅做 Tauri Channel 绑定的薄包装（事件载荷枚举定型属后续里程碑，
//! 见 docs/milestones/m1-red-report.md §7 R5）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// 电平节流下限间隔：10Hz（REQ-31：电平 ≤10Hz）。
const LEVEL_MIN_INTERVAL: Duration = Duration::from_millis(100);
/// 无待发事件时 flush 线程的休眠上限（publish 会唤醒，此值仅为兜底轮询周期）。
const FLUSHER_IDLE_POLL: Duration = Duration::from_secs(60);

/// Channel 中枢：录制侧单生产端语义（publish/update_state），多订阅端（UI 可重载重绑）。
pub struct ChannelHub<S, A, L> {
    inner: Arc<HubInner<S, A, L>>,
}

struct HubInner<S, A, L> {
    state: Mutex<S>,
    subs: Mutex<Subs<A, L>>,
    /// 与 `subs` 配对；publish 新增 pending 时唤醒 flush 线程以重算最近到期时刻。
    level_cv: Condvar,
}

struct Subs<A, L> {
    alerts: Vec<AlertEntry<A>>,
    levels: Vec<LevelGate<L>>,
}

/// 告警通道的 hub 侧条目：发送端 + 订阅存活标记（与接收端共享）。
struct AlertEntry<A> {
    tx: mpsc::Sender<A>,
    live: Arc<AtomicBool>,
}

/// 单个订阅的电平通道：首条立即放行；节流区间内事件仅保留最新值，到期由 flush 线程送达。
struct LevelGate<L> {
    tx: mpsc::Sender<L>,
    live: Arc<AtomicBool>,
    throttle: LevelThrottle,
    pending: Option<(L, Instant)>,
}

/// 订阅接收端（同步通道包装）：drop 时向 hub 标记订阅失效。
///
/// std mpsc 无 receiver_count，事务式投递以该标记为失效判定依据。
pub struct Receiver<T> {
    rx: mpsc::Receiver<T>,
    live: Arc<AtomicBool>,
}

impl<T> Receiver<T> {
    /// 阻塞至多 `timeout` 等待一条事件。
    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, mpsc::RecvTimeoutError> {
        self.rx.recv_timeout(timeout)
    }

    /// 非阻塞取一条事件。
    pub fn try_recv(&self) -> Result<T, mpsc::TryRecvError> {
        self.rx.try_recv()
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.live.store(false, Ordering::Release);
    }
}

/// 订阅句柄：订阅时刻的状态快照 + 告警/电平两条独立接收端。
pub struct Subscription<S, A, L> {
    /// 订阅时刻的状态快照（REQ-31：订阅入口传入 Channel + 当前状态快照）。
    pub snapshot: S,
    /// 告警/状态事件接收端（独立于电平通道，不被电平洪泛饿死）。
    pub alerts: Receiver<A>,
    /// 电平事件接收端（≤10Hz）。
    pub levels: Receiver<L>,
}

impl<S, A, L> ChannelHub<S, A, L>
where
    S: Clone + Send + 'static,
    A: Clone + Send + 'static,
    L: Clone + Send + 'static,
{
    /// 创建中枢并启动电平 trailing 送达线程（每 hub 一条，阻塞等待不占用 CPU）。
    pub fn new(initial_state: S) -> Self {
        let inner = Arc::new(HubInner {
            state: Mutex::new(initial_state),
            subs: Mutex::new(Subs {
                alerts: Vec::new(),
                levels: Vec::new(),
            }),
            level_cv: Condvar::new(),
        });
        let flusher_inner = Arc::clone(&inner);
        std::thread::Builder::new()
            .name("sr-core-level-flush".into())
            .spawn(move || level_flush_loop(flusher_inner))
            .expect("spawn sr-core level flusher thread");
        Self { inner }
    }

    /// 推进当前状态（之后的订阅将取得该快照）。
    pub fn update_state(&self, state: S) {
        *self.inner.state.lock().expect("state lock") = state;
    }

    /// 订阅入口：返回**当前**状态快照 + 告警/电平两条独立接收端（REQ-31）。
    pub fn subscribe(&self) -> Subscription<S, A, L> {
        let snapshot = self.inner.state.lock().expect("state lock").clone();
        let (alert_tx, alert_rx) = mpsc::channel();
        let (level_tx, level_rx) = mpsc::channel();
        let alert_live = Arc::new(AtomicBool::new(true));
        let level_live = Arc::new(AtomicBool::new(true));
        let gate = LevelGate {
            tx: level_tx,
            live: Arc::clone(&level_live),
            throttle: LevelThrottle::new(LEVEL_MIN_INTERVAL),
            pending: None,
        };
        let mut subs = self.inner.subs.lock().expect("subs lock");
        subs.alerts.push(AlertEntry {
            tx: alert_tx,
            live: Arc::clone(&alert_live),
        });
        subs.levels.push(gate);
        Subscription {
            snapshot,
            alerts: Receiver {
                rx: alert_rx,
                live: alert_live,
            },
            levels: Receiver {
                rx: level_rx,
                live: level_live,
            },
        }
    }

    /// 发布告警/状态事件（REQ-31）。
    ///
    /// 投递语义为**事务式**：当存在已失效的接收端（UI 重载遗留的旧订阅）时，
    /// 本次事件整体丢弃，仅回收失效订阅；存活订阅端以「订阅即快照」恢复状态视图。
    /// 因此每次 UI 重绑至多丢失一条在途事件，发布方永不阻塞、不感知失败（返回 ()）。
    pub fn publish_alert(&self, alert: A) {
        let mut subs = self.inner.subs.lock().expect("subs lock");
        if subs
            .alerts
            .iter()
            .any(|entry| !entry.live.load(Ordering::Acquire))
        {
            subs.alerts
                .retain(|entry| entry.live.load(Ordering::Acquire));
            return;
        }
        for entry in &subs.alerts {
            // 已预检存活；竞态下仍可能失败，容忍之（REQ-31：发送失败不得中断录制）。
            let _ = entry.tx.send(alert.clone());
        }
    }

    /// 发布电平事件（≤10Hz/订阅，REQ-31）。
    ///
    /// 首条立即放行；节流区间内仅保留最新值，区间结束时由内部线程送达
    /// （trailing 语义：任意区间至少送达 1 条，洪泛不堆积、不阻塞）。
    /// 存在失效接收端时与 [`Self::publish_alert`] 同为事务式投递（回收并丢弃本条）。
    pub fn publish_level(&self, level: L) {
        let now = Instant::now();
        let has_pending = {
            let mut subs = self.inner.subs.lock().expect("subs lock");
            if subs
                .levels
                .iter()
                .any(|gate| !gate.live.load(Ordering::Acquire))
            {
                subs.levels.retain(|gate| gate.live.load(Ordering::Acquire));
                false
            } else {
                let mut earliest: Option<Instant> = None;
                subs.levels.retain_mut(|gate| {
                    if gate.throttle.should_emit(now) {
                        gate.tx.send(level.clone()).is_ok()
                    } else {
                        let due = gate
                            .throttle
                            .last_emit()
                            .map(|t| t + LEVEL_MIN_INTERVAL)
                            .unwrap_or(now);
                        match gate.pending.as_mut() {
                            Some(slot) => {
                                slot.0 = level.clone();
                                slot.1 = due;
                            }
                            None => gate.pending = Some((level.clone(), due)),
                        }
                        earliest = Some(earliest.map_or(due, |e| e.min(due)));
                        true
                    }
                });
                earliest.is_some()
            }
        };
        if has_pending {
            self.inner.level_cv.notify_all();
        }
    }
}

/// 电平 trailing 送达线程：把各订阅节流区间内保留的最新值在到期时送出。
fn level_flush_loop<S, A, L>(inner: Arc<HubInner<S, A, L>>)
where
    L: Clone + Send + 'static,
{
    let mut subs = inner.subs.lock().expect("subs lock");
    loop {
        let now = Instant::now();
        for gate in subs.levels.iter_mut() {
            let due_reached = matches!(gate.pending, Some((_, due)) if due <= now);
            if due_reached {
                if let Some((level, _)) = gate.pending.take() {
                    // 接收端已亡：丢弃 pending，该订阅留待下次 publish 清除。
                    let _ = gate.tx.send(level);
                    // 记账本次送达，维持 ≤10Hz 节奏。
                    let _ = gate.throttle.should_emit(now);
                }
            }
        }
        let next_due = subs
            .levels
            .iter()
            .filter_map(|gate| gate.pending.as_ref().map(|(_, due)| *due))
            .min();
        let timeout = next_due
            .map(|due| due.saturating_duration_since(Instant::now()))
            .unwrap_or(FLUSHER_IDLE_POLL);
        let (guard, _) = inner
            .level_cv
            .wait_timeout(subs, timeout)
            .expect("subs lock");
        subs = guard;
    }
}

/// 电平节流确定性核心（REQ-31 ≤10Hz；显式时间参数，可注入时钟单测）。
#[derive(Debug, Clone)]
pub struct LevelThrottle {
    min_interval: Duration,
    last_emit: Option<Instant>,
}

impl LevelThrottle {
    /// `min_interval` 为最小发射间隔（10Hz ⇒ 100ms）。
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            last_emit: None,
        }
    }

    /// `now` 时刻是否允许发射：首事件立即放行；距上次发射不足 `min_interval` 则节流。
    /// 允许时记账（last_emit = now）。
    pub fn should_emit(&mut self, now: Instant) -> bool {
        let allowed = match self.last_emit {
            None => true,
            Some(last) => now.duration_since(last) >= self.min_interval,
        };
        if allowed {
            self.last_emit = Some(now);
        }
        allowed
    }

    /// 上次发射时刻（flush 线程计算下次送达时刻用）。
    pub fn last_emit(&self) -> Option<Instant> {
        self.last_emit
    }
}
