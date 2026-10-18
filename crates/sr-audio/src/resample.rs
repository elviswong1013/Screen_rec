//! 漂移滞回带与速率限幅（REQ-26 慢漂移决策核）：参数化、与阶跃校正分开记账。
//!
//! 契约（测试即契约，docs/milestones/m4-red-report.md §3.3 + drift_hysteresis.rs）：
//! - |offset| < enter ⇒ Hold（未进入校正时）；
//! - |offset| ≥ enter 进入校正：速率符号与偏差同号、非零、饱和于 ±rate_limit_ppm；
//! - 滞回：校正态在 (exit, enter) 区间**维持**校正；|offset| ≤ exit 退出；
//!   退出后 |offset| < enter 不得自行再进入；
//! - 增益（offset→rate 比例）不钉死（S6 定值前实现自由）；测量窗口/滤波属实现记账。
//!
//! 变比重采样（REQ-26/D2）：`DriftResampler` 使用 rubato 5.x `Async` + Sinc 插值
//! 实际应用决策速率；延迟（sinc 群延迟）参数化记账并做补偿，见其类型文档。

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Adjustable, Async, FixedAsync, Resampler, Resizable, SincInterpolationParameters,
    WindowFunction,
};

/// REQ-26 参数化漂移控制配置（三参数测试注入；锚点 40/20/500，S6 定值前可调）。
#[derive(Debug, Clone, PartialEq)]
pub struct DriftCfg {
    /// 进入校正的 |偏差| 阈值（ms）。
    pub enter_ms: f64,
    /// 退出阈值（< enter，滞回带）。
    pub exit_ms: f64,
    /// 速率限幅（ppm；±rate_limit_ppm 饱和）。
    pub rate_limit_ppm: f64,
}

/// 单次测量的决策输出。
#[derive(Debug, Clone, PartialEq)]
pub enum DriftAction {
    Hold,
    /// 符号与偏差同号（正 = 音频落后须加速/丢）；≠0；|·| ≤ limit。
    Correct {
        rate_ppm: f64,
    },
}

/// 漂移控制器（每音轨一个；滞回状态内部记账）。
pub struct DriftController {
    cfg: DriftCfg,
    in_correction: bool,
}

/// 增益（offset_ms → ppm 的比例系数；实现自由，S6 定值前取 10ppm/ms——
/// 40ms 锚点 → 400ppm，120ms 即触限幅；仅影响响应速度，不影响契约形状）。
const GAIN_PPM_PER_MS: f64 = 10.0;

impl DriftController {
    pub fn new(cfg: DriftCfg) -> Self {
        assert!(cfg.enter_ms > cfg.exit_ms, "滞回要求 enter_ms > exit_ms");
        assert!(cfg.exit_ms >= 0.0, "exit_ms 必须非负");
        assert!(cfg.rate_limit_ppm > 0.0, "rate_limit_ppm 必须为正");
        Self {
            cfg,
            in_correction: false,
        }
    }

    /// 输入内容偏差测量（正 = 音频内容落后基准：须加速/丢；负 = 超前：须减速/补）。
    pub fn on_measurement(&mut self, content_offset_ms: f64) -> DriftAction {
        let abs = content_offset_ms.abs();
        if self.in_correction {
            // 滞回：校正态须 |offset| ≤ exit 才退出；(exit, enter) 区间维持。
            if abs <= self.cfg.exit_ms {
                self.in_correction = false;
                return DriftAction::Hold;
            }
            self.correct(content_offset_ms)
        } else {
            if abs < self.cfg.enter_ms {
                return DriftAction::Hold;
            }
            self.in_correction = true;
            self.correct(content_offset_ms)
        }
    }

    /// 校正速率：与偏差同号、饱和于限幅（巨大偏差恰好限幅值）。
    fn correct(&self, content_offset_ms: f64) -> DriftAction {
        let raw = content_offset_ms * GAIN_PPM_PER_MS;
        let rate = raw.clamp(-self.cfg.rate_limit_ppm, self.cfg.rate_limit_ppm);
        DriftAction::Correct { rate_ppm: rate }
    }
}

