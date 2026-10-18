//! M15/REQ-3/4/8 L2：输出采样率端到端（MKV 音轨 + 纯录音 WAV）。
//!
//! 覆盖 `it_route_sample_rate_{44100,88200,96000}` 与
//! `it_wav_sample_rate_{44100,88200,96000}`：
//! - 成品经 ffprobe 断言**所有**音轨实际采样率 = 所选值；
//! - 时长与录制段一致；
//! - 已知 440Hz 音调 FFT 检出（防止「改标签未转换」假通过：若 PCM 未按输出率重采样，
//!   以标签声明的采样率解码后音调会按比例偏移，440Hz 频带检不出）。
//!
//! 环境缺失（默认渲染端点/编码器/WGC）→ BLOCKED（fail），不静默 skip（§4）。

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rustfft::{num_complex::Complex, FftPlanner};

use common::*;
use sr_audio::wasapi::AudioSourceSpec;
use sr_pipeline::route::{
    start_audio_only_with_options_and_rate, start_routes_with_options_and_rate, AudioOnlyConfig,
    AudioOnlyHandle, MultiRouteConfig, RouteError, RouteHandle, RouteTarget, VideoCfg,
};

/// 同一测试二进制内所有用例串行（ffplay/采集/WGC/编码器叠加会超出已验证并发面）。
static GATE: Mutex<()> = Mutex::new(());

const TONE_HZ: f64 = 440.0;
const TONE_PRESENT_DB: f64 = 20.0;
const NFFT: usize = 32768;

fn ffbin(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin")
        .join(name)
        .canonicalize()
        .unwrap_or_else(|e| {
            blocked(&format!(
                "无法定位锁定 n8.1 构建的 {name}.exe（canonicalize 失败：{e}）"
            ))
        });
    if !p.exists() {
        blocked(&format!("锁定 ffmpeg n8.1 构建缺失：{}", p.display()));
    }
    p
}

fn fresh_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr_m15_sample_rate_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("创建临时目录失败");
    d
}

/// 40s 正弦 WAV（48k/立体声/0.25 音量；ffplay 播放供进程环回采集）。
fn gen_tone_wav(dir: &Path) -> PathBuf {
    let out = dir.join("tone440.wav");
    let st = Command::new(ffbin("ffmpeg.exe"))
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=40,volume=0.25",
            "-ac",
            "2",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(&out)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap_or_else(|e| blocked(&format!("无法运行锁定 ffmpeg 生成测试音：{e}")));
    if !st.success() {
        blocked(&format!("ffmpeg 生成 440Hz 测试音失败（exit={st}）"));
    }
    out
}

struct Player(Child);
impl Drop for Player {
    fn drop(&mut self) {
        let _ = Command::new("taskkill")
            .args(["/PID", &self.0.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = self.0.wait();
    }
}

fn spawn_ffplay(wav: &Path) -> Player {
    let child = Command::new(ffbin("ffplay.exe"))
        .args(["-nodisp", "-autoexit", "-loglevel", "quiet"])
        .arg(wav)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn 锁定 ffplay：{e}")));
    Player(child)
}

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
    ffprobe_csv(
        path,
        &["-show_entries", "format=duration", "-of", "csv=p=0"],
    )
    .parse()
    .ok()
    .filter(|d| *d > 0.0)
    .unwrap_or_else(|| panic!("应能读出时长（{} 损坏/未封口？）", path.display()))
}

/// 全部音频流 (codec, sample_rate, channels)（**所有音轨**，REQ-4）。
fn probe_all_audio_streams(path: &Path) -> Vec<(String, u32, u32)> {
    let s = ffprobe_csv(
        path,
        &[
            "-select_streams",
            "a",
            "-show_entries",
            "stream=codec_name,sample_rate,channels",
            "-of",
            "csv=p=0",
        ],
    );
    let mut out = Vec::new();
    for line in s.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let parts: Vec<&str> = line.split(',').collect();
        assert_eq!(parts.len(), 3, "音频流 csv 形状（得到 {line:?}）");
        out.push((
            parts[0].to_string(),
            parts[1].parse().expect("sample_rate"),
            parts[2].parse().expect("channels"),
        ));
    }
    out
}

