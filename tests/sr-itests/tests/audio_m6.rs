//! L2 集成测试：M6 音频范围（REQ-15 系统音频 / REQ-16 麦克风 / REQ-19 纯录音 WAV /
//! REQ-17 全音轨模式补齐部分）；specs/001-screen-recorder.md §2.4（REQ-15/16/17/19 全文）、
//! §3.1/§3.3/§3.4/§3.5、§4（REQ-15/16/17/19 行）、§5 M6 行、§8 M0 冻结项（S3）。
//!
//! 运行方式（L2 门：`#[ignore]`，默认 `cargo test` 不执行本文件用例）：
//! ```text
//! cargo test -p sr-itests --test audio_m6 -- --ignored --nocapture
//! cargo test -p sr-itests --test audio_m6 it_audio_device_loopback_rms -- --ignored --nocapture   # 单用例
//! ```
//! 前置条件：交互式用户会话 + 可用默认渲染端点 + 仓库内锁定 ffmpeg n8.1 release 构建
//! （spec §8；含 ffplay/ffprobe）；麦克风用例另需采集设备与扬声器—麦克风声学耦合
//! （本机现状：默认渲染/采集均为蓝牙耳机、无耦合 → BLOCKED；见红报告 §6 设备状况）。
//! 缺失 → BLOCKED（fail），不静默 skip（§4）。
//! 本机环境（红测试编写时）：Windows 11 25H2，build 26200.9711，SESSIONNAME=Console
//! （非交互 shell 缺失该变量时用例按 §4 记 BLOCKED——运行时需确认）。
//!
//! 手法（S3 移植，§2/§3）：ffmpeg 合成正弦 WAV（440/880/1200Hz、音量 0.25 防混音
//! 削波二次谐波落位，M4 R2）→ 独立 ffplay 进程播放到默认输出 → 录制（设备环回/
//! 进程环回/纯录音 WAV）→ FFT（rustfft 32768 点 Hann、hop 半帧、帧间平均谱）判定
//! 存在性（高于本底 ≥20dB）与隔离（非目标 ≤ −40dB，S3 预锁定阈值）。
//!
//! 接口契约（测试即契约，详见 docs/milestones/m6-red-report.md §3；M4 契约的 M6 扩展面）：
//! - `sr_audio::wasapi::capture_device_loopback(duration) -> Result<CapturedPcm, CaptureError>`
//!   默认渲染端点设备环回（REQ-15；S3 apploop `--device-loopback` 同构），复用同一
//!   采集核（48k/f32/立体声 autoconvert、时长钉齐 ±48 帧）；默认端点缺失/激活失败 →
//!   `CaptureError::LoopbackUnavailable`（L2 → BLOCKED）。
//! - `sr_audio::wasapi::list_capture_devices() -> Result<Vec<AudioDeviceInfo>, CaptureError>`
//!   采集（麦克风）设备枚举（REQ-16；无设备 → Ok 空列表——明确报告，不静默）；
//!   `AudioDeviceInfo { id, name, is_default }`。
//! - `sr_audio::wasapi::capture_microphone(device: &str, duration) -> Result<CapturedPcm, CaptureError>`
//!   （REQ-16；device = AudioDeviceInfo.id；设备不存在/不可打开 → `CaptureError::MicUnavailable`
//!   快速返回，不得静默零包/静默成功——「无设备时明确不可用（SKIP 不得静默）」）。
//!   内容级验证（1200Hz 声学回路）前提为扬声器—麦克风耦合（§4 手法）；默认端点若为
//!   耳机/无声学回路（捕获 RMS 为数字静音级）→ 环境缺失分支记 BLOCKED（§4）。
//! - `sr_audio::wav`（REQ-19）：`repair_wav(path: &Path) -> Result<WavRepair, WavError>`
//!   头部重写（RIFF/data 长度）+ 尾部不完整样本裁切（崩溃残留；**幂等**：二跑无操作）；
//!   `WavRepair { frames: u64（修复后完整帧数，s16 立体声）, trimmed_bytes: u64（裁切字节数 0..3） }`。
//! - `sr_pipeline::route`（REQ-19）：`start_audio_only(AudioOnlyConfig { sources, output_dir })`
//!   `-> Result<AudioOnlyHandle, RouteError>`；`AudioOnlyHandle::request_stop()` /
//!   `wait_finalized(timeout) -> Result<AudioOnlyFinalized, RouteError>`（**停止即成品**：
//!   WAV 无 concat/后台收尾）；`AudioOnlyFinalized.wav_paths` 与 sources **同序同长**，
//!   每来源一条独立 WAV（48kHz s16 立体声）。
//! - `start_audio_route` 的 `AudioSourceSpec::System`（REQ-15 路由侧）与 `Mic`（REQ-16
//!   路由侧）语义落地（M4 时二者显式 TargetInvalid）；全音轨轨组织 = 给定来源列表
//!   各一条独立轨、互不混音（REQ-17「目标应用 + 系统 + 麦克风 + 追加应用」）。
//! - `CaptureError::MicUnavailable { detail }`（M6 新变体）。

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rustfft::{num_complex::Complex, FftPlanner};

use common::*;
use sr_audio::wasapi::{
    capture_device_loopback, capture_microphone, list_capture_devices, AudioDeviceInfo,
    AudioSourceSpec, CaptureError, CapturedPcm,
};
use sr_audio::wav::repair_wav;
use sr_pipeline::route::{
    start_audio_only, start_audio_route, AudioOnlyConfig, AudioOnlyHandle, AudioRouteConfig,
    RouteError, RouteHandle, RouteTarget, VideoCfg,
};

/// L2 音频用例串行门：同一测试二进制内 4 个用例各自起 ffplay/采集客户端/路由，
/// 串行执行（M4 R5/M5 R6 先例；全音轨用例同时开 3 个采集客户端，叠加并发会超出
/// S3 已验证并发面）。
static GATE: Mutex<()> = Mutex::new(());

fn gate() -> std::sync::MutexGuard<'static, ()> {
    GATE.lock().unwrap_or_else(|e| e.into_inner())
}

// ─────────────────────────── 判定阈值（S3 预锁定 + scratch 标定；见红报告 §7） ───────────────────────────

/// 音调存在性：目标频带峰值高于本底 ≥20dB（M4 `assert_tone_present` 同款）。
const TONE_PRESENT_DB: f64 = 20.0;
/// 隔离场景目标存在性：高于本底 ≥30dB（M4 `assert_tone_isolated` 同款）。
const ISOLATED_PRESENT_DB: f64 = 30.0;
/// 非目标泄漏：相对目标 ≤ −40dB（S3 预锁定阈值）。
const ISOLATION_DB: f64 = -40.0;
/// 系统轨（整机混音）两声电平差上限：S3 对照实测 0.62dB。
const MIX_PARITY_DB: f64 = 10.0;
/// 进程环回 WAV 非静音 RMS 下限（M5 同款：数字静音 ≈1e-4，实测 0.022 量级）。
const WAV_RMS_MIN: f64 = 0.005;
/// 设备环回非静音 RMS 下限（S3：设备路径比进程路径低 ≈5.4dB；阈值取该量级
/// 之下留裕度，数字静音 ≈1e-4）。
const DEV_RMS_MIN: f64 = 0.002;
/// 数字静音级 RMS 上限（M5 同款量级注记：数字静音 ≈1e-4；麦克风无声学回路时的实测
/// 量级，见红报告 §6 设备状况）。
const SILENCE_RMS: f64 = 1e-4;

