//! P1-2(c)：两路各带**不同应用音源**的隔离 FFT（P1-2 / REQ-17 接线扩面）。
//!
//! 两个 ffplay 子进程分别播放 440/880Hz 连续音；两路各 `App{pid}` 采集其一（4:3 小画布，
//! Monitor 0 目标）。逐路解码首音轨做 FFT：目标频率存在、非目标频率相对 ≤ −40dB。

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use common::*;
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;
use sr_audio::wasapi::AudioSourceSpec;
use sr_pipeline::route::{
    start_routes_with_encoder, MultiRouteConfig, RouteError, RouteTarget, VideoCfg,
};

static GATE: Mutex<()> = Mutex::new(());

fn ffbin(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin")
        .join(name)
}

fn fresh_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr_m11_dual_audio_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("创建临时目录失败");
    d
}

fn gen_tone_wav(dir: &Path, name: &str, hz: u32) -> PathBuf {
    let path = dir.join(name);
    let out = Command::new(ffbin("ffmpeg.exe"))
        .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg(format!(
            "sine=frequency={hz}:sample_rate=48000:duration=40,volume=0.25"
        ))
        .args(["-ac", "2", "-c:a", "pcm_s16le"])
        .arg(&path)
        .output()
        .unwrap_or_else(|e| blocked(&format!("无法生成测试音：{e}")));
    assert!(
        out.status.success(),
        "生成测试音失败: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    path
}

struct Player(Child);
impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_player(wav: &Path) -> Player {
    let child = Command::new(ffbin("ffplay.exe"))
        .args(["-nodisp", "-autoexit", "-loglevel", "error"])
        .arg(wav)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn ffplay：{e}")));
    Player(child)
}

fn decode_audio_mono(path: &Path) -> Vec<f32> {
    let out = Command::new(ffbin("ffmpeg.exe"))
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args([
            "-vn",
            "-map",
            "0:a:0",
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
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|e| blocked(&format!("音轨解码失败：{e}")));
    assert!(
        out.status.success(),
        "音轨解码失败: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

/// 1s Hann 窗 FFT 在指定频率的幅度（dB 相对目标）。
fn tone_db(samples: &[f32], hz: f64) -> f64 {
    let n = 48_000usize.min(samples.len());
    assert!(n >= 24_000, "分析窗口过短（{n}）");
    let start = (samples.len() - n) / 2; // 取中段，避开起停瞬态
    let mut buf: Vec<Complex<f32>> = samples[start..start + n]
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (n - 1) as f64).cos();
            Complex::new((*s as f64 * w) as f32, 0.0)
        })
        .collect();
    let mut planner = FftPlanner::<f32>::new();
    planner.plan_fft_forward(n).process(&mut buf);
    let bin = (hz * n as f64 / 48_000.0).round() as usize;
    let mag = |k: usize| buf[k].norm() as f64;
    let peak = (bin.saturating_sub(2)..=(bin + 2).min(n / 2))
        .map(mag)
        .fold(0.0f64, f64::max);
    peak
}

fn rel_db(samples: &[f32], target: f64, other: f64) -> f64 {
    let t = tone_db(samples, target);
    let o = tone_db(samples, other);
    20.0 * (o.max(1e-12) / t.max(1e-12)).log10()
}

#[test]
#[ignore = "L2：两路各带不同应用音源；逐路 FFT 目标在/非目标≤−40dB（P1-2c）"]
fn it_multi_route_audio_dual_isolated_fft() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let tmp = fresh_dir("tones");
    let wav_a = gen_tone_wav(&tmp, "a440.wav", 440);
    let wav_b = gen_tone_wav(&tmp, "b880.wav", 880);
    let pa = spawn_player(&wav_a);
    let pb = spawn_player(&wav_b);
    std::thread::sleep(Duration::from_millis(1500)); // 播放稳定
    let created =
        |pid: u32| process_created_unix_ms(pid).unwrap_or_else(|| blocked("读取进程创建时间失败"));

    let out = tmp.join("out");
    std::fs::create_dir_all(&out).expect("out");
    let cfg = |pid: u32| MultiRouteConfig {
        target: RouteTarget::Monitor { index: 0 },
        video: VideoCfg {
            canvas_w: 640,
            canvas_h: 360,
            fps: 30,
            cursor: false,
        },
        audio: vec![AudioSourceSpec::App {
            pid,
            created_at_unix_ms: created(pid),
        }],
        output_dir: out.clone(),
    };
    let handles = match start_routes_with_encoder(vec![cfg(pa.0.id()), cfg(pb.0.id())], None) {
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
        Err(e) => panic!("两路启动应成功（得到 {e:?}）"),
    };
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(8) {
        std::thread::sleep(Duration::from_millis(100));
    }
    for h in &handles {
        h.request_stop();
    }
    let finals: Vec<_> = handles
        .iter()
        .map(|h| {
            h.wait_finalized(Duration::from_secs(120))
                .expect("两路收尾应成功")
        })
        .collect();

    for (fin, (target, other, label)) in finals.iter().zip([
        (440.0f64, 880.0f64, "route-A(440)"),
        (880.0, 440.0, "route-B(880)"),
    ]) {
        let samples = decode_audio_mono(&fin.final_path);
        let db = rel_db(&samples, target, other);
        let t = tone_db(&samples, target);
        println!(
            "[dual-audio] {label}: 目标幅度={t:.1} 非目标相对={db:.2}dB；{}",
            fin.final_path.display()
        );
        assert!(t > 1.0, "{label}: 目标音应存在（幅度 {t:.2}）");
        assert!(
            db <= -40.0,
            "{label}: 非目标频率应 ≤−40dB（得到 {db:.2}dB；串音/路由错误）"
        );
    }
    drop(pa);
    drop(pb);
    println!("[dual-audio] 结论：两路应用音源隔离（各自目标在、非目标 ≤−40dB）");
}