/// 解码音轨 idx 到指定采样率单声道 f32。
fn decode_audio_mono_at(path: &Path, idx: usize, rate: u32) -> Vec<f32> {
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
            &rate.to_string(),
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

/// 32768 点 Hann 窗、hop 半帧、帧间平均功率谱。
fn avg_spectrum(x: &[f32], sample_rate: u32) -> (f64, Vec<f64>) {
    use std::f64::consts::PI;
    assert!(
        x.len() > NFFT * 2,
        "可分析样本过短（{} < 2×{NFFT}）",
        x.len()
    );
    let hop = NFFT / 2;
    let head = (0.5 * sample_rate as f64) as usize;
    let tail = x.len().saturating_sub((0.2 * sample_rate as f64) as usize);
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
        for (k, v) in buf.iter().enumerate() {
            acc[k] += v.norm_sqr();
        }
        frames += 1;
        start += hop;
    }
    assert!(frames >= 2, "可分析帧数不足（{frames}）");
    for a in &mut acc {
        *a /= frames as f64;
    }
    (sample_rate as f64 / NFFT as f64, acc)
}

/// 440Hz 频带峰值相对本底 ≥20dB（REQ-4 已知音调 FFT）。
fn assert_tone_present(x: &[f32], rate: u32, label: &str) {
    let (bin_hz, power) = avg_spectrum(x, rate);
    let band_peak = |hz: f64, half_bw: f64| -> f64 {
        let k0 = (((hz - half_bw) / bin_hz).floor() as usize).max(1);
        let k1 = (((hz + half_bw) / bin_hz).ceil() as usize).min(power.len() / 2);
        let p = (k0..=k1).fold(0.0f64, |m, k| m.max(power[k]));
        10.0 * p.log10()
    };
    let mut floor_samples: Vec<f64> = (1..power.len() / 2)
        .map(|k| (k, k as f64 * bin_hz))
        .filter(|(_, hz)| *hz >= 100.0 && *hz <= 6000.0 && (*hz - TONE_HZ).abs() > 40.0)
        .map(|(k, _)| power[k])
        .collect();
    assert!(floor_samples.len() >= 8, "噪声底样本过少");
    floor_samples.sort_by(|a, b| a.partial_cmp(b).expect("NaN 功率"));
    let floor = 10.0 * floor_samples[floor_samples.len() / 2].log10();
    let tone = band_peak(TONE_HZ, 5.0);
    println!(
        "[fft:{label}] rate={rate}Hz tone({TONE_HZ})={tone:.1}dB floor={floor:.1}dB ⇒ {:.1}dB（阈值 ≥{TONE_PRESENT_DB}）",
        tone - floor
    );
    assert!(
        tone - floor >= TONE_PRESENT_DB,
        "REQ-4：{label} 应以声明采样率 {rate}Hz 检出 {TONE_HZ}Hz 音调（差 {:.1}dB < {TONE_PRESENT_DB}dB——疑似仅改标签未重采样）",
        tone - floor
    );
}

/// RIFF WAV 的实际帧数与 fmt 采样率（帧数 ÷ 率 = 时长，REQ-3/4）。
fn wav_frames_and_header_rate(path: &Path) -> (u64, u32) {
    let bytes = std::fs::read(path).expect("读取 WAV");
    assert!(bytes.len() >= 44, "WAV 头部不足 44 字节");
    assert_eq!(&bytes[0..4], b"RIFF", "应 RIFF");
    assert_eq!(&bytes[8..12], b"WAVE", "应 WAVE");
    let channels = u16::from_le_bytes([bytes[22], bytes[23]]) as u64;
    let rate = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
    let data_bytes = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]) as u64;
    assert!(channels > 0, "声道数应为正");
    assert_eq!(
        data_bytes as usize + 44,
        bytes.len(),
        "data 长度应覆盖文件（已封口）"
    );
    let frames = data_bytes / (channels * 2);
    (frames, rate)
}

fn start_routes_ok(cfgs: Vec<MultiRouteConfig>, rate: u32) -> Vec<RouteHandle> {
    match start_routes_with_options_and_rate(cfgs, None, false, rate) {
        Ok(h) => h,
        Err(e) if matches!(e, RouteError::EncoderUnavailable) => {
            blocked(&format!("无可用编码器：{e:?}"))
        }
        Err(e) if matches!(e, RouteError::ConsentDenied) => {
            blocked(&format!("WGC 授权被拒：{e:?}"))
        }
        Err(e) if matches!(e, RouteError::LoopbackUnavailable { .. }) => {
            blocked(&format!("环回不可用：{e:?}"))
        }
        Err(e) => panic!("启动应成功（得到 {e:?}）"),
    }
}