/// 慢漂移路径的 sinc 滤波器长度（tap 数；rubato 参数，REQ-26 记账）。
///
/// 128 在 48kHz 下群延迟为 `SINC_LEN / 2` 输出帧（~1.33ms），对 440Hz~音乐通带
/// 的幅频滚降远低于可闻阈值；rubato 默认 256 质量更高但 CPU/延迟翻倍（S6 可调）。
const SINC_LEN: usize = 128;

/// 单次交给 rubato 的最大输入块（帧）。调用方块更大时按此切块，避免按最坏
/// 调用尺寸预分配内部缓冲；切块不改变输出帧数/样本（见 `drift_resampler` 单测）。
const MAX_CHUNK: usize = 8192;

/// rubato 允许的比率相对范围（±1000ppm 限幅 ≈ 1.001，留余量）。
const MAX_RATIO_REL: f64 = 1.01;

/// 速率输入限幅（ppm）：`DriftController` 的规格锚点为 ±500ppm（S6 定值），
/// 重采样器自身防御性限幅沿用既有单测契约 ±1000ppm。
const MAX_PPM: f64 = 1000.0;

/// rubato 内容延迟换算：`output_delay()` 以滤波支撑计（`sinc_len/2`），实测
/// 输出索引 0 已含首帧内容，内容延迟比其少 1 帧（见 `impulse_delay_*` 单测）。
const DELAY_SUPPORT_BIAS: f64 = 1.0;

/// 变比重采样器（REQ-26/D2：录制管线把 `DriftController` 的 ppm 速率应用到样本流；
/// M15/REQ-6：基础比率 = 输出率/捕获率）。
///
/// 实现：rubato 5.x **Async + Sinc 插值**（BlackmanHarris2 窗，Cubic 插值），
/// `FixedAsync::Input` 固定输入、输出帧数随比率变化。接口保持「`set_rate_ppm` +
/// `process`」。
///
/// 语义：
/// - `base` = `out_rate / in_rate`；有效 `ratio = base / (1 + ppm/1e6)`。正 ppm
///   （音频落后 → 加速/丢）输出帧数变少；负 ppm（减速/补）变多；
/// - `base == 1` 且 0ppm：逐样本恒等直通（既有 48k 基线行为不变）；`base != 1`
///   时 0ppm **不得**直通——构造即进入 rubato，按基础比率转换；
/// - 分块：`process` 接受任意长度交错 f32，内部按 ≤ [`MAX_CHUNK`] 帧切块并逐块
///   `set_chunk_size`，跨块相位由 rubato 状态连续保持（分块/整块输出一致）；
/// - 延迟补偿：rubato 的 sinc 群延迟约 `sinc_len/2` 输出帧（实测内容延迟为
///   支撑一半减 1 帧；[`Self::delay_frames`] 显式记账，常量不累积）。直通/校正
///   切换的补偿逻辑与既有契约一致；冷启动前 `delay_frames()` 帧为滤波器启动
///   斜坡（REQ-26 记账项）；
/// - 通道数固定（管线 2），输入/输出均为交错 f32。
pub struct DriftResampler {
    channels: usize,
    /// 基础比率（输出率 / 输入率；M15/REQ-6）。
    base_ratio: f64,
    rate_ppm: f64,
    mode: Mode,
    /// 期望输出帧数（输入时间轴映射 + 冷启动滤波延迟；f64 累积，避免逐块舍入）。
    expected_out: f64,
    /// 已返回输出帧数（`process` + `flush`）。
    emitted_out: u64,
    /// 冷启动首次 `process` 时补记的滤波延迟（中途接入已补偿，为 0）。
    pending_delay: f64,
    /// `flush` 为终结操作（重复调用返回空）。
    flushed: bool,
}

