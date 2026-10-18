//! 时间轴单位换算（REQ-25 统一 epoch；specs/001-screen-recorder.md §2.6/§3.4）。
//!
//! 三个显式域（**禁止隐式混用**）：
//! - **QPC ticks**：`QueryPerformanceCounter` 计数，也是 WGC `SystemRelativeTime` 的域
//!   （`windows-capture` 以 100ns ticks 呈现，见 sr-capture `WGC_TICKS_PER_SEC`）；
//! - **100ns（hns）单位**：WASAPI 设备位置/时间戳与内部时间轴的统一单位（REQ-18/25）；
//! - **UNIX epoch 毫秒**：对外时间戳（命名、清单展示）。
//!
//! 换算规则（显式实现 + 单测，REQ-25）：
//! - ticks ↔ hns 使用 **round-half-away-from-zero**（±0.5 向远离 0 的方向），
//!   以 i128 中间量避免乘法溢出，超出 i64 时饱和；
//! - 负值合法（屏障前数据以 T0 为 0，early/late 均为负/正），舍入对负数对称；
//! - 非法频率（≤0）不猜默认值，返回 `None`；
//! - [`Timeline`] 以公共 T0（QPC ticks + 对应 UNIX ms）把 QPC/设备位置映射到同一
//!   相对时间轴（100ns / 秒），并提供 epoch 毫秒回算（跨日由整数运算自然处理）。
//!
//! 纯换算函数不读取系统状态（确定性单测）；`qpc_*_now` 为 OS 读取（route 会话起点复用）。

/// 每秒的 100ns 单位数。
pub const HNS_PER_SEC: i64 = 10_000_000;
/// 每毫秒的 100ns 单位数。
pub const HNS_PER_MS: i64 = 10_000;

/// i128 → i64 饱和转换（超出范围夹到 i64::MIN/MAX）。
fn clamp_i64(v: i128) -> i64 {
    i64::try_from(v).unwrap_or(if v.is_negative() { i64::MIN } else { i64::MAX })
}

/// 四舍五入的整数除法（round-half-away-from-zero；`d` 必须 > 0）。
fn div_round(n: i128, d: i128) -> i128 {
    debug_assert!(d > 0, "div_round 的分母必须为正");
    let half = d / 2;
    if n >= 0 {
        (n + half) / d
    } else {
        (n - half) / d
    }
}

/// QPC ticks → 100ns（四舍五入；`freq <= 0` → `None`）。
pub fn qpc_ticks_to_hns(ticks: i64, freq: i64) -> Option<i64> {
    if freq <= 0 {
        return None;
    }
    Some(clamp_i64(div_round(
        ticks as i128 * HNS_PER_SEC as i128,
        freq as i128,
    )))
}

/// 100ns → QPC ticks（四舍五入；`freq <= 0` → `None`）。
pub fn hns_to_qpc_ticks(hns: i64, freq: i64) -> Option<i64> {
    if freq <= 0 {
        return None;
    }
    Some(clamp_i64(div_round(
        hns as i128 * freq as i128,
        HNS_PER_SEC as i128,
    )))
}

/// UNIX 毫秒 → 自 UNIX epoch 的 100ns（饱和到 i64）。
pub fn unix_ms_to_hns(ms: u64) -> i64 {
    clamp_i64(ms as i128 * HNS_PER_MS as i128)
}

/// 自 UNIX epoch 的 100ns → UNIX 毫秒（向下取整；epoch 之前（含 0）→ 0）。
pub fn hns_to_unix_ms(hns: i64) -> u64 {
    if hns <= 0 {
        0
    } else {
        (hns / HNS_PER_MS) as u64
    }
}

/// 公共时间轴：QPC 域与 100ns/epoch 域的可逆映射（T0 = 会话起点，REQ-25）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeline {
    freq: i64,
    qpc_t0: i64,
    qpc_t0_hns: i64,
    unix_t0_ms: u64,
}

