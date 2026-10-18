//! L2（M16/REQ-11/12/13/14）：显示器目标端点环回、失效传播与轨数。
//!
//! 运行：
//! ```text
//! cargo test -p sr-itests --test endpoint_audio -- --ignored --nocapture --test-threads=1
//! ```
//! 前置：交互式会话 + ≥2 个活动渲染端点（非默认端点 = 第二端点）+ WGC + 编码器；
//! 环境缺失记 BLOCKED（不静默 skip）。
//!
//! 失效注入走生产传播链：`inject_device_loss_for_tests` → 与
//! `IMMNotificationClient` 回调相同的设备失效分发 → 写线程 → shared.stop /
//! stop_notice → route worker 优雅停路；另一路（HRESULT）由采集核单测覆盖。

mod common;

use std::f64::consts::PI;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustfft::{num_complex::Complex, FftPlanner};
use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

use common::*;
use sr_audio::wasapi::AudioSourceSpec;
use sr_pipeline::route::{
    start_routes_with_options_and_rate, MultiRouteConfig, RouteError, RouteHandle, RouteTarget,
    VideoCfg,
};

static GATE: Mutex<()> = Mutex::new(());

const NFFT: usize = 32768;
const TONE_PRESENT_DB: f64 = 20.0;

fn ffbin(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin")
        .join(name)
        .canonicalize()
        .unwrap_or_else(|e| blocked(&format!("无法定位锁定 n8.1 的 {name}.exe：{e}")));
    if !p.exists() {
        blocked(&format!("锁定 ffmpeg n8.1 构建缺失：{}", p.display()));
    }
    p
}

fn fresh_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr_m16_endpoint_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("创建临时目录");
    d
}

fn ffprobe_csv(path: &Path, args: &[&str]) -> String {
    let out = Command::new(ffbin("ffprobe.exe"))
        .args(["-v", "error"])
        .args(args)
        .arg(path)
        .output()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn ffprobe：{e}")));
    assert!(
        out.status.success(),
        "ffprobe 失败（exit={}，stderr={}）",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

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
        let p: Vec<&str> = line.split(',').collect();
        assert_eq!(p.len(), 3, "音频流 csv 形状（得到 {line:?}）");
        out.push((
            p[0].to_string(),
            p[1].parse().expect("sample_rate"),
            p[2].parse().expect("channels"),
        ));
    }
    out
}

fn probe_duration(path: &Path) -> f64 {
    ffprobe_csv(
        path,
        &["-show_entries", "format=duration", "-of", "csv=p=0"],
    )
    .parse()
    .ok()
    .filter(|d| *d > 0.0)
    .unwrap_or_else(|| panic!("应能读出成品时长（{}）", path.display()))
}

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
        .unwrap_or_else(|e| blocked(&format!("无法解码音轨：{e}")));
    assert!(out.status.success(), "音轨解码失败（exit={}）", out.status);
    out.stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

fn avg_spectrum(x: &[f32], sample_rate: u32) -> (f64, Vec<f64>) {
    assert!(x.len() > NFFT * 2, "可分析样本过短（{}）", x.len());
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

fn assert_tone_present(x: &[f32], rate: u32, hz: f64, label: &str) {
    let (bin_hz, power) = avg_spectrum(x, rate);
    let band_peak = |hz: f64, half_bw: f64| -> f64 {
        let k0 = (((hz - half_bw) / bin_hz).floor() as usize).max(1);
        let k1 = (((hz + half_bw) / bin_hz).ceil() as usize).min(power.len() / 2);
        let p = (k0..=k1).fold(0.0f64, |m, k| m.max(power[k]));
        10.0 * p.log10()
    };
    let mut floor: Vec<f64> = (1..power.len() / 2)
        .map(|k| (k, k as f64 * bin_hz))
        .filter(|(_, f)| *f >= 120.0 && *f <= 6000.0 && (*f - hz).abs() > 40.0)
        .map(|(k, _)| power[k])
        .collect();
    assert!(floor.len() >= 8, "噪声底样本过少");
    floor.sort_by(|a, b| a.partial_cmp(b).expect("NaN 功率"));
    let floor_db = 10.0 * floor[floor.len() / 2].log10();
    let tone = band_peak(hz, 5.0);
    println!(
        "[fft:{label}] rate={rate}Hz tone({hz})={tone:.1}dB floor={floor_db:.1}dB ⇒ {:.1}dB",
        tone - floor_db
    );
    assert!(
        tone - floor_db >= TONE_PRESENT_DB,
        "REQ-11：{label} 应以声明采样率 {rate}Hz 检出 {hz}Hz 端点音调（差 {:.1}dB）",
        tone - floor_db
    );
}

// ─────────────────────────── 指定端点音调渲染（测试替身，产品外） ───────────────────────────

struct TonePlayer {
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Drop for TonePlayer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.join.take() {
            let _ = h.join();
        }
    }
}