enum Mode {
    /// 未进入漂移校正（始终 0ppm）：逐样本恒等，保留 1 帧尾部 + 最近历史帧
    /// 供首次接入 rubato 时预热（`history` 为最近 `SINC_LEN/2` 帧输出）。
    Passthrough { tail: Vec<f32>, history: Vec<f32> },
    /// 已接入 rubato。`carry` 为接入时直通路径保留的 1 帧；`trim_remaining`
    /// 为中途接入时待丢弃的输出帧（延迟补偿），`trim_base` 为已登记补偿量。
    Active {
        resampler: Box<Async<f32>>,
        carry: Vec<f32>,
        trim_remaining: usize,
        trim_base: usize,
    },
}

impl DriftResampler {
    /// 构造重采样器：`in_rate` = 实际捕获率，`out_rate` = 成品输出率。
    ///
    /// `base = out_rate / in_rate`。`base != 1` 时构造函数即进入 rubato（0ppm 也
    /// 必须转换，不得直通）；`base == 1` 时保持直通，直到首次非零 ppm 才接入。
    pub fn new(channels: usize, in_rate: u32, out_rate: u32) -> Self {
        assert!(channels > 0, "DriftResampler 通道数必须为正");
        assert!(in_rate > 0 && out_rate > 0, "采样率必须为正");
        let base_ratio = out_rate as f64 / in_rate as f64;
        let (mode, pending_delay) = if (base_ratio - 1.0).abs() < f64::EPSILON {
            (
                Mode::Passthrough {
                    tail: Vec::new(),
                    history: Vec::new(),
                },
                0.0,
            )
        } else {
            (
                Mode::Active {
                    resampler: Box::new(build_resampler(base_ratio, channels)),
                    carry: Vec::new(),
                    trim_remaining: 0,
                    trim_base: 0,
                },
                content_delay_frames(base_ratio),
            )
        };
        Self {
            channels,
            base_ratio,
            rate_ppm: 0.0,
            mode,
            expected_out: 0.0,
            emitted_out: 0,
            pending_delay,
            flushed: false,
        }
    }

    /// 当前有效比率（每输入帧产生的输出帧数）。
    fn effective_ratio(&self, ppm: f64) -> f64 {
        self.base_ratio / (1.0 + ppm / 1_000_000.0)
    }

    /// 设置速率（ppm；内部限幅 ±1000ppm 防误用；0 = 不校正，回落基础比率）。
    ///
    /// `base == 1` 且 0ppm 且尚未校正过时保持逐样本直通（既有契约：恒等、仅尾部
    /// 1 帧延迟）；首次非零速率在此接入 rubato，并完成延迟补偿预热。已接入后
    /// 0ppm 仍走 rubato（ratio=base），避免在直通/重采样两种延迟之间来回切换。
    pub fn set_rate_ppm(&mut self, ppm: f64) {
        let ppm = ppm.clamp(-MAX_PPM, MAX_PPM);
        if ppm == self.rate_ppm {
            return;
        }
        self.rate_ppm = ppm;
        let ch = self.channels;
        let ratio = self.effective_ratio(ppm);

        if let Mode::Active { resampler, .. } = &mut self.mode {
            if (resampler.resample_ratio() - ratio).abs() > f64::EPSILON {
                resampler
                    .set_resample_ratio(ratio, false)
                    .expect("rubato 比率已限幅在构造的相对范围内");
            }
            return;
        }
        if ppm == 0.0 {
            return;
        }

        // 首次接入：取出直通状态（尾部 carry + 最近历史帧）。
        let (carry, history) = match std::mem::replace(
            &mut self.mode,
            Mode::Passthrough {
                tail: Vec::new(),
                history: Vec::new(),
            },
        ) {
            Mode::Passthrough { tail, history } => (tail, history),
            Mode::Active { .. } => unreachable!("上方已处理 Active"),
        };
        let mut resampler = build_resampler(ratio, ch);
        let mut trim_remaining = 0;
        let mut trim_base = 0;
        if !history.is_empty() {
            // 中途接入：预热 = 已经写出的历史帧（其输出全部丢弃）。预热输出
            // 覆盖到内容位置 `a + (W - D)/r`（a=历史起点、W=预热输出帧数、
            // D=滤波群延迟），丢弃 T = H*r - W + D 帧后恰好从直通保留的
            // carry（a+H，即下一输入帧）继续：无重放、无跳帧、无累积偏移。
            let warm_frames = history.len() / ch;
            let warm_out = process_block(&mut resampler, &history[..warm_frames * ch], ch);
            let delay = content_delay_frames(ratio);
            let trim = (warm_frames as f64 * ratio - (warm_out.len() / ch) as f64 + delay)
                .max(0.0)
                .round() as usize;
            trim_remaining = trim;
            trim_base = trim;
        } else {
            // 冷启动：尚未写过样本，输出开头会包含滤波启动斜坡；把该固定延迟
            // 计入期望帧数，停止 flush 时补齐尾部（内容完整、长度含延迟）。
            self.pending_delay = content_delay_frames(ratio);
        }
        self.mode = Mode::Active {
            resampler: Box::new(resampler),
            carry,
            trim_remaining,
            trim_base,
        };
    }