impl Timeline {
    /// `freq > 0` 才构造；`qpc_t0` 为公共 T0 的 QPC ticks，`unix_t0_ms` 为同一时刻的
    /// UNIX 毫秒（由调用方在同一临界区内读取，二者偏差由调用方保证）。
    pub fn new(freq: i64, qpc_t0: i64, unix_t0_ms: u64) -> Option<Self> {
        let qpc_t0_hns = qpc_ticks_to_hns(qpc_t0, freq)?;
        Some(Self {
            freq,
            qpc_t0,
            qpc_t0_hns,
            unix_t0_ms,
        })
    }

    /// QPC 频率（ticks/秒）。
    pub fn freq(&self) -> i64 {
        self.freq
    }

    /// 公共 T0 的 QPC ticks。
    pub fn qpc_t0(&self) -> i64 {
        self.qpc_t0
    }

    /// 公共 T0 的 UNIX 毫秒。
    pub fn unix_t0_ms(&self) -> u64 {
        self.unix_t0_ms
    }

    fn ticks_to_hns(&self, ticks: i64) -> i64 {
        qpc_ticks_to_hns(ticks, self.freq).expect("Timeline::new 已校验 freq > 0")
    }

    /// QPC ticks（WGC SystemRelativeTime）→ 相对 T0 的 100ns（可为负 = 屏障前）。
    pub fn qpc_to_rel_hns(&self, qpc_ticks: i64) -> i64 {
        self.ticks_to_hns(qpc_ticks) - self.qpc_t0_hns
    }

    /// 相对 T0 的 100ns → QPC ticks。
    pub fn rel_hns_to_qpc(&self, rel_hns: i64) -> i64 {
        hns_to_qpc_ticks(rel_hns + self.qpc_t0_hns, self.freq)
            .expect("Timeline::new 已校验 freq > 0")
    }

    /// WASAPI 设备位置/时间戳（100ns 域，与 QPC 同源）→ 相对 T0 的 100ns。
    pub fn hns_to_rel_hns(&self, hns: i64) -> i64 {
        hns - self.qpc_t0_hns
    }

    /// 相对 T0 的 100ns → UNIX 毫秒（向下取整；epoch 之前 → 0）。
    pub fn rel_hns_to_unix_ms(&self, rel_hns: i64) -> u64 {
        let v = self.unix_t0_ms as i128 * HNS_PER_MS as i128 + rel_hns as i128;
        if v <= 0 {
            0
        } else {
            u64::try_from(v / HNS_PER_MS as i128).unwrap_or(u64::MAX)
        }
    }

    /// QPC ticks → UNIX 毫秒。
    pub fn qpc_to_unix_ms(&self, qpc_ticks: i64) -> u64 {
        self.rel_hns_to_unix_ms(self.qpc_to_rel_hns(qpc_ticks))
    }

    /// QPC ticks → 相对 T0 的秒（f64；负值合法）。
    pub fn qpc_to_rel_secs(&self, qpc_ticks: i64) -> f64 {
        self.qpc_to_rel_hns(qpc_ticks) as f64 / HNS_PER_SEC as f64
    }
}

/// QPC 频率（ticks/秒）；读取失败 → 0（调用方按非法频率处理）。
pub fn qpc_frequency() -> i64 {
    use windows::Win32::System::Performance::QueryPerformanceFrequency;
    unsafe {
        let mut freq = 0i64;
        let _ = QueryPerformanceFrequency(&mut freq);
        freq.max(0)
    }
}

/// 当前 QPC ticks；读取失败 → 0。
pub fn qpc_now_ticks() -> i64 {
    use windows::Win32::System::Performance::QueryPerformanceCounter;
    unsafe {
        let mut count = 0i64;
        let _ = QueryPerformanceCounter(&mut count);
        count
    }
}

/// 当前 QPC → 100ns（REQ-25；频率非法 → 0）。
pub fn qpc_now_hns() -> i64 {
    let freq = qpc_frequency();
    qpc_ticks_to_hns(qpc_now_ticks(), freq).unwrap_or(0)
}