fn start_audio_only_ok(cfg: AudioOnlyConfig, rate: u32) -> AudioOnlyHandle {
    match start_audio_only_with_options_and_rate(cfg, false, rate) {
        Ok(h) => h,
        Err(e) if matches!(e, RouteError::LoopbackUnavailable { .. }) => {
            blocked(&format!("纯录音启动时环回不可用：{e:?}"))
        }
        Err(e) => panic!("纯录音启动应成功（得到 {e:?}）"),
    }
}

// ─────────────────────────── MKV 音轨（REQ-3/4） ───────────────────────────

fn route_sample_rate_case(rate: u32) {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let tmp = fresh_dir(&format!("route_{rate}"));
    let wav = gen_tone_wav(&tmp);
    let player = spawn_ffplay(&wav);
    std::thread::sleep(Duration::from_millis(1500));
    let pid = player.0.id();
    let created = process_created_unix_ms(pid)
        .unwrap_or_else(|| blocked("无法读取 ffplay 创建时间（身份地面真值）"));

    let win = ProbeWindow::new(&format!("SR-M15-RATE-{rate}-{}", std::process::id()));
    std::thread::sleep(Duration::from_millis(300));

    let out = tmp.join("out");
    std::fs::create_dir_all(&out).expect("创建输出目录");
    let handles = start_routes_ok(
        vec![MultiRouteConfig {
            target: RouteTarget::Window { hwnd: win.hwnd_u64 },
            video: VideoCfg {
                canvas_w: 640,
                canvas_h: 360,
                fps: 30,
                cursor: false,
            },
            audio: vec![AudioSourceSpec::App {
                pid,
                created_at_unix_ms: created,
            }],
            output_dir: out,
        }],
        rate,
    );
    std::thread::sleep(Duration::from_millis(300));
    let t_rec = Instant::now();
    std::thread::sleep(Duration::from_secs(10));
    handles[0].request_stop();
    let span = t_rec.elapsed().as_secs_f64();
    let fin = handles[0]
        .wait_finalized(Duration::from_secs(120))
        .expect("REQ-3：10s 录制收尾应成功");

    assert!(fin.final_path.exists(), "成品应存在");
    let streams = probe_all_audio_streams(&fin.final_path);
    assert!(
        !streams.is_empty(),
        "REQ-4：成品应至少含 1 条音轨（{}）",
        fin.final_path.display()
    );
    for (i, (codec, ar, ch)) in streams.iter().enumerate() {
        assert_eq!(codec, "aac", "音轨 {i} 应为 aac");
        assert_eq!(
            *ar, rate,
            "REQ-3/4：音轨 {i} 实际采样率应为所选 {rate}（得到 {ar}）"
        );
        assert_eq!(*ch, 2, "音轨 {i} 应为立体声");
    }
    let dur = probe_duration(&fin.final_path);
    println!("[route-rate-{rate}] 时长 {dur:.3}s（录制段 {span:.3}s）；音轨 {streams:?}");
    assert!(
        (dur - span).abs() <= 2.0,
        "REQ-4：成品时长应≈录制段（{dur:.3}s vs {span:.3}s）"
    );

    let mono = decode_audio_mono_at(&fin.final_path, 0, rate);
    assert_tone_present(&mono, rate, &format!("route-{rate}"));

    drop(win);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
#[ignore = "L2：需交互式会话/WGC/编码器/默认渲染端点。cargo test -p sr-itests --test sample_rate -- --ignored --nocapture --test-threads=1"]
fn it_route_sample_rate_44100() {
    route_sample_rate_case(44_100);
}

#[test]
#[ignore = "L2：需交互式会话/WGC/编码器/默认渲染端点"]
fn it_route_sample_rate_88200() {
    route_sample_rate_case(88_200);
}

#[test]
#[ignore = "L2：需交互式会话/WGC/编码器/默认渲染端点"]
fn it_route_sample_rate_96000() {
    route_sample_rate_case(96_000);
}

// ─────────────────────────── 纯录音 WAV（REQ-3/4） ───────────────────────────

fn wav_sample_rate_case(rate: u32) {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let tmp = fresh_dir(&format!("wav_{rate}"));
    let wav = gen_tone_wav(&tmp);
    let player = spawn_ffplay(&wav);
    std::thread::sleep(Duration::from_millis(1500));
    let pid = player.0.id();
    let created = process_created_unix_ms(pid)
        .unwrap_or_else(|| blocked("无法读取 ffplay 创建时间（身份地面真值）"));

    let out = tmp.join("out");
    std::fs::create_dir_all(&out).expect("创建输出目录");
    let handle = start_audio_only_ok(
        AudioOnlyConfig {
            sources: vec![AudioSourceSpec::App {
                pid,
                created_at_unix_ms: created,
            }],
            output_dir: out,
        },
        rate,
    );
    std::thread::sleep(Duration::from_millis(200));
    let t_rec = Instant::now();
    std::thread::sleep(Duration::from_secs(6));
    handle.request_stop();
    let span = t_rec.elapsed().as_secs_f64();
    let fin = handle
        .wait_finalized(Duration::from_secs(20))
        .expect("REQ-3：纯录音停止即成品");

    assert_eq!(fin.wav_paths.len(), 1);
    let path = &fin.wav_paths[0];
    let streams = probe_all_audio_streams(path);
    assert_eq!(streams.len(), 1, "WAV 应恰 1 条音轨");
    let (codec, ar, ch) = &streams[0];
    assert_eq!(codec, "pcm_s16le", "WAV 应为 s16");
    assert_eq!(
        *ar, rate,
        "REQ-3/4：WAV 实际采样率应为所选 {rate}（得到 {ar}）"
    );
    assert_eq!(*ch, 2, "WAV 应为立体声");

    let dur = probe_duration(path);
    let (frames, header_rate) = wav_frames_and_header_rate(path);
    assert_eq!(header_rate, rate, "WAV 头部采样率应为所选值");
    let by_frames = frames as f64 / header_rate as f64;
    println!(
        "[wav-rate-{rate}] 时长 {dur:.3}s；帧数 {frames} ÷ {header_rate} = {by_frames:.3}s（录制段 {span:.3}s）"
    );
    assert!(
        (by_frames - dur).abs() <= 0.05,
        "REQ-4：帧数 ÷ 率 应等于时长（{frames} ÷ {header_rate} = {by_frames:.3}s vs {dur:.3}s）"
    );
    assert!(
        (dur - span).abs() <= 1.0,
        "REQ-4：WAV 时长应≈录制段（{dur:.3}s vs {span:.3}s）"
    );

    let mono = decode_audio_mono_at(path, 0, rate);
    assert_tone_present(&mono, rate, &format!("wav-{rate}"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
#[ignore = "L2：需交互式会话/默认渲染端点/编码器"]
fn it_wav_sample_rate_44100() {
    wav_sample_rate_case(44_100);
}

#[test]
#[ignore = "L2：需交互式会话/默认渲染端点"]
fn it_wav_sample_rate_88200() {
    wav_sample_rate_case(88_200);
}

#[test]
#[ignore = "L2：需交互式会话/默认渲染端点"]
fn it_wav_sample_rate_96000() {
    wav_sample_rate_case(96_000);
}

// ─────────────────────────── 端点混音率查询（REQ-2） ───────────────────────────

/// REQ-2：`get_endpoint_mix_rate` 底层查询（MTA 线程、默认渲染端点 / 采集设备）。
/// 查询失败只返回 None（UI 显示未知）；本用例在环境存在时应能读出正采样率。
#[test]
#[ignore = "L2：需交互式会话 + 默认渲染端点（无端点 → BLOCKED）"]
fn it_endpoint_mix_rate_query() {
    require_interactive_session();
    let _os = record_os_build();
    let render = sr_audio::wasapi::query_endpoint_mix_rate(None);
    let Some(render) = render else {
        blocked("默认渲染端点混音率查询返回 None（无端点/查询失败）——REQ-2 正路径环境缺失");
    };
    assert!(render.rate > 0, "渲染端点混音率应为正（得到 {render:?}）");
    assert_eq!(render.kind, "render");
    println!(
        "[mix-rate] 渲染端点 {}：{} Hz",
        render.device_name, render.rate
    );

    match sr_audio::wasapi::list_capture_devices() {
        Ok(devices) if devices.is_empty() => {
            println!("[mix-rate] 无采集设备：麦克风端点查询正路径记 BLOCKED（环境缺失，非通过）");
        }
        Ok(devices) => {
            for d in devices {
                let mic = sr_audio::wasapi::query_endpoint_mix_rate(Some(d.id.clone()));
                let Some(mic) = mic else {
                    panic!("REQ-2：采集设备 {}（{}）混音率查询不应失败", d.name, d.id);
                };
                assert!(mic.rate > 0, "采集设备混音率应为正（{mic:?}）");
                assert_eq!(mic.kind, "capture");
                println!("[mix-rate] 采集设备 {}：{} Hz", mic.device_name, mic.rate);
            }
        }
        Err(e) => blocked(&format!("无法枚举采集设备（{e:?}）")),
    }
}