    /// 当前路径的固定内容延迟（输出帧，含小数；不随运行时间累积）。
    ///
    /// 直通 = 1（尾部保留帧）；rubato 冷启动 = 实测内容延迟
    /// `(SINC_LEN/2 − 1) × ratio`（约 63 帧 ≈ 1.31ms）；中途接入完成补偿后 = 0。
    /// REQ-26「重采样延迟补偿」的记账入口。
    pub fn delay_frames(&self) -> f64 {
        match &self.mode {
            Mode::Passthrough { .. } => 1.0,
            Mode::Active {
                resampler,
                trim_base,
                ..
            } => (content_delay_frames(resampler.resample_ratio()) - *trim_base as f64).max(0.0),
        }
    }

    /// 处理一块交错样本，返回重采样后的交错样本。
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        let ch = self.channels;
        let frames = input.len() / ch;
        if frames == 0 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(frames * ch);
        match &mut self.mode {
            Mode::Passthrough { tail, history } => {
                let mut buf: Vec<f32> = Vec::with_capacity((frames + 1) * ch);
                buf.extend_from_slice(tail);
                buf.extend_from_slice(&input[..frames * ch]);
                let total = buf.len() / ch;
                // 0ppm 恒等：输出 = 历史 + 当前块去掉最后一帧（尾部 1 帧延迟）。
                out.extend_from_slice(&buf[..(total - 1) * ch]);
                tail.clear();
                tail.extend_from_slice(&buf[(total - 1) * ch..]);
                // 保留最近 SINC_LEN/2 帧已写输出，供中途接入预热。
                let keep = (SINC_LEN / 2).min(out.len() / ch);
                if keep > 0 {
                    history.clear();
                    history.extend_from_slice(&out[(out.len() / ch - keep) * ch..]);
                }
                // 直通为 1:1 时间轴（尾部 1 帧由 flush 补齐）。
                self.expected_out += frames as f64;
            }
            Mode::Active {
                resampler,
                carry,
                trim_remaining,
                ..
            } => {
                if self.pending_delay > 0.0 {
                    self.expected_out += self.pending_delay;
                    self.pending_delay = 0.0;
                }
                self.expected_out += frames as f64 * resampler.resample_ratio();
                let mut block: Vec<f32> = Vec::with_capacity(carry.len() + frames * ch);
                block.append(carry); // 接入时直通保留的下一帧
                block.extend_from_slice(&input[..frames * ch]);
                let total = block.len() / ch;
                let mut raw = Vec::with_capacity(total * ch);
                let mut pos = 0;
                while pos < total {
                    let n = (total - pos).min(MAX_CHUNK);
                    let produced = process_block(resampler, &block[pos * ch..(pos + n) * ch], ch);
                    raw.extend_from_slice(&produced);
                    pos += n;
                }
                // 延迟补偿：丢弃中途接入登记的待补偿输出帧。
                let raw_frames = raw.len() / ch;
                let drop = (*trim_remaining).min(raw_frames);
                *trim_remaining -= drop;
                out.extend_from_slice(&raw[drop * ch..]);
            }
        }
        self.emitted_out += (out.len() / ch) as u64;
        out
    }

    /// 停止冲刷（终结操作；可重复调用，第二次起返回空）。
    ///
    /// 把重采样器内部尚未吐出的尾部按理论帧数补齐：直通 = 保留的 1 帧；
    /// rubato = 泵零直到「已返回帧数」达到期望值 `Σ 输入×比率 + 冷启动延迟`
    /// （中途接入已补偿，延迟项为 0）。返回交错 f32，调用方应在关闭音频管道、
    /// 发 EOF 之前写入（`route.rs` 音频写线程停止路径）。
    ///
    /// 输出帧数按 `round(期望值)` 截断，误差 ≤1 帧；内容为滤波器对真实输入的
    /// 自然衰减尾部，不会引入额外静音段。
    pub fn flush(&mut self) -> Vec<f32> {
        if self.flushed {
            return Vec::new();
        }
        self.flushed = true;
        let ch = self.channels;
        let target = self.expected_out.round().max(0.0) as u64;
        let mut out = Vec::new();
        match &mut self.mode {
            Mode::Passthrough { tail, .. } => {
                if self.emitted_out < target {
                    out = std::mem::take(tail);
                    self.emitted_out += (out.len() / ch) as u64;
                }
            }
            Mode::Active {
                resampler,
                carry,
                trim_remaining,
                ..
            } => {
                if self.emitted_out >= target {
                    return out;
                }
                let ratio = resampler.resample_ratio();
                // 首次块可带接入时保留的 carry；其余为零冲刷。
                let mut block: Vec<f32> = Vec::new();
                if !carry.is_empty() {
                    block.append(carry);
                }
                loop {
                    let need = target - self.emitted_out;
                    // 预留 carry 1 帧，保证单块不超过 rubato 最大块。
                    let pad_frames = ((need as f64 / ratio).ceil() as usize + 2 * SINC_LEN)
                        .clamp(1, MAX_CHUNK - 1);
                    block.resize(block.len() / ch * ch + pad_frames * ch, 0.0);
                    let raw = process_block(resampler, &block, ch);
                    block.clear();
                    let raw_frames = raw.len() / ch;
                    let drop = (*trim_remaining).min(raw_frames);
                    *trim_remaining -= drop;
                    let avail = raw_frames - drop;
                    let take = (avail as u64).min(need) as usize;
                    out.extend_from_slice(&raw[drop * ch..(drop + take) * ch]);
                    self.emitted_out += take as u64;
                    // raw_frames == 0 才无进展；trim 全消耗时仍需继续泵零。
                    if self.emitted_out >= target || raw_frames == 0 {
                        break;
                    }
                }
            }
        }
        out
    }
}