// ─────────────────────────── 崩溃录制子进程再入协议（REQ-19） ───────────────────────────

/// 子进程再入环境变量（仅父进程设置；子进程模式 = 启动纯录音后保持，等待被强杀）。
const CHILD_DIR_ENV: &str = "SR_M6_WAV_CHILD_DIR";
const CHILD_PID_ENV: &str = "SR_M6_WAV_CHILD_SRC_PID";
const CHILD_CREATED_ENV: &str = "SR_M6_WAV_CHILD_SRC_CREATED";
/// 子进程就绪标记（含 started_unix_ms；stdout 已重定向到 child.log）。
const CHILD_STARTED_FILE: &str = "child.started";
/// 子进程启动失败诊断（BLOCKED 分流用）。
const CHILD_ERROR_FILE: &str = "child.error";

// ─────────────────────────── 锁定构建 / 音调源（S3 §2 手法） ───────────────────────────

/// 锁定 ffmpeg/ffplay/ffprobe 位置：spec §8 M0 冻结的 BtbN **n8.1 release** 构建。
fn ffbin(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin")
        .join(name)
        .canonicalize()
        .unwrap_or_else(|e| blocked(&format!("无法定位锁定 n8.1 构建的 {name}.exe（canonicalize 失败：{e}；spec §8 冻结 sidecar 构建）")));
    if !p.exists() {
        blocked(&format!("锁定 ffmpeg n8.1 构建缺失：{}", p.display()));
    }
    p
}

/// 进程树退出即清理的守卫（taskkill /T /F）。
struct KillTreeOnDrop(Child);
impl Drop for KillTreeOnDrop {
    fn drop(&mut self) {
        let _ = Command::new("taskkill")
            .args(["/PID", &self.0.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = self.0.wait();
    }
}

/// 用锁定 ffmpeg 生成测试音 WAV（sine 48k 立体声 f32；音量 0.25——两声相加 0.5
/// 不削波，避免 440 的二次谐波（削波失真）落位 880 造成假泄漏，M4 R2）。
fn gen_tone_wav(dir: &Path, name: &str, hz: u32) -> PathBuf {
    let out = dir.join(name);
    let st = Command::new(ffbin("ffmpeg.exe"))
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency={hz}:sample_rate=48000:duration=40,volume=0.25"),
            "-ac",
            "2",
            "-c:a",
            "pcm_f32le",
        ])
        .arg(&out)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap_or_else(|e| blocked(&format!("无法运行锁定 ffmpeg 生成测试音：{e}")));
    if !st.success() {
        blocked(&format!("ffmpeg 生成 {hz}Hz 测试音失败（exit={st}）"));
    }
    out
}

fn spawn_ffplay(wav: &Path) -> KillTreeOnDrop {
    let child = Command::new(ffbin("ffplay.exe"))
        .args(["-nodisp", "-autoexit", "-loglevel", "quiet"])
        .arg(wav)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn 锁定 ffplay：{e}")));
    KillTreeOnDrop(child)
}

// ─────────────────────────── ffprobe / 解码 ───────────────────────────