/// 向指定渲染端点持续写 880Hz 正弦（共享模式、f32/双声道、原生率）。
fn play_tone_to_endpoint(device_id: &str, hz: f64) -> Result<TonePlayer, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    let id = device_id.to_string();
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    let join = std::thread::Builder::new()
        .name("sr-m16-tone-endpoint".into())
        .spawn(move || {
            let result = (|| -> Result<(), String> {
                let _ = wasapi::initialize_mta();
                let enumerator = DeviceEnumerator::new().map_err(|e| format!("枚举器: {e:?}"))?;
                let device = enumerator
                    .get_device(&id)
                    .map_err(|e| format!("get_device({id}): {e:?}"))?;
                let mut client = device
                    .get_iaudioclient()
                    .map_err(|e| format!("get_iaudioclient: {e:?}"))?;
                let rate = client
                    .get_mixformat()
                    .map(|f| f.get_samplespersec())
                    .unwrap_or(48_000);
                let fmt = WaveFormat::new(32, 32, &SampleType::Float, rate as usize, 2, None);
                client
                    .initialize_client(
                        &fmt,
                        &Direction::Render,
                        &StreamMode::PollingShared {
                            autoconvert: true,
                            buffer_duration_hns: 0,
                        },
                    )
                    .map_err(|e| format!("initialize_client(render): {e:?}"))?;
                let render = client
                    .get_audiorenderclient()
                    .map_err(|e| format!("get_audiorenderclient: {e:?}"))?;
                client
                    .start_stream()
                    .map_err(|e| format!("start_stream(render): {e:?}"))?;
                let _ = tx.send(Ok(()));
                let mut phase = 0.0f64;
                while !stop_thread.load(Ordering::Acquire) {
                    let frames = client
                        .get_available_space_in_frames()
                        .unwrap_or(0)
                        .min(rate / 100) as usize;
                    if frames == 0 {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    let mut bytes = Vec::with_capacity(frames * 8);
                    for _ in 0..frames {
                        let s = (2.0 * PI * phase).sin() as f32 * 0.25;
                        phase = (phase + hz / rate as f64) % 1.0;
                        bytes.extend_from_slice(&s.to_le_bytes());
                        bytes.extend_from_slice(&s.to_le_bytes());
                    }
                    if render.write_to_device(frames, &bytes, None).is_err() {
                        break;
                    }
                }
                let _ = client.stop_stream();
                Ok(())
            })();
            if let Err(e) = result {
                let _ = tx.send(Err(e));
            }
        })
        .map_err(|e| format!("spawn 端点音调线程: {e}"))?;
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(TonePlayer {
            stop,
            join: Some(join),
        }),
        Ok(Err(e)) => {
            let _ = join.join();
            Err(e)
        }
        Err(_) => {
            stop.store(true, Ordering::Release);
            let _ = join.join();
            Err("端点音调渲染激活超时（5s）".into())
        }
    }
}

// ─────────────────────────── 端点/路由助手 ───────────────────────────

fn default_render_id() -> String {
    sr_audio::wasapi::default_render_device_id()
        .unwrap_or_else(|| blocked("无法解析默认渲染端点（REQ-11 环境缺失：无渲染端点）"))
}

fn endpoints() -> Vec<sr_enum::render_endpoints::RenderEndpoint> {
    sr_enum::render_endpoints::list_render_endpoints()
        .unwrap_or_else(|e| blocked(&format!("渲染端点枚举失败：{e:?}")))
}

