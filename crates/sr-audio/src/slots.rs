//! 样本槽编排（REQ-18）：WASAPI 包（100ns 时间戳）→ 写入计划（补齐/裁切/接续）。
//!
//! 契约（测试即契约，docs/milestones/m4-red-report.md §3.2 + silence_pad_late_packet.rs）：
//! - 首包建立 T0（其起始槽 = 0；绝对时间戳任意——WASAPI 包时间戳为 QPC 域 100ns）；
//! - 起始槽 = floor((ts−T0)·rate/10⁷)（100ns→样本显式换算，REQ-25）；
//! - 空洞（起始槽 > 水位）→ 先补 `pad_frames` 零帧再写（静音补齐按样本槽）；
//! - 迟到（起始槽 < 水位）→ 裁切重叠前缀，余量在水位接续；整包落在已写区间 →
//!   丢弃（水位不变）——禁止同一时间段写两次（REQ-18 红线）；
//! - SILENT 旗标（真实静音）同样占时间轴（内容置零由写入方执行，不触发额外差异）；
//! - `written_frames()` = 已编排输出总帧数（含补齐；迟到裁切不回退）。
//!
//! 供流式生产使用的扩展（不影响上述契约面）：`pad_to_wall_clock`（墙钟续流——
//! **仅源真实停滞时**补齐到 now，判定见 [`DEFAULT_STALL_100NS`]）、`pad_to_ts`
//! （无条件推进原语，保留给已判定停滞/显式续流的调用方）、`pad_to`（把总帧数补到
//! 请求值——时长钉齐，S3 §3.3）。

/// 源停滞判定阈值（100ns；50ms）。
///
/// 依据：WASAPI 共享模式周期标称 10ms（本机实测 10ms 与 26.6ms 两种），loopback 包
/// 时间戳相对当前 QPC 的固有落后（端点/缓冲延迟）实测 0.3–0.7ms，drain 线程调度抖动
/// 可达数十 ms。50ms = 5×标称周期、≈2×最坏实测周期：既不会把正常抖动误判为停滞，
/// 又远小于真实源停滞（端点失效/音源消失通常 ≫100ms）。阈值只决定「何时开始补」，
/// 补的目标仍是当前 QPC，因此停滞恢复后的时间轴依然与墙钟对齐（REQ-18）。
pub const DEFAULT_STALL_100NS: i64 = 500_000;

/// 单个 WASAPI 包经样本槽编排后的写入计划（REQ-18）。
#[derive(Debug, Clone, PartialEq)]
pub struct PacketPlan {
    /// 写入前须补的静音帧数（时间轴空洞；0=连续）。
    pub pad_frames: u64,
    /// 迟到裁切：包首部与已写区间重叠而丢弃的帧数（== frames ⇒ 整包丢弃）。
    pub trimmed_frames: usize,
    /// 写入起始样本槽（相对 T0；迟到裁切时 = 当前水位，接续写入）。
    pub start_sample: u64,
}

/// 样本槽节奏器（每音轨一个；非线程安全，须在采集线程内使用）。
pub struct SampleSlotPacer {
    rate: u32,
    /// 首包时间戳（100ns；T0）。
    t0: Option<i64>,
    /// 已编排水位（含补齐；迟到裁切不回退）。
    watermark: u64,
    /// 最近一个真实包的结束时刻（100ns；只在 `on_packet` 更新并取最大值——
    /// 乱序/迟到包不得把停滞判据往回拨）。
    last_packet_end_100ns: Option<i64>,
    /// 源停滞阈值（100ns）：`pad_to_wall_clock` 的门限（见 [`DEFAULT_STALL_100NS`]）。
    stall_100ns: i64,
}

impl SampleSlotPacer {
    /// `sample_rate` = 管线固定速率（REQ-18：48_000）；停滞阈值取默认 50ms。
    pub fn new(sample_rate: u32) -> Self {
        Self::with_stall_threshold(sample_rate, DEFAULT_STALL_100NS)
    }

    /// 注入停滞阈值（端点周期特殊/测试用）；`stall_100ns` 必须为正。
    pub fn with_stall_threshold(sample_rate: u32, stall_100ns: i64) -> Self {
        assert!(sample_rate > 0, "sample_rate 必须为正");
        assert!(stall_100ns > 0, "停滞阈值必须为正");
        Self {
            rate: sample_rate,
            t0: None,
            watermark: 0,
            last_packet_end_100ns: None,
            stall_100ns,
        }
    }

    /// 无包端点续流锚点：在采集启动时建立 T0（仅当尚未建立）。
    ///
    /// M16/REQ-12：显式渲染端点可能长时间无客户端/无包（静音），首包永不 Establishment
    /// 会让墙钟续流无法工作；此锚点让采集核从启动时刻起按墙钟补静音（不判失效）。
    pub fn start_at(&mut self, ts_100ns: i64) {
        if self.t0.is_none() {
            self.t0 = Some(ts_100ns);
            self.last_packet_end_100ns = Some(ts_100ns);
        }
    }