fn ffprobe_csv(path: &Path, args: &[&str]) -> String {
    let out = Command::new(ffbin("ffprobe.exe"))
        .args(["-v", "error"])
        .args(args)
        .arg(path)
        .output()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn 锁定 ffprobe：{e}")));
    assert!(
        out.status.success(),
        "ffprobe 失败（exit={}，stderr={}）",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn probe_duration(path: &Path) -> f64 {
    let s = ffprobe_csv(
        path,
        &["-show_entries", "format=duration", "-of", "csv=p=0"],
    );
    s.parse::<f64>()
        .ok()
        .filter(|d| *d > 0.0)
        .unwrap_or_else(|| panic!("应能读出容器时长（{} 未封口/损坏？）", path.display()))
}

/// 宽松时长探测（不 panic）——仅用于修复前取证打印。注意：未封口 WAV 的头部长度为
/// 占位值时 ffprobe 仍可能按文件大小推断出时长（scratch 实测）⇒ 该值不作断言。
fn probe_duration_soft(path: &Path) -> Option<f64> {
    let out = Command::new(ffbin("ffprobe.exe"))
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|d| *d > 0.0)
}

fn probe_stream_count(path: &Path, selector: &str) -> usize {
    let s = ffprobe_csv(
        path,
        &[
            "-select_streams",
            selector,
            "-show_entries",
            "stream=codec_type",
            "-of",
            "csv=p=0",
        ],
    );
    s.lines().filter(|l| !l.trim().is_empty()).count()
}

/// 音轨 idx 的 (codec, sample_rate, channels)。
fn probe_audio_stream_at(path: &Path, idx: usize) -> (String, u32, u32) {
    let sel = format!("a:{idx}");
    let s = ffprobe_csv(
        path,
        &[
            "-select_streams",
            &sel,
            "-show_entries",
            "stream=codec_name,sample_rate,channels",
            "-of",
            "csv=p=0",
        ],
    );
    // 形如 "pcm_s16le,48000,2"
    let parts: Vec<&str> = s.split(',').collect();
    assert_eq!(parts.len(), 3, "音轨 csv 形状（得到 {s:?}）");
    (
        parts[0].trim().to_string(),
        parts[1].trim().parse().expect("sample_rate"),
        parts[2].trim().parse().expect("channels"),
    )
}

/// 首视频流 (codec, width, height)。
fn probe_video_stream(path: &Path) -> (String, u32, u32) {
    let s = ffprobe_csv(
        path,
        &[
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name,width,height",
            "-of",
            "csv=p=0",
        ],
    );
    let parts: Vec<&str> = s.split(',').collect();
    assert_eq!(parts.len(), 3, "视频流 csv 形状（得到 {s:?}）");
    (
        parts[0].trim().to_string(),
        parts[1].trim().parse().expect("width"),
        parts[2].trim().parse().expect("height"),
    )
}

/// 解码音轨 idx 为 48k 单声道 f32（FFT/RMS 分析输入）。
fn decode_audio_track_mono(path: &Path, idx: usize) -> Vec<f32> {
    let sel = format!("0:a:{idx}");
    let out = Command::new(ffbin("ffmpeg.exe"))
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args([
            "-vn",
            "-map",
            &sel,
            "-f",
            "f32le",
            "-acodec",
            "pcm_f32le",
            "-ac",
            "1",
            "-ar",
            "48000",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn 锁定 ffmpeg 解码音轨：{e}")));
    assert!(out.status.success(), "音轨解码失败（exit={}）", out.status);
    assert_eq!(out.stdout.len() % 4, 0, "f32 字节流应为 4 字节倍数");
    out.stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

/// 解码首音轨为 48k 单声道 f32。
fn decode_audio_mono(path: &Path) -> Vec<f32> {
    decode_audio_track_mono(path, 0)
}

/// 整文件可解码（ffmpeg -v error -f null -）。
fn decode_clean(path: &Path) -> bool {
    let out = Command::new(ffbin("ffmpeg.exe"))
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn 锁定 ffmpeg 解码：{e}")));
    if !out.status.success() || !out.stderr.is_empty() {
        println!(
            "[decode] {} 报错：{}",
            path.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    out.status.success() && out.stderr.is_empty()
}

// ─────────────────────────── FFT 分析（S3 §2 方法移植，rustfft） ───────────────────────────

/// 32768 点 @48k：0.6827s 窗、bin=1.4648Hz（S3 同参数）；帧间平均功率谱。
const NFFT: usize = 32768;
/// 跳过首 1.0s（起播/激活过渡）。
const SKIP_HEAD_S: f64 = 1.0;
const SKIP_TAIL_S: f64 = 0.5;

struct Spectrum {
    bin_hz: f64,
    power: Vec<f64>,
}

fn avg_spectrum(x: &[f32], sample_rate: u32) -> Spectrum {
    use std::f64::consts::PI;
    assert!(x.len() > NFFT * 2, "可分析样本过短（{} < 2×NFFT）", x.len());
    let hop = NFFT / 2;
    let head = (SKIP_HEAD_S * sample_rate as f64) as usize;
    let tail = x
        .len()
        .saturating_sub((SKIP_TAIL_S * sample_rate as f64) as usize);
    let win: Vec<f64> = (0..NFFT)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / NFFT as f64).cos())
        .collect();

    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(NFFT);
    let mut acc = vec![0.0f64; NFFT];
    let mut frames = 0usize;
    let mut start = head;
    while start + NFFT <= tail {
        let mut buf: Vec<Complex<f64>> = (0..NFFT)
            .map(|i| Complex::new(x[start + i] as f64 * win[i], 0.0))
            .collect();
        fft.process(&mut buf);
        for k in 0..NFFT {
            acc[k] += buf[k].norm_sqr();
        }
        frames += 1;
        start += hop;
    }
    assert!(frames >= 2, "可分析帧数不足（{frames}）");
    for a in &mut acc {
        *a /= frames as f64;
    }
    Spectrum {
        bin_hz: sample_rate as f64 / NFFT as f64,
        power: acc,
    }
}

impl Spectrum {
    /// 频带 [hz−bw, hz+bw] 内最大 bin 的相对 dB（跨段比较用差值；S3：目标频带 ±5Hz 取最大 bin）。
    fn band_peak_rel_db(&self, hz: f64, half_bw: f64) -> f64 {
        let k0 = (((hz - half_bw) / self.bin_hz).floor() as usize).max(1);
        let k1 = (((hz + half_bw) / self.bin_hz).ceil() as usize).min(self.power.len() / 2);
        let p = (k0..=k1).fold(0.0f64, |m, k| m.max(self.power[k]));
        10.0 * p.log10()
    }

    /// 噪声底：[lo,hi] 去除排除带后的中位数功率（S3 §2 同手法）。
    fn band_median_rel_db(&self, lo: f64, hi: f64, exclude: &[(f64, f64)]) -> f64 {
        let mut ps: Vec<f64> = (1..self.power.len() / 2)
            .map(|k| (k, k as f64 * self.bin_hz))
            .filter(|(_, f)| {
                *f >= lo && *f <= hi && !exclude.iter().any(|&(a, b)| *f >= a && *f <= b)
            })
            .map(|(k, _)| self.power[k])
            .collect();
        assert!(ps.len() >= 8, "噪声底样本过少（{}）", ps.len());
        ps.sort_by(|a, b| a.partial_cmp(b).expect("NaN 功率"));
        ps[ps.len() / 2]
    }
}

/// 单轨 440/880 频带相对电平（相对本底与相互差值的判定输入）。
struct TrackBands {
    t440: f64,
    t880: f64,
    floor: f64,
}

fn bands_of(x: &[f32], sample_rate: u32) -> TrackBands {
    let s = avg_spectrum(x, sample_rate);
    TrackBands {
        t440: s.band_peak_rel_db(440.0, 5.0),
        t880: s.band_peak_rel_db(880.0, 5.0),
        floor: s.band_median_rel_db(100.0, 6000.0, &[(430.0, 450.0), (870.0, 890.0)]),
    }
}

/// 隔离判定（S3 预锁定）：目标存在（≥本底+30dB）且非目标 ≤ −40dB（相对目标）。
fn assert_tone_isolated(x: &[f32], sample_rate: u32, target_hz: f64, other_hz: f64, label: &str) {
    let s = avg_spectrum(x, sample_rate);
    let t = s.band_peak_rel_db(target_hz, 5.0);
    let o = s.band_peak_rel_db(other_hz, 5.0);
    let floor = s.band_median_rel_db(
        100.0,
        6000.0,
        &[
            (target_hz - 10.0, target_hz + 10.0),
            (other_hz - 10.0, other_hz + 10.0),
        ],
    );
    println!("[fft:{label}] target({target_hz}Hz)={t:.1}dB other({other_hz}Hz)={o:.1}dB floor={floor:.1}dB ⇒ 泄漏 {:.1}dB", o - t);
    assert!(
        t - floor >= ISOLATED_PRESENT_DB,
        "REQ-19：{label} 目标 {target_hz}Hz 应存在（高于本底 ≥{ISOLATED_PRESENT_DB}dB，差 {:.1}dB）",
        t - floor
    );
    assert!(
        o - t <= ISOLATION_DB,
        "REQ-19：{label} 非目标 {other_hz}Hz 泄漏应 ≤ −40dB（相对目标 {:.1}dB；S3 预锁定阈值）",
        o - t
    );
}

// ─────────────────────────── 被测 API 的 BLOCKED 包装 ───────────────────────────

fn capture_device_loopback_ok(duration: Duration, label: &str) -> CapturedPcm {
    match capture_device_loopback(duration) {
        Ok(c) => c,
        Err(e) if matches!(e, CaptureError::LoopbackUnavailable { .. }) => blocked(&format!(
            "{label}：默认渲染端点设备环回不可用（E_LOOPBACK_UNAVAILABLE，§3.5 环境缺失）：{e:?}"
        )),
        Err(e) => panic!("REQ-15：{label} 默认渲染端点设备环回应成功（得到 {e:?}）"),
    }
}

fn start_audio_only_ok(cfg: AudioOnlyConfig) -> AudioOnlyHandle {
    match start_audio_only(cfg) {
        Ok(h) => h,
        Err(e) if matches!(e, RouteError::LoopbackUnavailable { .. }) => blocked(&format!(
            "纯录音启动时环回不可用（E_LOOPBACK_UNAVAILABLE，§3.5 环境缺失）：{e:?}"
        )),
        Err(e) => panic!("REQ-19：start_audio_only 应成功（得到 {e:?}）"),
    }
}

fn start_audio_route_ok(cfg: AudioRouteConfig) -> RouteHandle {
    match start_audio_route(cfg) {
        Ok(h) => h,
        Err(e) if matches!(e, RouteError::EncoderUnavailable { .. }) => blocked(&format!(
            "无可用编码器（§9 T1：明确报错并列出可用编码器）：{e:?}"
        )),
        Err(e) if matches!(e, RouteError::ConsentDenied { .. }) => {
            blocked(&format!("WGC 授权被拒（E_CONSENT_DENIED）：{e:?}"))
        }
        Err(e) if matches!(e, RouteError::LoopbackUnavailable { .. }) => blocked(&format!(
            "环回不可用（E_LOOPBACK_UNAVAILABLE，§3.5 环境缺失）：{e:?}"
        )),
        Err(e) => panic!("start_audio_route 应成功（得到 {e:?}）"),
    }
}

// ─────────────────────────── 杂项 ───────────────────────────

fn fresh_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr_m6_red_audio_m6_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("创建临时目录失败");
    d
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn audio_rms(x: &[f32]) -> f64 {
    let s: f64 = x.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / x.len() as f64;
    s.sqrt()
}

/// 递归收集 dir 下指定扩展名文件（含子目录——崩溃残留可能位于会话临时子目录）。
fn collect_files_recursive(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p
                .extension()
                .map_or(false, |x| x.to_string_lossy().eq_ignore_ascii_case(ext))
            {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// 捕获结果公共断言（REQ-18 管线格式 + 时长按请求值钉齐，M4 同款）。
fn assert_captured_pcm(cap: &CapturedPcm, expect_secs: u64, label: &str) {
    assert_eq!(
        cap.sample_rate, 48_000,
        "REQ-18：{label} 管线格式应为 48kHz"
    );
    assert_eq!(cap.channels, 2, "REQ-18：{label} 管线格式应为立体声");
    let frames = cap.samples.len() / 2;
    let expect = expect_secs as usize * 48_000;
    assert!(
        (frames as i64 - expect as i64).abs() <= 48,
        "REQ-18：{label} 静音补齐应把时长钉到请求值（帧数 {frames} vs 期望 {expect}，容差 48 帧/1ms）"
    );
}

// ─────────────────────────── 崩溃录制子进程（REQ-19 头部修复场景） ───────────────────────────

/// 子进程模式：启动纯录音（App 音源）后保持运行，直至被父进程强杀——模拟进程级
/// 崩溃（header 未封口的 WAV 残留 = `repair_wav` 的目标场景）。启动结果经 marker
/// 文件回传（stdout 重定向到 child.log，仅作诊断）。
fn run_wav_child() -> ! {
    let dir = PathBuf::from(std::env::var(CHILD_DIR_ENV).expect("子进程需 CHILD_DIR"));
    let src_pid: u32 = std::env::var(CHILD_PID_ENV)
        .expect("子进程需 CHILD_PID")
        .parse()
        .expect("CHILD_PID 应为整数");
    let src_created: u64 = std::env::var(CHILD_CREATED_ENV)
        .expect("子进程需 CHILD_CREATED")
        .parse()
        .expect("CHILD_CREATED 应为整数");
    let started_ms = now_unix_ms();
    let _keep_alive = match start_audio_only(AudioOnlyConfig {
        sources: vec![AudioSourceSpec::App {
            pid: src_pid,
            created_at_unix_ms: src_created,
        }],
        output_dir: dir.clone(),
    }) {
        Ok(h) => h,
        Err(e) => {
            let _ = std::fs::write(dir.join(CHILD_ERROR_FILE), format!("start_error={e:?}\n"));
            println!("CHILD-ERROR {e:?}");
            std::process::exit(2);
        }
    };
    std::fs::write(
        dir.join(CHILD_STARTED_FILE),
        format!("started_unix_ms={started_ms}\n"),
    )
    .expect("写 child.started 失败");
    println!("CHILD-STARTED started_unix_ms={started_ms}");
    // 保持录制直至被强杀（不 request_stop：崩溃语义）
    loop {
        std::thread::sleep(Duration::from_millis(500));
    }
}

// ─────────────────────────── 用例 ───────────────────────────

/// REQ-15：系统音频 = 默认渲染端点设备环回（v1 仅默认端点；系统轨为整机混合，含
/// 各应用声音）。A(440Hz)+B(880Hz) 同播 → 设备环回同步录 6s：48k/2ch、时长钉齐；
/// **系统轨非静音 RMS**；两声均在（S3 场景 b 对照；两声电平差 ≤10dB）。
#[test]
#[ignore = "L2：需交互式会话/默认渲染端点/锁定 n8.1 ffplay（specs §4）。显式运行：cargo test -p sr-itests --test audio_m6 it_audio_device_loopback_rms -- --ignored --nocapture"]
fn it_audio_device_loopback_rms() {
    let _g = gate();
    require_interactive_session();
    let _os = record_os_build();

    let tmp = fresh_dir("devlb");
    let wav_a = gen_tone_wav(&tmp, "a440.wav", 440);
    let wav_b = gen_tone_wav(&tmp, "b880.wav", 880);
    let _pa = spawn_ffplay(&wav_a);
    let _pb = spawn_ffplay(&wav_b);
    std::thread::sleep(Duration::from_millis(1500)); // 起播

    const DUR: Duration = Duration::from_secs(6);
    let t = Instant::now();
    let cap = capture_device_loopback_ok(DUR, "系统轨（设备环回）");
    println!("[devlb] 设备环回采集耗时 {:.2}s", t.elapsed().as_secs_f64());
    assert_captured_pcm(&cap, 6, "系统轨（设备环回）");
    println!(
        "[devlb] stats：padded={} dropped={} discontinuities={}",
        cap.stats.padded_frames, cap.stats.dropped_frames, cap.stats.discontinuities
    );

    let left: Vec<f32> = cap.samples.chunks(2).map(|c| c[0]).collect();
    let mid = &left[48_000..left.len() - 48_000]; // 去首尾 1s（起播/尾包过渡）
    let rms = audio_rms(mid);
    println!("[devlb] 去首尾 1s RMS={rms:.4}（阈值 ≥{DEV_RMS_MIN}）");
    assert!(
        rms >= DEV_RMS_MIN,
        "REQ-15：系统轨（整机混音）应非静音（RMS {rms:.4} < {DEV_RMS_MIN}；数字静音 ≈1e-4 量级）"
    );

    let b = bands_of(mid, 48_000);
    println!(
        "[devlb] 440={:.1}dB 880={:.1}dB 本底={:.1}dB",
        b.t440, b.t880, b.floor
    );
    assert!(
        b.t440 - b.floor >= TONE_PRESENT_DB,
        "REQ-15：系统轨应含 440Hz 应用音（整机混合，S3 场景 b 对照；差 {:.1}dB）",
        b.t440 - b.floor
    );
    assert!(
        b.t880 - b.floor >= TONE_PRESENT_DB,
        "REQ-15：系统轨应含 880Hz 应用音（差 {:.1}dB）",
        b.t880 - b.floor
    );
    assert!(
        (b.t440 - b.t880).abs() <= MIX_PARITY_DB,
        "REQ-15/S3 对照：整机混音两声电平相当（S3 实测差 0.62dB；得到 {:.1}dB）",
        b.t440 - b.t880
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// REQ-16：麦克风 = 设备枚举与逐路选择；**无设备时明确不可用（SKIP，不得静默）**。
/// 本用例：枚举采集设备 → 无设备时反证「设备不存在/不可打开必须显式报
/// `MicUnavailable`（不得静默成功——S3 phantom 教训同族）」并记 BLOCKED；设备存在时
/// 以默认设备录 6s（48k/2ch/时长钉齐），1200Hz 经默认输出播放（§4 手法：麦克风=
/// 1200Hz）要求声学回路捕获含该音调；捕获为数字静音级 ⇒ 无声学回路（如耳机）⇒
/// 环境缺失 BLOCKED（本机默认渲染/采集均为蓝牙耳机，见红报告 §6 设备状况）。
#[test]
#[ignore = "L2：需交互式会话 + 采集（麦克风）设备 + 扬声器—麦克风声学路径（耳机环境 → BLOCKED）。显式运行：cargo test -p sr-itests --test audio_m6 it_mic_capture_present_or_skip -- --ignored --nocapture"]
fn it_mic_capture_present_or_skip() {
    let _g = gate();
    require_interactive_session();
    let _os = record_os_build();

    let devices: Vec<AudioDeviceInfo> = match list_capture_devices() {
        Ok(d) => d,
        Err(e) => blocked(&format!(
            "无法枚举采集（麦克风）设备（{e:?}；§3.5 环境缺失）"
        )),
    };
    println!(
        "[mic] 采集设备 {} 个：{:?}",
        devices.len(),
        devices
            .iter()
            .map(|d| format!("{}（default={}）", d.name, d.is_default))
            .collect::<Vec<_>>()
    );

    if devices.is_empty() {
        // REQ-16：无设备时明确不可用（SKIP，不得静默）——反证「不得静默成功」：
        // 设备不存在/不可打开必须显式报错且快速返回（phantom PID 静默零包教训同族）。
        let t = Instant::now();
        match capture_microphone("sr-m6-no-such-device", Duration::from_secs(1)) {
            Err(CaptureError::MicUnavailable { .. }) => {
                println!(
                    "[mic] 显式不可用语义成立（{:.0}ms，MicUnavailable）",
                    t.elapsed().as_secs_f64() * 1e3
                )
            }
            Err(other) => panic!("REQ-16：无设备/设备不存在应报 MicUnavailable（得到 {other:?}）"),
            Ok(_) => panic!("REQ-16：无设备时不得静默成功返回（零包/静音；SKIP 不得静默）"),
        }
        assert!(
            t.elapsed() <= Duration::from_secs(5),
            "REQ-16：不可用应快速返回（耗时 {:?}）",
            t.elapsed()
        );
        blocked("本机当前无麦克风设备（list_capture_devices 为空；S3 §1 曾实测无输入端点）——REQ-16 正路径（设备存在 → 1200Hz 声学捕获）在本机记 BLOCKED（SKIP 不算通过，spec §4）");
    }

    // 设备存在：默认（或首个）设备 + 扬声器播放 1200Hz → 声学回路捕获
    let dev = devices.iter().find(|d| d.is_default).unwrap_or(&devices[0]);
    println!("[mic] 选用设备 {}（id={}）", dev.name, dev.id);
    let tmp = fresh_dir("mic");
    let wav = gen_tone_wav(&tmp, "m1200.wav", 1200);
    let _player = spawn_ffplay(&wav);
    std::thread::sleep(Duration::from_millis(1500)); // 起播

    const DUR: Duration = Duration::from_secs(6);
    let cap = match capture_microphone(&dev.id, DUR) {
        Ok(c) => c,
        Err(e) if matches!(e, CaptureError::MicUnavailable { .. }) => blocked(&format!(
            "默认麦克风设备在枚举后不可打开（明确不可用；REQ-16）：{e:?}"
        )),
        Err(e) => panic!("REQ-16：设备存在时麦克风捕获应成功（得到 {e:?}）"),
    };
    assert_captured_pcm(&cap, 6, "麦克风");
    let mono: Vec<f32> = cap.samples.chunks(2).map(|c| c[0]).collect();
    assert!(
        mono.len() > 3 * 48_000,
        "麦克风样本应 ≥3s（得到 {} 帧）",
        mono.len()
    );
    let mid = &mono[48_000..mono.len() - 48_000];
    let rms = audio_rms(mid);
    let spec = avg_spectrum(mid, 48_000);
    let p1200 = spec.band_peak_rel_db(1200.0, 5.0);
    let floor = spec.band_median_rel_db(100.0, 6000.0, &[(1190.0, 1210.0)]);
    println!(
        "[mic] 去首尾 1s RMS={rms:.6}；FFT 1200Hz={p1200:.1}dB 本底={floor:.1}dB ⇒ 差 {:.1}dB（阈值 ≥{TONE_PRESENT_DB}）",
        p1200 - floor
    );
    println!(
        "[mic] stats：padded={} dropped={} discontinuities={}",
        cap.stats.padded_frames, cap.stats.dropped_frames, cap.stats.discontinuities
    );
    if p1200 - floor < TONE_PRESENT_DB {
        // 内容级验证的前提 = 扬声器—麦克风声学路径（§4 手法）。默认渲染/采集端点若为
        // 同一耳机（或麦克风静音），捕获为数字静音级 ⇒ 「环境缺失」分支（§4：BLOCKED，
        // SKIP 不算通过），须在扬声器—麦克风耦合的机器复跑；有信号但无目标音调才是失败。
        if rms < SILENCE_RMS {
            blocked(&format!(
                "麦克风捕获正常但未拾到 1200Hz（RMS={rms:.6} < {SILENCE_RMS}，数字静音级）：默认渲染/采集端点无声学回路（耳机/静音）——REQ-16 内容级验证环境缺失（§4），留扬声器—麦克风耦合机器复跑"
            ));
        }
        panic!(
            "REQ-16：麦克风应拾到 1200Hz（RMS={rms:.6} 非静音但无目标音调；差 {:.1}dB < {TONE_PRESENT_DB}dB）",
            p1200 - floor
        );
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

/// REQ-19：纯录音——1..N 来源 → 各来源**独立 WAV（48kHz s16）**；**停止即成品**；
/// 崩溃恢复 = 头部重写 + 尾部不完整样本裁切。
/// 相位 A（正常停止）：App(440)+App(880) 双源 → 2 条 WAV（与 sources 同序）、时长≈
/// 录制段、各自内容隔离（各自只含自身音调，非目标 ≤ −40dB）、非静音。
/// 相位 B（进程级崩溃）：以本测试二进制再入启动「崩溃录制子进程」→ 强杀（taskkill）→
/// 残留头部未封口 WAV → 注入 2 字节尾部余量（确定性覆盖「尾部不完整样本裁切」）→
/// `repair_wav` → 时长≈子进程存活段、帧对齐、s16/48k/2ch、可解码、非静音、幂等。
#[test]
#[ignore = "L2：需交互式会话/默认渲染端点/锁定 n8.1（specs §4）。显式运行：cargo test -p sr-itests --test audio_m6 it_audio_only_wav_duration_and_repair -- --ignored --nocapture"]
fn it_audio_only_wav_duration_and_repair() {
    // 子进程再入模式（崩溃源）：先于 GATE/会话检查（父进程启动它并强杀）。
    if std::env::var(CHILD_DIR_ENV).is_ok() {
        run_wav_child();
    }

    let _g = gate();
    require_interactive_session();
    let _os = record_os_build();

    let tmp = fresh_dir("wav");
    let wav_a = gen_tone_wav(&tmp, "a440.wav", 440);
    let wav_b = gen_tone_wav(&tmp, "b880.wav", 880);
    let player_a = spawn_ffplay(&wav_a);
    let _player_b = spawn_ffplay(&wav_b);
    std::thread::sleep(Duration::from_millis(1500)); // 起播

    let pid_a = player_a.0.id();
    let pid_b = _player_b.0.id();
    let created_a = process_created_unix_ms(pid_a)
        .unwrap_or_else(|| blocked("无法读取 ffplay A 创建时间（身份地面真值）"));
    let created_b = process_created_unix_ms(pid_b)
        .unwrap_or_else(|| blocked("无法读取 ffplay B 创建时间（身份地面真值）"));

    // ── 相位 A：正常停止（停止即成品）──
    let out_clean = tmp.join("clean");
    std::fs::create_dir_all(&out_clean).expect("创建输出目录失败");
    let h = start_audio_only_ok(AudioOnlyConfig {
        sources: vec![
            AudioSourceSpec::App {
                pid: pid_a,
                created_at_unix_ms: created_a,
            },
            AudioSourceSpec::App {
                pid: pid_b,
                created_at_unix_ms: created_b,
            },
        ],
        output_dir: out_clean.clone(),
    });
    std::thread::sleep(Duration::from_millis(200));
    let t_rec = Instant::now();
    std::thread::sleep(Duration::from_secs(5));
    h.request_stop();
    let span = t_rec.elapsed().as_secs_f64();
    let t_fin = Instant::now();
    let fin = h
        .wait_finalized(Duration::from_secs(15))
        .expect("REQ-19：停止即成品（WAV 收尾应即时完成；实现若引入后台 concat 属契约违背）");
    println!(
        "[wav] 停止→成品 {:.2}s（录制段 {span:.3}s）",
        t_fin.elapsed().as_secs_f64()
    );

    assert_eq!(
        fin.wav_paths.len(),
        2,
        "REQ-19：1..N 来源 → 各来源独立 WAV（wav_paths 与 sources 同序同长；得到 {:?}）",
        fin.wav_paths
    );
    let dir_canon = out_clean.canonicalize().expect("canonicalize 输出目录");
    for (i, p) in fin.wav_paths.iter().enumerate() {
        assert!(p.exists(), "REQ-19：WAV[{i}] 应存在（{}）", p.display());
        assert!(
            p.extension()
                .map_or(false, |e| e.to_string_lossy().eq_ignore_ascii_case("wav")),
            "REQ-19：WAV[{i}] 应为 .wav（{}）",
            p.display()
        );
        let pc = p
            .canonicalize()
            .unwrap_or_else(|e| panic!("canonicalize WAV[{i}] 失败：{e}"));
        assert!(
            pc.starts_with(&dir_canon),
            "REQ-19：WAV[{i}] 应位于输出目录内（{}）",
            p.display()
        );
    }

    // wav_paths 与 sources 同序：源 0 = 440、源 1 = 880（内容交叉验证）
    for (p, target_hz, other_hz, label) in [
        (&fin.wav_paths[0], 440.0f64, 880.0f64, "wav-440（源0）"),
        (&fin.wav_paths[1], 880.0, 440.0, "wav-880（源1）"),
    ] {
        let (codec, ar, ch) = probe_audio_stream_at(p, 0);
        assert_eq!(
            codec, "pcm_s16le",
            "REQ-19：WAV 编码应为 s16（得到 {codec}）"
        );
        assert_eq!(ar, 48_000, "REQ-19：WAV 应 48kHz");
        assert_eq!(ch, 2, "REQ-18/19：WAV 应为立体声（管线固定格式）");
        let dur = probe_duration(p);
        println!("[wav] {label} 时长 {dur:.3}s（录制段 {span:.3}s）");
        assert!(
            (dur - span).abs() <= 1.0,
            "REQ-19：WAV 时长应≈录制段（{dur:.3}s vs {span:.3}s，容差 1s）"
        );
        assert!(
            decode_clean(p),
            "REQ-19：{label} 应整文件可解码（停止即成品）"
        );
        let mono = decode_audio_mono(p);
        assert!(
            mono.len() > 48_000 * 3,
            "REQ-19：{label} 样本应 ≥3s（得到 {} 帧）",
            mono.len()
        );
        let rms = audio_rms(&mono);
        println!("[wav] {label} RMS={rms:.4}");
        assert!(
            rms >= WAV_RMS_MIN,
            "REQ-19：{label} 应非静音（RMS {rms:.4} < {WAV_RMS_MIN}）"
        );
        assert_tone_isolated(&mono, 48_000, target_hz, other_hz, label);
    }

    // ── 相位 B：进程级崩溃（强杀录制子进程）→ repair_wav ──
    let out_crash = fresh_dir("wav-crash");
    let exe = std::env::current_exe().expect("current_exe");
    let child_log = std::fs::File::create(out_crash.join("child.log")).expect("创建子进程日志失败");
    let child_log2 = child_log.try_clone().expect("clone 子进程日志句柄失败");
    let mut child = Command::new(&exe)
        .args([
            "--exact",
            "it_audio_only_wav_duration_and_repair",
            "--ignored",
            "--nocapture",
        ])
        .env(CHILD_DIR_ENV, &out_crash)
        .env(CHILD_PID_ENV, pid_a.to_string())
        .env(CHILD_CREATED_ENV, created_a.to_string())
        .stdout(Stdio::from(child_log))
        .stderr(Stdio::from(child_log2))
        .spawn()
        .unwrap_or_else(|e| panic!("无法 spawn 录制子进程（{}）：{e}", exe.display()));
    println!(
        "[wav-crash] 崩溃录制子进程 pid={}（App 440Hz 源）",
        child.id()
    );

    let started_ms = wait_until(Duration::from_secs(15), Duration::from_millis(200), || {
        std::fs::read_to_string(out_crash.join(CHILD_STARTED_FILE))
            .ok()
            .and_then(|s| {
                s.lines()
                    .find_map(|l| l.strip_prefix("started_unix_ms="))
                    .and_then(|v| v.trim().parse::<u64>().ok())
            })
    });
    let Some(started_ms) = started_ms else {
        let err = std::fs::read_to_string(out_crash.join(CHILD_ERROR_FILE)).unwrap_or_default();
        let log = std::fs::read_to_string(out_crash.join("child.log")).unwrap_or_default();
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = child.wait();
        if err.contains("LoopbackUnavailable") {
            blocked(&format!(
                "子进程纯录音启动时环回不可用（E_LOOPBACK_UNAVAILABLE，§3.5 环境缺失）：{err}"
            ));
        }
        panic!("崩溃录制子进程未在 15s 内就绪（{CHILD_STARTED_FILE} 未出现）；child.error={err:?}；child.log={log}");
    };
    println!("[wav-crash] 子进程就绪于 unix_ms={started_ms}，继续录 2.5s 后强杀（模拟进程级崩溃）");
    std::thread::sleep(Duration::from_millis(2500));
    let st = Command::new("taskkill")
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("spawn taskkill");
    assert!(
        st.success(),
        "taskkill /PID {} /T /F 失败：{st}",
        child.id()
    );
    let _ = child.wait();
    // 存活段在强杀**生效完成**后测量：taskkill 有实际生效延迟（scratch 实测 ≈0.8–1.0s），
    // 以请求前时点计算会把该延迟算进 WAV 时长（scratch 实测 delta 1.01s）；完成后测量把
    // 「WAV 时长 ≈ 存活段」收敛到写入节拍粒度。
    let kill_ms = now_unix_ms();

    let wavs: Vec<PathBuf> = collect_files_recursive(&out_crash, "wav")
        .into_iter()
        .filter(|p| std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false))
        .collect();
    assert_eq!(
        wavs.len(),
        1,
        "REQ-19：崩溃子进程应留下恰 1 个半成品 WAV（得到 {wavs:?}）"
    );
    let wav = &wavs[0];
    let size = std::fs::metadata(wav).expect("WAV 元数据").len();
    assert!(
        size > 44,
        "REQ-19：崩溃残留 WAV 应已有数据（{size} 字节 = 头部 + 样本）"
    );
    // 确定性覆盖「尾部不完整样本」：把文件尾部余量定为 2 字节（s16 立体声帧=4 字节）。
    // taskkill 时点随机、天然可能恰好帧对齐（scratch 实测 1920 字节/块写入恒对齐），
    // 按 size%4 注入 0..3 字节余量使裁切分支必被触达。
    let rem = (size % 4) as usize;
    let inject = (2 + 4 - rem) % 4;
    if inject > 0 {
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(wav)
            .expect("打开残留 WAV 追加");
        f.write_all(&vec![0xABu8; inject])
            .expect("写入尾部不完整样本注入");
        f.flush().expect("flush 注入");
    }
    let size_tail = std::fs::metadata(wav).expect("WAV 元数据").len();
    assert_eq!(
        size_tail % 4,
        2,
        "注入后尾部余量应为 2 字节（得到 {size_tail}）"
    );
    println!(
        "[wav-crash] 强杀后残留 {size} 字节（注入 {inject} 字节尾部余量）；修复前时长探测={:?}（头部未封口，诊断用）",
        probe_duration_soft(wav)
    );

    let rep =
        repair_wav(wav).unwrap_or_else(|e| panic!("REQ-19：崩溃残留 WAV 应可修复（得到 {e:?}）"));
    let size_rep = std::fs::metadata(wav).expect("修复后 WAV 元数据").len();
    println!(
        "[wav-crash] repair：frames={} trimmed_bytes={}（文件 {size_tail} → {size_rep} 字节）",
        rep.frames, rep.trimmed_bytes
    );
    assert_eq!(
        rep.trimmed_bytes, 2,
        "REQ-19：应裁切 2 字节尾部不完整样本（得到 {}）",
        rep.trimmed_bytes
    );
    assert_eq!(
        size_rep,
        size_tail - 2,
        "REQ-19：裁切应反映于文件（{size_tail} → {size_rep}）"
    );
    assert_eq!(
        size_rep % 4,
        0,
        "REQ-19：修复后应帧对齐（s16 立体声帧=4 字节；得到 {size_rep} 字节）"
    );
    // 幂等：再次修复应为无操作（已封口文件不被再次改动）
    let rep2 = repair_wav(wav).unwrap_or_else(|e| panic!("REQ-19：修复应幂等（二跑得到 {e:?}）"));
    assert_eq!(
        rep2.trimmed_bytes, 0,
        "REQ-19：二次修复应无裁切（得到 {}）",
        rep2.trimmed_bytes
    );
    assert_eq!(
        std::fs::metadata(wav).expect("WAV 元数据").len(),
        size_rep,
        "REQ-19：二次修复不应改变文件"
    );

    let (codec, ar, ch) = probe_audio_stream_at(wav, 0);
    assert_eq!(
        codec, "pcm_s16le",
        "REQ-19：修复后 WAV 应为 s16（得到 {codec}）"
    );
    assert_eq!(ar, 48_000, "REQ-19：修复后 WAV 应 48kHz");
    assert_eq!(ch, 2, "REQ-19：修复后 WAV 应为立体声");
    let dur = probe_duration(wav);
    let alive = (kill_ms.saturating_sub(started_ms)) as f64 / 1000.0;
    println!("[wav-crash] 修复后时长 {dur:.3}s vs 子进程存活段 {alive:.3}s（容差 1.5s）");
    assert!(
        (dur - alive).abs() <= 1.5,
        "REQ-19：修复后时长应≈崩溃前存活段（{dur:.3}s vs {alive:.3}s；头部重写 + 尾部裁切）"
    );
    let rep_secs = rep.frames as f64 / 48_000.0;
    println!("[wav-crash] repair.frames 折算 {rep_secs:.3}s（应与 ffprobe 一致 ±0.1s）");
    assert!(
        (rep_secs - dur).abs() <= 0.1,
        "REQ-19：repair 记账应与文件实际时长一致（{rep_secs:.3}s vs {dur:.3}s）"
    );
    assert!(decode_clean(wav), "REQ-19：修复后应整文件可解码");
    let mono = decode_audio_mono(wav);
    assert!(
        mono.len() > 48_000,
        "REQ-19：修复后样本应 ≥1s（得到 {} 帧）",
        mono.len()
    );
    let rms = audio_rms(&mono);
    println!("[wav-crash] 修复后 RMS={rms:.4}");
    assert!(
        rms >= WAV_RMS_MIN,
        "REQ-19：修复后音频应非静音（崩溃前 440Hz 在播；RMS {rms:.4}）"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// REQ-17 全音轨模式（spec §4 `it_audio_multitrack_ffprobe_fft` 的全音轨部分姊妹用例；
/// 隔离模式部分在 audio_isolation.rs，未改动）：目标应用 + 系统 + 追加应用（含全选），
/// 各独立轨、互不混音。音源 = [App(440), System, App(880)]（目标窗口为测试自建窗口、
/// 无天然应用音源）⇒ 成品 MKV 恰 3 条音轨：应用 A 轨只含 440、应用 B 轨只含 880、
/// 系统轨含两声（S3 场景 b 对照：两电平差 ≤10dB）——轨组织与内容双证「互不混音」。
/// 未启用麦克风 ⇒ 无 mic 轨；路由侧 Mic 轨（AudioSourceSpec::Mic）内容级验证需
/// 扬声器—麦克风耦合环境，本机为蓝牙耳机双端点，未纳入本用例（见红报告 §8）。
#[test]
#[ignore = "L2：需交互式会话/WGC/可用编码器/默认渲染端点（specs §4）。显式运行：cargo test -p sr-itests --test audio_m6 it_audio_multitrack_ffprobe_fft_all_tracks -- --ignored --nocapture"]
fn it_audio_multitrack_ffprobe_fft_all_tracks() {
    let _g = gate();
    require_interactive_session();
    let _os = record_os_build();

    let tmp = fresh_dir("alltracks");
    let out_dir = tmp.join("out");
    std::fs::create_dir_all(&out_dir).expect("创建输出目录失败");
    let wav_a = gen_tone_wav(&tmp, "a440.wav", 440);
    let wav_b = gen_tone_wav(&tmp, "b880.wav", 880);
    let player_a = spawn_ffplay(&wav_a);
    let _player_b = spawn_ffplay(&wav_b);
    std::thread::sleep(Duration::from_millis(1500)); // 起播

    let pid_a = player_a.0.id();
    let pid_b = _player_b.0.id();
    let created_a = process_created_unix_ms(pid_a)
        .unwrap_or_else(|| blocked("无法读取 ffplay A 创建时间（身份地面真值）"));
    let created_b = process_created_unix_ms(pid_b)
        .unwrap_or_else(|| blocked("无法读取 ffplay B 创建时间（身份地面真值）"));

    let probe = ProbeWindow::new(&format!("SR-M6-RED-alltracks-{}", std::process::id()));
    std::thread::sleep(Duration::from_millis(300));

    let h = start_audio_route_ok(AudioRouteConfig {
        target: RouteTarget::Window {
            hwnd: probe.hwnd_u64,
        },
        video: VideoCfg {
            canvas_w: 640,
            canvas_h: 360,
            fps: 30,
            cursor: true,
        },
        audio: vec![
            AudioSourceSpec::App {
                pid: pid_a,
                created_at_unix_ms: created_a,
            },
            AudioSourceSpec::System,
            AudioSourceSpec::App {
                pid: pid_b,
                created_at_unix_ms: created_b,
            },
        ],
        output_dir: out_dir.clone(),
    });
    std::thread::sleep(Duration::from_millis(300));
    let t_rec = Instant::now();
    std::thread::sleep(Duration::from_secs(8));
    h.request_stop();
    let span = t_rec.elapsed().as_secs_f64();
    let fin = h
        .wait_finalized(Duration::from_secs(120))
        .expect("REQ-17：全音轨路由停止后收尾应成功（EOF 优雅停止 + 后台 concat）");

    assert!(
        fin.final_path.exists(),
        "成品应存在（{}）",
        fin.final_path.display()
    );
    assert!(
        fin.final_path
            .extension()
            .map_or(false, |e| e.to_string_lossy() == "mkv"),
        "成品应为 MKV（{}）",
        fin.final_path.display()
    );

    // ── 轨道形状：1 视频 + 恰 3 独立音轨（应用×2 + 系统；无 mic 源 ⇒ 无 mic 轨）──
    let (vcodec, w, hgt) = probe_video_stream(&fin.final_path);
    assert_eq!(vcodec, "h264", "T6：v1 仅 H.264");
    assert_eq!((w, hgt), (640, 360), "D6：固定画布 640x360");
    assert_eq!(
        probe_stream_count(&fin.final_path, "v"),
        1,
        "应恰 1 条视频轨"
    );
    assert_eq!(
        probe_stream_count(&fin.final_path, "a"),
        3,
        "REQ-17 全音轨：应用×2 + 系统源 ⇒ 恰 3 条独立音轨（未启用麦克风 ⇒ 无 mic 轨）"
    );
    for i in 0..3 {
        let (acodec, ar, ch) = probe_audio_stream_at(&fin.final_path, i);
        assert_eq!(acodec, "aac", "§3.3 基准：音频编码 aac（音轨 {i}）");
        assert_eq!(ar, 48_000, "REQ-18：音轨 {i} 应 48kHz");
        assert_eq!(ch, 2, "REQ-18：音轨 {i} 应为立体声");
    }
    let dur = probe_duration(&fin.final_path);
    println!("[alltracks] 时长 {dur:.3}s（录制段 {span:.3}s）");
    assert!(
        (dur - span).abs() <= 2.0,
        "REQ-17：成品时长应≈录制段（{dur:.3}s vs {span:.3}s）"
    );

    // ── 内容级分类（不依赖轨序）：系统轨含两声；应用轨各自只含自身音调（≤ −40dB）──
    let mut system_idx: Vec<usize> = Vec::new();
    let mut app440_idx: Vec<usize> = Vec::new();
    let mut app880_idx: Vec<usize> = Vec::new();
    for i in 0..3 {
        let mono = decode_audio_track_mono(&fin.final_path, i);
        assert!(
            mono.len() > 48_000 * 4,
            "音轨 {i} 样本应 ≥4s（得到 {} 帧；音轨缺失/过短？）",
            mono.len()
        );
        let b = bands_of(&mono, 48_000);
        let has440 = b.t440 - b.floor >= TONE_PRESENT_DB;
        let has880 = b.t880 - b.floor >= TONE_PRESENT_DB;
        println!(
            "[alltracks] 音轨 {i}：440={:.1}dB 880={:.1}dB 本底={:.1}dB ⇒ 440在={has440} 880在={has880}",
            b.t440, b.t880, b.floor
        );
        match (has440, has880) {
            (true, true) => {
                assert!(
                    (b.t440 - b.t880).abs() <= MIX_PARITY_DB,
                    "REQ-15/17：系统轨（整机混音）应同时含两声且电平相当（S3 对照实测差 0.62dB；得到 {:.1}dB）",
                    b.t440 - b.t880
                );
                system_idx.push(i);
            }
            (true, false) => {
                assert!(
                    b.t880 - b.t440 <= ISOLATION_DB,
                    "REQ-17：应用轨（440）非目标 880 泄漏应 ≤ −40dB（相对目标 {:.1}dB；S3 预锁定阈值）",
                    b.t880 - b.t440
                );
                app440_idx.push(i);
            }
            (false, true) => {
                assert!(
                    b.t440 - b.t880 <= ISOLATION_DB,
                    "REQ-17：追加应用轨（880）非目标 440 泄漏应 ≤ −40dB（相对目标 {:.1}dB）",
                    b.t440 - b.t880
                );
                app880_idx.push(i);
            }
            (false, false) => panic!(
                "REQ-17：音轨 {i} 既不含量身音调（440/880 均低于本底+{TONE_PRESENT_DB}dB）：静音/空轨？"
            ),
        }
    }
    assert_eq!(
        (system_idx.len(), app440_idx.len(), app880_idx.len()),
        (1, 1, 1),
        "REQ-17 全音轨：应恰 1 系统轨 + 1 目标应用轨 + 1 追加应用轨、互不混音（分类得到 system={system_idx:?} app440={app440_idx:?} app880={app880_idx:?}）"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