/// 第二端点（active 且 ≠ 默认）；不可用 → BLOCKED（REQ-11 行）。
fn non_default_endpoint() -> (String, String) {
    let default = default_render_id().to_ascii_lowercase();
    endpoints()
        .into_iter()
        .find(|e| e.is_active && e.device_id.to_ascii_lowercase() != default)
        .map(|e| (e.device_id, e.friendly_name))
        .unwrap_or_else(|| blocked("第二渲染端点不可用（仅默认端点）——REQ-11 正路径环境缺失"))
}

fn start_route(sources: Vec<AudioSourceSpec>, rate: u32, keep: bool, out: &Path) -> RouteHandle {
    let cfg = MultiRouteConfig {
        target: RouteTarget::Monitor { index: 0 },
        video: VideoCfg {
            canvas_w: 640,
            canvas_h: 360,
            fps: 30,
            cursor: false,
        },
        audio: sources,
        output_dir: out.to_path_buf(),
    };
    match start_routes_with_options_and_rate(vec![cfg], None, keep, rate) {
        Ok(mut h) => h.remove(0),
        Err(e @ RouteError::EncoderUnavailable)
        | Err(e @ RouteError::ConsentDenied)
        | Err(e @ RouteError::LoopbackUnavailable { .. }) => {
            blocked(&format!("启动环境缺失：{e:?}"))
        }
        Err(e) => panic!("REQ-11：端点路由启动应成功（得到 {e:?}）"),
    }
}

fn endpoint_source(id: &str) -> AudioSourceSpec {
    AudioSourceSpec::RenderEndpoint {
        device_id: id.to_string(),
    }
}

fn stop_ok(handle: RouteHandle, label: &str) -> sr_pipeline::route::Finalized {
    handle.request_stop();
    handle
        .wait_finalized(Duration::from_secs(120))
        .unwrap_or_else(|e| panic!("{label} 收尾应成功（得到 {e:?}）"))
}

fn session_dir_of(fin: &sr_pipeline::route::Finalized) -> PathBuf {
    fin.segment_list
        .parent()
        .expect("segment_list 应有父目录")
        .to_path_buf()
}

// ─────────────────────────── REQ-11/13：端点音调内容级验证 ───────────────────────────