    /// WASAPI 包：100ns 时间戳 + 帧数 + SILENT 旗标 → 写入计划。
    pub fn on_packet(&mut self, ts_100ns: i64, frames: usize, silent: bool) -> PacketPlan {
        let _ = silent; // 真实静音同样占时间轴：决策与普通包一致（内容置零由写入方执行）
        if self.t0.is_none() {
            self.t0 = Some(ts_100ns);
        }
        let t0 = self.t0.expect("T0 已建立");
        // 真实包的结束时刻（无论连续/迟到都记录）：墙钟续流的停滞判据。
        let end = self.packet_end_100ns(ts_100ns, frames);
        self.last_packet_end_100ns =
            Some(self.last_packet_end_100ns.map_or(end, |prev| prev.max(end)));
        let computed = if ts_100ns >= t0 {
            // floor((ts−T0)·rate/10⁷)：i128 中间量防乘法溢出；非负保证截断即 floor。
            ((ts_100ns - t0) as i128 * self.rate as i128 / 10_000_000i128) as u64
        } else {
            // T0 之前的包：视为迟到（落点为负 → 全部与已写区间重叠处理）。
            return self.late_plan(frames as usize, 0);
        };
        let frames = frames as u64;
        if computed >= self.watermark {
            // 连续或空洞：先补 (computed − 水位) 帧零再写。
            let pad = computed - self.watermark;
            self.watermark = computed + frames;
            PacketPlan {
                pad_frames: pad,
                trimmed_frames: 0,
                start_sample: computed,
            }
        } else {
            self.late_plan(frames as usize, computed)
        }
    }

    /// 迟到路径：裁切与已写区间重叠的前缀，余量在水位接续；整包重叠 ⇒ 丢弃。
    /// `start_sample` 恒为实际写入位（= 水位；整包丢弃时亦返回水位）。
    fn late_plan(&mut self, frames: usize, computed: u64) -> PacketPlan {
        let overlap = self.watermark.saturating_sub(computed) as usize;
        let trimmed = overlap.min(frames);
        let remaining = (frames - trimmed) as u64;
        let start = self.watermark;
        self.watermark += remaining;
        PacketPlan {
            pad_frames: 0,
            trimmed_frames: trimmed,
            start_sample: start,
        }
    }

    /// 已编排输出总帧数（含补齐；迟到裁切不回退）。
    pub fn written_frames(&self) -> u64 {
        self.watermark
    }

    /// 包结束时刻（100ns）：`ts + frames×10⁷/rate`（与起始槽同一换算）。
    fn packet_end_100ns(&self, ts_100ns: i64, frames: usize) -> i64 {
        ts_100ns.saturating_add((frames as i64).saturating_mul(10_000_000) / self.rate as i64)
    }

    /// 墙钟续流（REQ-18「流不断」）：仅当「`now_100ns` − 上一真实包结束」达到
    /// 停滞阈值（[`DEFAULT_STALL_100NS`]）时，才把时间轴推进到 `now_100ns`
    /// （补静音）；否则时间轴不动、返回 0。
    ///
    /// 为什么不能无条件推进：loopback 包时间戳天然落后当前 QPC 一个端点延迟
    /// （本机实测 0.3–0.7ms，调度抖动下可达数十 ms）。无条件 `pad_to_ts(now)`
    /// 会把该延迟补成静音，随后等量真实载荷被迟到裁切丢弃——时间轴不变，但内容
    /// 被逐段替换（M12-H 实测 610s 内 12–50s）。真实停滞（端点失效/音源消失）下
    /// 该间隔会远超阈值，补齐照常发生，且目标仍是 now，故墙钟对齐不受影响。
    pub fn pad_to_wall_clock(&mut self, now_100ns: i64) -> u64 {
        let Some(last_end) = self.last_packet_end_100ns else {
            return 0;
        };
        if now_100ns.saturating_sub(last_end) < self.stall_100ns {
            return 0;
        }
        self.pad_to_ts(now_100ns)
    }

    /// 把时间轴**无条件**推进到 `ts_100ns` 对应的槽位（原语；流式采集应使用
    /// [`Self::pad_to_wall_clock`]，否则会把正常端点延迟补成静音）。
    /// 返回补齐帧数（T0 未建立或已越过水位时为 0）。**单调**：已越过水位时
    /// 不回退水位（回退会让后续包对同一时段二次补零/重复写，REQ-18 红线）。
    pub fn pad_to_ts(&mut self, ts_100ns: i64) -> u64 {
        let Some(t0) = self.t0 else { return 0 };
        if ts_100ns < t0 {
            return 0;
        }
        let target = ((ts_100ns - t0) as i128 * self.rate as i128 / 10_000_000i128) as u64;
        let pad = target.saturating_sub(self.watermark);
        self.watermark = self.watermark.max(target);
        pad
    }

    /// 把已编排总帧数补到 `total_frames`（时长钉齐，S3 §3.3）。返回实际补齐帧数。
    pub fn pad_to(&mut self, total_frames: u64) -> u64 {
        let pad = total_frames.saturating_sub(self.watermark);
        self.watermark = self.watermark.max(total_frames);
        pad
    }
}