/// 内容延迟（输出帧）：sinc 支撑的一半减去 1 帧偏置（见 [`DELAY_SUPPORT_BIAS`]）。
fn content_delay_frames(ratio: f64) -> f64 {
    (SINC_LEN as f64 / 2.0 - DELAY_SUPPORT_BIAS) * ratio
}

/// 构造 rubato sinc 重采样器（Async、固定输入、输出可变）。
fn build_resampler(ratio: f64, channels: usize) -> Async<f32> {
    let params = SincInterpolationParameters::new(SINC_LEN, WindowFunction::BlackmanHarris2);
    Async::<f32>::new_sinc(
        ratio,
        MAX_RATIO_REL,
        &params,
        MAX_CHUNK,
        channels,
        FixedAsync::Input,
    )
    .expect("rubato sinc 构造参数固定有效")
}

/// 处理一个 1..=MAX_CHUNK 帧的块，返回交错输出。
fn process_block(resampler: &mut Async<f32>, data: &[f32], channels: usize) -> Vec<f32> {
    let frames = data.len() / channels;
    debug_assert!((1..=MAX_CHUNK).contains(&frames));
    resampler
        .set_chunk_size(frames)
        .expect("块长在 rubato 最大块内");
    let adapter = InterleavedSlice::new(data, channels, frames).expect("交错样本布局一致");
    resampler
        .process(&adapter, None)
        .expect("rubato 处理参数一致")
        .take_data()
}