fn endpoint_audio_case(rate: u32, tag: &str) {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let (endpoint, name) = non_default_endpoint();
    let tmp = fresh_dir(tag);
    let player = play_tone_to_endpoint(&endpoint, 880.0)
        .unwrap_or_else(|e| blocked(&format!("无法向非默认端点渲染音调（{endpoint}）：{e}")));
    std::thread::sleep(Duration::from_millis(800));

    let out = tmp.join("out");
    std::fs::create_dir_all(&out).expect("创建输出目录");
    let handle = start_route(vec![endpoint_source(&endpoint)], rate, false, &out);
    std::thread::sleep(Duration::from_millis(300));
    let t = Instant::now();
    std::thread::sleep(Duration::from_secs(10));
    let span = t.elapsed().as_secs_f64();
    let fin = stop_ok(handle, "端点音调路由");
    assert!(fin.final_path.exists(), "成品应存在");

    let streams = probe_all_audio_streams(&fin.final_path);
    assert_eq!(
        streams.len(),
        1,
        "REQ-13：仅预选端点 → 恰 1 条音轨（得到 {streams:?}）"
    );
    let (codec, ar, ch) = &streams[0];
    assert_eq!(codec, "aac");
    assert_eq!(*ar, rate, "REQ-11：输出率应为设定值 {rate}");
    assert_eq!(*ch, 2);
    let dur = probe_duration(&fin.final_path);
    assert!(
        (dur - span).abs() <= 2.0,
        "成品时长应≈录制段（{dur:.3}s vs {span:.3}s）"
    );
    let mono = decode_audio_mono_at(&fin.final_path, 0, rate);
    assert_tone_present(&mono, rate, 880.0, &format!("endpoint-{tag}"));

    println!(
        "[endpoint-audio-{tag}] endpoint={name} ({endpoint}) 成品 {} 音轨={streams:?}",
        fin.final_path.display()
    );
    drop(player);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
#[ignore = "L2：需交互式会话 + ≥2 端点 + WGC/编码器。cargo test -p sr-itests --test endpoint_audio -- --ignored --nocapture --test-threads=1"]
fn it_route_endpoint_audio_fft_nondefault_48000() {
    endpoint_audio_case(48_000, "48k");
}

#[test]
#[ignore = "L2：需交互式会话 + ≥2 端点 + WGC/编码器"]
fn it_route_endpoint_audio_fft_nondefault_96000() {
    endpoint_audio_case(96_000, "96k");
}

// ─────────────────────────── REQ-11：启动校验 ───────────────────────────

#[test]
#[ignore = "L2：需交互式会话 + WGC/编码器"]
fn it_route_endpoint_invalid_at_start() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let tmp = fresh_dir("invalid");
    let out = tmp.join("out");
    std::fs::create_dir_all(&out).expect("创建输出目录");

    // 不存在的 device_id → 明确失败、不产生路段。
    let cfg = MultiRouteConfig {
        target: RouteTarget::Monitor { index: 0 },
        video: VideoCfg {
            canvas_w: 640,
            canvas_h: 360,
            fps: 30,
            cursor: false,
        },
        audio: vec![endpoint_source("__sr_m16_nonexistent_endpoint__")],
        output_dir: out.clone(),
    };
    let err = match start_routes_with_options_and_rate(vec![cfg], None, false, 48_000) {
        Err(e) => e,
        Ok(_) => panic!("REQ-11：无效端点必须在启动时失败"),
    };
    assert!(
        matches!(err, RouteError::TargetInvalid { .. }),
        "REQ-11：无效端点应为明确错误（E_TARGET_LOST），得到 {err:?}"
    );
    let residues: Vec<_> = std::fs::read_dir(out.join(".screenrec_tmp"))
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    assert!(
        residues.is_empty(),
        "REQ-11：启动校验失败不得产生路段/会话目录（得到 {residues:?}）"
    );

    // 非 Render（采集设备）→ 同样拒绝；本机无采集设备时该分支 BLOCKED。
    match sr_audio::wasapi::list_capture_devices() {
        Ok(devices) if devices.is_empty() => {
            println!(
                "[endpoint-invalid] 无采集设备：非 Render 校验分支记为 BLOCKED（环境缺失，非通过）"
            );
        }
        Ok(devices) => {
            let mic = &devices[0];
            let cfg = MultiRouteConfig {
                target: RouteTarget::Monitor { index: 0 },
                video: VideoCfg {
                    canvas_w: 640,
                    canvas_h: 360,
                    fps: 30,
                    cursor: false,
                },
                audio: vec![endpoint_source(&mic.id)],
                output_dir: out.clone(),
            };
            let err = match start_routes_with_options_and_rate(vec![cfg], None, false, 48_000) {
                Err(e) => e,
                Ok(_) => panic!("REQ-11：非 Render 设备不得作为端点音源启动"),
            };
            assert!(
                matches!(err, RouteError::TargetInvalid { .. }),
                "非 Render 设备应明确失败（得到 {err:?}）"
            );
        }
        Err(e) => blocked(&format!("无法枚举采集设备（{e:?}）")),
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

// ─────────────────────────── REQ-12：运行期失效传播 ───────────────────────────

#[test]
#[ignore = "L2：需交互式会话 + ≥2 端点 + WGC/编码器（注入失效）"]
fn it_route_endpoint_runtime_lost_keeps_others() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let (endpoint, name) = non_default_endpoint();
    let tmp = fresh_dir("runtime-lost");
    let out = tmp.join("out");
    std::fs::create_dir_all(&out).expect("创建输出目录");

    // D：另一路（系统音源 + 自建窗口目标）必须继续。
    let win = ProbeWindow::new(&format!("SR-M16-LOST-{}", std::process::id()));
    std::thread::sleep(Duration::from_millis(300));
    let other = {
        let cfg = MultiRouteConfig {
            target: RouteTarget::Window { hwnd: win.hwnd_u64 },
            video: VideoCfg {
                canvas_w: 320,
                canvas_h: 180,
                fps: 30,
                cursor: false,
            },
            audio: vec![AudioSourceSpec::System],
            output_dir: out.clone(),
        };
        match start_routes_with_options_and_rate(vec![cfg], None, false, 48_000) {
            Ok(mut h) => h.remove(0),
            Err(e @ RouteError::EncoderUnavailable)
            | Err(e @ RouteError::ConsentDenied)
            | Err(e @ RouteError::LoopbackUnavailable { .. }) => {
                blocked(&format!("启动环境缺失：{e:?}"))
            }
            Err(e) => panic!("其它路启动应成功（得到 {e:?}）"),
        }
    };

    // A：成功分支 + keep=false → 优雅停路、notice 可读、原段已清理。
    let endpoint_route = start_route(vec![endpoint_source(&endpoint)], 48_000, false, &out);
    std::thread::sleep(Duration::from_millis(6500)); // 至少一个 5s 段
    sr_audio::wasapi::inject_device_loss_for_tests(&endpoint, "test: device removed");
    let fin = endpoint_route
        .wait_finalized(Duration::from_secs(90))
        .expect("REQ-12：端点失效后该路应优雅收尾成功（内容保留）");
    assert!(fin.final_path.exists());
    let notice = fin
        .stop_notice
        .as_ref()
        .expect("REQ-12：成功收尾分支必须携带 stop_notice");
    assert_eq!(notice.code, "E_TARGET_LOST");
    assert_eq!(
        notice.endpoint.as_deref(),
        Some(name.as_str()),
        "stop_notice.endpoint 应为端点名（缺名回退 device_id）"
    );
    assert!(
        !session_dir_of(&fin).exists(),
        "REQ-12/M14c：成功+keep=false 时应清理原段（{}）",
        session_dir_of(&fin).display()
    );

    // 其它路继续：仍在录制（未因另一路端点失效而停）。
    assert!(
        matches!(
            other.wait_finalized(Duration::from_millis(200)),
            Err(RouteError::WaitTimeout)
        ),
        "REQ-12：仅停失效路，其它路必须继续"
    );

    // B：成功分支 + keep=true → 原段保留。
    let out_keep = tmp.join("out_keep");
    std::fs::create_dir_all(&out_keep).expect("创建输出目录");
    let endpoint_keep = start_route(vec![endpoint_source(&endpoint)], 48_000, true, &out_keep);
    std::thread::sleep(Duration::from_millis(6500));
    sr_audio::wasapi::inject_device_loss_for_tests(&endpoint, "test: device removed again");
    let fin_keep = endpoint_keep
        .wait_finalized(Duration::from_secs(90))
        .expect("REQ-12：keep=true 收尾应成功");
    assert_eq!(
        fin_keep.stop_notice.as_ref().map(|n| n.code.as_str()),
        Some("E_TARGET_LOST")
    );
    assert!(
        session_dir_of(&fin_keep).exists(),
        "REQ-12/M14c：成功+keep=true 时原段保留（{}）",
        session_dir_of(&fin_keep).display()
    );

    // C：收尾失败分支 → notice 仍可读、原段保留。
    let out_fail = tmp.join("out_fail");
    std::fs::create_dir_all(&out_fail).expect("创建输出目录");
    let endpoint_fail = start_route(vec![endpoint_source(&endpoint)], 48_000, false, &out_fail);
    std::thread::sleep(Duration::from_millis(6500));
    // 注入收尾失败（生产 finalize 路径，保留原段），随后通知链注入端点失效。
    endpoint_fail.force_finalize_failure_for_tests();
    sr_audio::wasapi::inject_device_loss_for_tests(
        &endpoint,
        "test: device removed (failure branch)",
    );
    let err = endpoint_fail
        .wait_finalized(Duration::from_secs(90))
        .expect_err("REQ-12：注入收尾失败 → 失败分支");
    println!("[endpoint-lost] failure branch = {err:?}");
    let notice = endpoint_fail
        .stop_notice()
        .expect("REQ-12：失败分支 stop_notice 必须仍可读");
    assert_eq!(notice.code, "E_TARGET_LOST");
    let sessions: Vec<PathBuf> = std::fs::read_dir(out_fail.join(".screenrec_tmp"))
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    assert!(
        !sessions.is_empty() && sessions.iter().any(|d| dir_has_mkv(d)),
        "REQ-12：收尾失败必须保留原段（得到 {sessions:?}）"
    );

    // 清理其它路（D）并确认其正常收尾。
    let other_fin = stop_ok(other, "其它路");
    assert!(other_fin.final_path.exists());
    assert!(
        other_fin.stop_notice.is_none(),
        "REQ-12：静音/无包不得产生目标丢失 notice"
    );

    drop(win);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
#[ignore = "L2：需交互式会话 + WGC/编码器"]
fn it_route_endpoint_silence_does_not_stop() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let (endpoint, _) = non_default_endpoint();
    let tmp = fresh_dir("silence");
    let out = tmp.join("out");
    std::fs::create_dir_all(&out).expect("创建输出目录");
    let handle = start_route(vec![endpoint_source(&endpoint)], 48_000, true, &out);
    std::thread::sleep(Duration::from_secs(4));
    assert!(
        handle.stop_notice().is_none(),
        "REQ-12：静音/无包不得判失效"
    );
    assert!(
        matches!(
            handle.wait_finalized(Duration::from_millis(100)),
            Err(RouteError::WaitTimeout)
        ),
        "端点路应仍在录制"
    );
    let fin = stop_ok(handle, "静音端点路");
    assert!(fin.final_path.exists());
    let _ = std::fs::remove_dir_all(&tmp);
}

// ─────────────────────────── REQ-13：轨数与去重（成品 ffprobe） ───────────────────────────

fn track_count_case(sources: Vec<AudioSourceSpec>, expected: usize, tag: &str, rate: u32) {
    let tmp = fresh_dir(tag);
    let out = tmp.join("out");
    std::fs::create_dir_all(&out).expect("创建输出目录");
    let handle = start_route(sources, rate, true, &out);
    std::thread::sleep(Duration::from_secs(6));
    let fin = stop_ok(handle, tag);
    let streams = probe_all_audio_streams(&fin.final_path);
    assert_eq!(
        streams.len(),
        expected,
        "REQ-13：{tag} 成品实际音轨数应为 {expected}（得到 {streams:?}）"
    );
    for (i, (codec, ar, ch)) in streams.iter().enumerate() {
        assert_eq!(codec, "aac", "轨 {i}");
        assert_eq!(*ar, rate, "轨 {i} 采样率");
        assert_eq!(*ch, 2, "轨 {i} 声道");
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
#[ignore = "L2：需交互式会话 + ≥2 端点 + WGC/编码器（48k/96k 轨数）"]
fn it_route_display_endpoint_tracks() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let default = default_render_id();
    let (non_default, _) = non_default_endpoint();

    // 仅预选端点 → 1 轨（48k）。
    track_count_case(vec![endpoint_source(&default)], 1, "only-endpoint", 48_000);
    // 端点==系统（默认端点 + System）→ 去重 1 轨（96k 组合）。
    track_count_case(
        vec![endpoint_source(&default), AudioSourceSpec::System],
        1,
        "dedupe-endpoint-system",
        96_000,
    );
    // 端点≠系统 → 2 轨（96k 组合）。
    track_count_case(
        vec![endpoint_source(&non_default), AudioSourceSpec::System],
        2,
        "endpoint-plus-system",
        96_000,
    );
}

#[test]
#[ignore = "L2：需交互式会话 + 采集设备 + WGC/编码器；无麦克风 → BLOCKED"]
fn it_route_display_endpoint_plus_mic_tracks() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let default = default_render_id();
    let mic = match sr_audio::wasapi::list_capture_devices() {
        Ok(d) if !d.is_empty() => d[0].id.clone(),
        Ok(_) => blocked("无采集设备：端点+麦克风 2 轨分支记为 BLOCKED（环境缺失）"),
        Err(e) => blocked(&format!("无法枚举采集设备：{e:?}")),
    };
    track_count_case(
        vec![
            endpoint_source(&default),
            AudioSourceSpec::Mic { device: mic },
        ],
        2,
        "endpoint-plus-mic",
        48_000,
    );
}

fn dir_has_mkv(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten().any(|e| {
                e.path()
                    .extension()
                    .map_or(false, |x| x.eq_ignore_ascii_case("mkv"))
                    && std::fs::metadata(e.path())
                        .map(|m| m.len() > 0)
                        .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}
