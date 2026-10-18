//! L2（M17/P3）：MP4 输出、恢复、合并、批量回滚与等待超时（REQ-17/18/19）。
//!
//! 运行：
//! ```text
//! cargo test -p sr-itests --test container -- --ignored --nocapture --test-threads=1
//! ```
//! 前置：交互式会话 + WGC/编码器；端点相关用例另需 ≥2 个活动渲染端点。
//! 环境缺失记 BLOCKED（不静默 skip）。
//!
//! 断言口径（REQ-19）：`format_name` 必须属于 MP4 容器族（mov/mp4/m4a/3gp/3g2/mj2），
//! **不得断言 == "mp4"**；逐轨率/声道/内容 FFT、整文件解码、时长容差。

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
use sr_pipeline::finalize::{
    set_finalize_phase_barrier_for_tests, FinalizePhaseBarrier, FinalizeTestPhase,
};
use sr_pipeline::naming::OutputContainer;
use sr_pipeline::recovery::{
    save_manifest_atomic, scan_sessions, ManifestRoute, RouteIdentity, SessionManifest,
};
use sr_pipeline::route::{
    rollback_cancel_count_for_tests, set_remux_delay_for_tests, set_route_started_hook_for_tests,
    start_routes_with_options_rate_and_container, MultiRouteConfig, RouteError, RouteHandle,
    RouteTarget, VideoCfg,
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
    let d = std::env::temp_dir().join(format!("sr_m17_container_{tag}_{}", std::process::id()));
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

fn probe_format_name(path: &Path) -> String {
    ffprobe_csv(
        path,
        &["-show_entries", "format=format_name", "-of", "csv=p=0"],
    )
}

fn assert_mp4_family(path: &Path) {
    let name = probe_format_name(path).replace('"', "");
    let ok = name
        .split(',')
        .map(str::trim)
        .any(|t| ["mov", "mp4", "m4a", "3gp", "3g2", "mj2"].contains(&t));
    assert!(
        ok,
        "REQ-19：成品容器族应为 mov/mp4/m4a/3gp/3g2/mj2（得到 {name:?}；不得断言 ==\"mp4\"）"
    );
}

fn assert_mkv_family(path: &Path) {
    let name = probe_format_name(path).replace('"', "");
    let ok = name
        .split(',')
        .map(str::trim)
        .any(|t| t == "matroska" || t == "webm");
    assert!(ok, "合并产物应保持 MKV（得到 {name:?}）");
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
    let p: Vec<&str> = s.split(',').collect();
    assert_eq!(p.len(), 3, "视频流 csv 形状（得到 {s:?}）");
    (
        p[0].to_string(),
        p[1].parse().expect("width"),
        p[2].parse().expect("height"),
    )
}

fn decode_clean(path: &Path) -> bool {
    let out = Command::new(ffbin("ffmpeg.exe"))
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn 锁定 ffmpeg：{e}")));
    out.status.success() && out.stderr.is_empty()
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
        "{label}：应以声明采样率 {rate}Hz 检出 {hz}Hz 音调（差 {:.1}dB）",
        tone - floor_db
    );
}

// ─────────────────────────── 指定端点音调渲染（测试替身） ───────────────────────────

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

fn play_tone_to_endpoint(device_id: &str, hz: f64) -> Result<TonePlayer, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    let id = device_id.to_string();
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    let join = std::thread::Builder::new()
        .name("sr-m17-tone-endpoint".into())
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

fn default_render_id() -> String {
    sr_audio::wasapi::default_render_device_id()
        .unwrap_or_else(|| blocked("无法解析默认渲染端点（环境缺失：无渲染端点）"))
}

fn non_default_endpoint() -> (String, String) {
    let default = default_render_id().to_ascii_lowercase();
    sr_enum::render_endpoints::list_render_endpoints()
        .unwrap_or_else(|e| blocked(&format!("渲染端点枚举失败：{e:?}")))
        .into_iter()
        .find(|e| e.is_active && e.device_id.to_ascii_lowercase() != default)
        .map(|e| (e.device_id, e.friendly_name))
        .unwrap_or_else(|| blocked("第二渲染端点不可用（仅默认端点）——端点用例环境缺失"))
}

// ─────────────────────────── 路由助手 ───────────────────────────

fn cfg(out: &Path, sources: Vec<AudioSourceSpec>) -> MultiRouteConfig {
    MultiRouteConfig {
        target: RouteTarget::Monitor { index: 0 },
        video: VideoCfg {
            canvas_w: 640,
            canvas_h: 360,
            fps: 30,
            cursor: false,
        },
        audio: sources,
        output_dir: out.to_path_buf(),
    }
}

fn start_one(
    cfg: MultiRouteConfig,
    rate: u32,
    keep: bool,
    container: OutputContainer,
) -> RouteHandle {
    let container_tag = container.as_str();
    match start_routes_with_options_rate_and_container(vec![cfg], None, keep, rate, container) {
        Ok(mut h) => h.remove(0),
        Err(e @ RouteError::EncoderUnavailable)
        | Err(e @ RouteError::ConsentDenied)
        | Err(e @ RouteError::LoopbackUnavailable { .. }) => {
            blocked(&format!("启动环境缺失（{container_tag}）：{e:?}"))
        }
        Err(e) => panic!("路由启动应成功（得到 {e:?}）"),
    }
}

fn stop_ok(handle: RouteHandle, label: &str) -> sr_pipeline::route::Finalized {
    handle.request_stop();
    handle
        .wait_finalized(Duration::from_secs(240))
        .unwrap_or_else(|e| panic!("{label} 收尾应成功（得到 {e:?}）"))
}

fn session_dir_of(fin: &sr_pipeline::route::Finalized) -> PathBuf {
    fin.segment_list
        .parent()
        .expect("segment_list 应有父目录")
        .to_path_buf()
}

fn tmp_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                out.extend(tmp_files(&p));
            } else if p.to_string_lossy().contains(".tmp") {
                out.push(p);
            }
        }
    }
    out
}

fn media_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.extension().map_or(false, |x| {
                            x.eq_ignore_ascii_case("mp4") || x.eq_ignore_ascii_case("mkv")
                        })
                })
                .collect()
        })
        .unwrap_or_default()
}

// ─────────────────────────── REQ-17/19：MP4 成品 ───────────────────────────

#[test]
#[ignore = "L2：需交互式会话 + 渲染端点 + WGC/编码器；MP4 容器族/音轨 FFT/解码/清理"]
fn it_route_container_mp4() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let default = default_render_id();
    let player = play_tone_to_endpoint(&default, 660.0)
        .unwrap_or_else(|e| blocked(&format!("无法向默认端点渲染音调（{default}）：{e}")));
    std::thread::sleep(Duration::from_millis(800));

    let root = fresh_dir("route");
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("out");
    let handle = start_one(
        cfg(&out, vec![AudioSourceSpec::System]),
        48_000,
        false,
        OutputContainer::Mp4,
    );
    let t = Instant::now();
    std::thread::sleep(Duration::from_secs(8));
    let span = t.elapsed().as_secs_f64();
    let fin = stop_ok(handle, "MP4 路由");

    assert_eq!(
        fin.final_path.extension().and_then(|x| x.to_str()),
        Some("mp4"),
        "REQ-17：成品应为 .mp4（得到 {}）",
        fin.final_path.display()
    );
    assert!(fin.final_path.exists());
    assert_mp4_family(&fin.final_path);
    assert_eq!(
        probe_video_stream(&fin.final_path),
        ("h264".to_string(), 640, 360),
        "视频形状"
    );
    let streams = probe_all_audio_streams(&fin.final_path);
    assert_eq!(
        streams.len(),
        1,
        "System 音源应恰 1 音轨（得到 {streams:?}）"
    );
    let (codec, ar, ch) = &streams[0];
    assert_eq!(codec, "aac");
    assert_eq!(*ar, 48_000, "REQ-19：逐轨采样率");
    assert_eq!(*ch, 2, "REQ-19：逐轨声道");
    let dur = probe_duration(&fin.final_path);
    assert!(
        (dur - span).abs() <= 2.0,
        "REQ-19：时长≈录制段（{dur:.3}s vs {span:.3}s）"
    );
    assert!(decode_clean(&fin.final_path), "REQ-19：整文件应可解码");
    assert_tone_present(
        &decode_audio_mono_at(&fin.final_path, 0, 48_000),
        48_000,
        660.0,
        "mp4-system",
    );
    assert!(
        !session_dir_of(&fin).exists(),
        "keep=false：成功后应清理原段（{}）",
        session_dir_of(&fin).display()
    );
    assert!(
        tmp_files(&out).is_empty(),
        "不应残留 .tmp：{:?}",
        tmp_files(&out)
    );
    drop(player);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "L2：需交互式会话 + WGC/编码器；MP4 四档率流拷贝（44.1k/48k/88.2k/96k）"]
fn it_route_container_mp4_four_rates() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let root = fresh_dir("four-rates");
    for rate in [44_100u32, 48_000, 88_200, 96_000] {
        let out = root.join(format!("out-{rate}"));
        std::fs::create_dir_all(&out).expect("out");
        let handle = start_one(
            cfg(&out, vec![AudioSourceSpec::System]),
            rate,
            false,
            OutputContainer::Mp4,
        );
        std::thread::sleep(Duration::from_secs(4));
        let fin = stop_ok(handle, &format!("MP4 四档率 {rate}"));
        assert_eq!(
            fin.final_path.extension().and_then(|x| x.to_str()),
            Some("mp4")
        );
        assert_mp4_family(&fin.final_path);
        let streams = probe_all_audio_streams(&fin.final_path);
        assert_eq!(streams.len(), 1, "{rate}: 1 音轨（得到 {streams:?}）");
        assert_eq!(streams[0].1, rate, "{rate}: 流拷贝应保持所选采样率");
        assert_eq!(streams[0].2, 2, "{rate}: 声道");
        assert!(decode_clean(&fin.final_path), "{rate}: MP4 应可解码");
        println!("[mp4-four-rates] {rate} → {}", fin.final_path.display());
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "L2：需交互式会话 + ≥2 端点 + WGC/编码器；多音轨 MP4（端点≠系统 2 轨）"]
fn it_route_container_mp4_multitrack_endpoint_system() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let (endpoint, name) = non_default_endpoint();
    let player = play_tone_to_endpoint(&endpoint, 880.0)
        .unwrap_or_else(|e| blocked(&format!("无法向非默认端点渲染音调（{endpoint}）：{e}")));
    std::thread::sleep(Duration::from_millis(800));

    let root = fresh_dir("multitrack");
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("out");
    let handle = start_one(
        cfg(
            &out,
            vec![
                AudioSourceSpec::RenderEndpoint {
                    device_id: endpoint.clone(),
                },
                AudioSourceSpec::System,
            ],
        ),
        48_000,
        true,
        OutputContainer::Mp4,
    );
    let t = Instant::now();
    std::thread::sleep(Duration::from_secs(8));
    let span = t.elapsed().as_secs_f64();
    let fin = stop_ok(handle, "多音轨 MP4 路由");

    assert_eq!(
        fin.final_path.extension().and_then(|x| x.to_str()),
        Some("mp4")
    );
    assert_mp4_family(&fin.final_path);
    let streams = probe_all_audio_streams(&fin.final_path);
    assert_eq!(
        streams.len(),
        2,
        "REQ-17/19：端点≠系统 → 2 音轨（得到 {streams:?}）"
    );
    for (i, (codec, ar, ch)) in streams.iter().enumerate() {
        assert_eq!(codec, "aac", "轨 {i}");
        assert_eq!(*ar, 48_000, "轨 {i} 采样率");
        assert_eq!(*ch, 2, "轨 {i} 声道");
    }
    let dur = probe_duration(&fin.final_path);
    assert!((dur - span).abs() <= 2.0, "时长≈录制段（{dur:.3}）");
    assert!(decode_clean(&fin.final_path));
    assert_tone_present(
        &decode_audio_mono_at(&fin.final_path, 0, 48_000),
        48_000,
        880.0,
        &format!("mp4-endpoint-{name}"),
    );
    println!(
        "[mp4-multitrack] endpoint={name} ({endpoint}) 成品 {} 音轨={streams:?}",
        fin.final_path.display()
    );
    drop(player);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "L2：需交互式会话 + ≥2 端点 + WGC/编码器；端点失效后 MP4 收尾"]
fn it_route_container_mp4_endpoint_lost() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let (endpoint, _) = non_default_endpoint();
    let root = fresh_dir("lost");
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("out");
    let handle = start_one(
        cfg(
            &out,
            vec![AudioSourceSpec::RenderEndpoint {
                device_id: endpoint.clone(),
            }],
        ),
        48_000,
        false,
        OutputContainer::Mp4,
    );
    std::thread::sleep(Duration::from_millis(6500));
    sr_audio::wasapi::inject_device_loss_for_tests(&endpoint, "test: M17 device removed");
    let fin = handle
        .wait_finalized(Duration::from_secs(240))
        .expect("REQ-17：端点失效后 MP4 收尾应成功");
    assert_eq!(
        fin.final_path.extension().and_then(|x| x.to_str()),
        Some("mp4")
    );
    assert_mp4_family(&fin.final_path);
    assert!(decode_clean(&fin.final_path), "MP4 应可解码");
    assert_eq!(
        fin.stop_notice.as_ref().map(|n| n.code.as_str()),
        Some("E_TARGET_LOST"),
        "失效分支应携带 stop_notice"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "L2：需交互式会话 + WGC/编码器；同秒同目标两路 MP4 预留 + 并发提交"]
fn it_route_container_mp4_concurrent_reservations() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let root = fresh_dir("concurrent");
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("out");
    let handles = match start_routes_with_options_rate_and_container(
        vec![cfg(&out, Vec::new()), cfg(&out, Vec::new())],
        None,
        false,
        48_000,
        OutputContainer::Mp4,
    ) {
        Ok(h) => h,
        Err(e @ RouteError::EncoderUnavailable)
        | Err(e @ RouteError::ConsentDenied)
        | Err(e @ RouteError::LoopbackUnavailable { .. }) => {
            blocked(&format!("启动环境缺失：{e:?}"))
        }
        Err(e) => panic!("两路启动应成功（得到 {e:?}）"),
    };
    let reserved: Vec<PathBuf> = handles.iter().map(|h| h.reserved_final_path()).collect();
    assert_ne!(reserved[0], reserved[1], "同秒同目标必须预留不同最终名");
    assert_eq!(
        reserved[0].extension().and_then(|x| x.to_str()),
        Some("mp4"),
        "REQ-17：预留从启动即按容器扩展名"
    );
    assert_eq!(
        reserved[1].extension().and_then(|x| x.to_str()),
        Some("mp4")
    );

    std::thread::sleep(Duration::from_secs(3));
    for h in &handles {
        h.request_stop();
    }
    let finals: Vec<_> = handles
        .iter()
        .map(|h| {
            h.wait_finalized(Duration::from_secs(240))
                .expect("并发两路收尾均应成功")
        })
        .collect();
    for (i, fin) in finals.iter().enumerate() {
        assert_eq!(fin.final_path, reserved[i], "收尾必须提交到预留名");
        assert!(fin.final_path.exists());
        assert_mp4_family(&fin.final_path);
        assert!(decode_clean(&fin.final_path));
    }
    assert!(
        tmp_files(&out).is_empty(),
        "无 .tmp 残留：{:?}",
        tmp_files(&out)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "L2：需交互式会话 + WGC/编码器；提交阶段冲突 → 不覆盖/无成品/原段保留"]
fn it_route_container_mp4_commit_collision() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let root = fresh_dir("collision");
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("out");
    let handle = start_one(cfg(&out, Vec::new()), 48_000, false, OutputContainer::Mp4);
    let reserved = handle.reserved_final_path();
    std::fs::write(&reserved, b"EXTERNAL-SENTINEL").expect("写入占位最终文件");

    std::thread::sleep(Duration::from_secs(3));
    handle.request_stop();
    let res = handle.wait_finalized(Duration::from_secs(240));
    assert!(
        matches!(res, Err(RouteError::FinalizeFailed { .. })),
        "REQ-17：最终名被占用时收尾应显式失败（得到 {res:?}）"
    );
    assert_eq!(
        std::fs::read(&reserved).expect("占位文件应仍存在"),
        b"EXTERNAL-SENTINEL",
        "既有文件不得被覆盖"
    );
    assert!(!handle.segments().is_empty(), "失败必须保留原段");
    assert!(tmp_files(&out).is_empty(), "失败路径应清理临时物");
    let _ = std::fs::remove_dir_all(&root);
}

// ─────────────────────────── REQ-17：等待/回滚契约 ───────────────────────────

#[test]
#[ignore = "L2：需交互式会话 + WGC/编码器；普通 wait_finalized 超时不得取消录制"]
fn it_wait_finalized_timeout_not_cancel() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let root = fresh_dir("wait-timeout");
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("out");
    let handle = start_one(cfg(&out, Vec::new()), 48_000, false, OutputContainer::Mkv);
    assert!(
        matches!(
            handle.wait_finalized(Duration::from_millis(300)),
            Err(RouteError::WaitTimeout)
        ),
        "REQ-17：普通等待超时应返回 WaitTimeout"
    );
    std::thread::sleep(Duration::from_secs(2));
    let fin = stop_ok(handle, "超时后停止");
    assert!(
        fin.final_path.exists(),
        "REQ-17：wait 超时后录制必须未被取消，停止后仍成功收尾"
    );
    let _ = std::fs::remove_dir_all(&root);
}

struct DelayGuard;
impl Drop for DelayGuard {
    fn drop(&mut self) {
        set_remux_delay_for_tests(Duration::ZERO);
        sr_pipeline::route::set_start_delay_for_tests(Duration::ZERO);
    }
}

#[test]
#[ignore = "L2：需交互式会话 + WGC/编码器；延迟 remux + 第二路启动失败 → 回滚取消/无迟到成品"]
fn it_batch_rollback_no_late_commit() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let root = fresh_dir("rollback");
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("out");
    // 前序路有 6s 产出路段；remux 暂停 8s——回滚必须取消而不是等它提交。
    sr_pipeline::route::set_start_delay_for_tests(Duration::from_secs(6));
    set_remux_delay_for_tests(Duration::from_secs(8));
    let _guard = DelayGuard;

    let good = cfg(&out, Vec::new());
    let bad = MultiRouteConfig {
        target: RouteTarget::Window { hwnd: 0 },
        video: VideoCfg {
            canvas_w: 320,
            canvas_h: 180,
            fps: 30,
            cursor: false,
        },
        audio: Vec::new(),
        output_dir: out.clone(),
    };
    let before = rollback_cancel_count_for_tests();
    let started = Instant::now();
    let err = match start_routes_with_options_rate_and_container(
        vec![good, bad],
        None,
        false,
        48_000,
        OutputContainer::Mp4,
    ) {
        Err(e) => e,
        Ok(_) => panic!("第二路启动失败必须回滚整批"),
    };
    let elapsed = started.elapsed();
    println!("[rollback] err={err:?} elapsed={elapsed:?}");
    assert!(
        rollback_cancel_count_for_tests() > before,
        "REQ-17：批量回滚必须显式取消后台收尾"
    );
    assert!(
        elapsed < Duration::from_secs(20),
        "REQ-17：回滚不得等待完整 remux 延迟（elapsed={elapsed:?}）"
    );
    // 回滚返回后不得有迟到提交：再等一个 remux 延迟窗口确认成品不出现。
    std::thread::sleep(Duration::from_secs(9));
    assert!(
        media_files(&out).is_empty(),
        "REQ-17：回滚返回后不得出现迟到成品（得到 {:?}）",
        media_files(&out)
    );
    assert!(
        tmp_files(&out).is_empty(),
        "REQ-17：回滚后不得遗留临时物（得到 {:?}）",
        tmp_files(&out)
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ─────────────────────────── P1-2：取消阶段屏障（remux / 提交边界） ───────────────────────────

struct PhaseBarrierGuard {
    phase: FinalizeTestPhase,
    barrier: Arc<FinalizePhaseBarrier>,
}

impl Drop for PhaseBarrierGuard {
    fn drop(&mut self) {
        // 先放行（断言失败时不得把后台收尾线程永久留在屏障内），再清除注入。
        self.barrier.release();
        set_finalize_phase_barrier_for_tests(self.phase, None);
    }
}

fn phase_guard(phase: FinalizeTestPhase, barrier: &Arc<FinalizePhaseBarrier>) -> PhaseBarrierGuard {
    set_finalize_phase_barrier_for_tests(phase, Some(barrier.clone()));
    PhaseBarrierGuard {
        phase,
        barrier: barrier.clone(),
    }
}

struct RouteStartedHookGuard;
impl Drop for RouteStartedHookGuard {
    fn drop(&mut self) {
        set_route_started_hook_for_tests(None);
    }
}

#[test]
#[ignore = "L2：需交互式会话 + WGC/编码器；remux 进行中取消 → 无成品/无临时物/段保留"]
fn it_cancel_during_remux_no_commit() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let root = fresh_dir("cancel-remux");
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("out");
    let barrier = Arc::new(FinalizePhaseBarrier::new());
    let _guard = phase_guard(FinalizeTestPhase::RemuxStarted, &barrier);

    let handle = start_one(cfg(&out, Vec::new()), 48_000, true, OutputContainer::Mp4);
    let reserved = handle.reserved_final_path();
    std::thread::sleep(Duration::from_secs(3));
    handle.request_stop();
    assert!(
        barrier.wait_entered(Duration::from_secs(180)),
        "P1-2：收尾应进入 remux 阶段（子进程已 spawn）"
    );
    assert!(
        handle.cancel_finalize_for_tests(),
        "P1-2：取消必须能在 remux 进行中先于提交生效"
    );
    barrier.release();

    let res = handle.wait_finalized(Duration::from_secs(120));
    assert!(
        matches!(res, Err(RouteError::FinalizeFailed { .. })),
        "P1-2：remux 中取消应返回 FinalizeFailed（得到 {res:?}）"
    );
    assert!(!reserved.exists(), "P1-2：取消后不得有迟到成品");
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        !reserved.exists(),
        "P1-2：迟到窗口内也不得出现成品（{}）",
        reserved.display()
    );
    assert!(
        !handle.segments().is_empty(),
        "P1-2：取消路径必须保留原段（{}）",
        handle.segments().len()
    );
    assert!(
        tmp_files(&out).is_empty(),
        "P1-2：取消后不得遗留 .tmp（得到 {:?}）",
        tmp_files(&out)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "L2：需交互式会话 + WGC/编码器；提交边界取消 → 无成品/无临时物/段保留"]
fn it_cancel_at_commit_boundary_no_commit() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let root = fresh_dir("cancel-commit");
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("out");
    let barrier = Arc::new(FinalizePhaseBarrier::new());
    let _guard = phase_guard(FinalizeTestPhase::CommitBoundary, &barrier);

    let handle = start_one(cfg(&out, Vec::new()), 48_000, true, OutputContainer::Mp4);
    let reserved = handle.reserved_final_path();
    std::thread::sleep(Duration::from_secs(3));
    handle.request_stop();
    assert!(
        barrier.wait_entered(Duration::from_secs(180)),
        "P1-2：收尾应到达提交边界（remux 校验完成后、safe_commit 之前）"
    );
    assert!(
        handle.cancel_finalize_for_tests(),
        "P1-2：提交边界取消必须先于 safe_commit 生效"
    );
    barrier.release();

    let res = handle.wait_finalized(Duration::from_secs(120));
    assert!(
        matches!(res, Err(RouteError::FinalizeFailed { .. })),
        "P1-2：提交边界取消应返回 FinalizeFailed（得到 {res:?}）"
    );
    assert!(!reserved.exists(), "P1-2：提交边界取消不得发布成品");
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        !reserved.exists(),
        "P1-2：迟到窗口内也不得出现成品（{}）",
        reserved.display()
    );
    assert!(
        !handle.segments().is_empty(),
        "P1-2：取消路径必须保留原段（{}）",
        handle.segments().len()
    );
    assert!(
        tmp_files(&out).is_empty(),
        "P1-2：提交边界取消后不得遗留 .tmp（得到 {:?}）",
        tmp_files(&out)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "L2：需交互式会话 + WGC/编码器；外部占位文件 + 第二路启动失败 → 回滚不得删除外部文件"]
fn it_batch_rollback_preserves_external_reserved_placeholder() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let root = fresh_dir("rollback-external");
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("out");

    let seen: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
    let seen_hook = seen.clone();
    let hook: Arc<dyn Fn(&Path) + Send + Sync> = Arc::new(move |p: &Path| {
        // 第一路已启动并预留最终名：外部（非本事务）文件占用该路径。
        std::fs::write(p, b"EXTERNAL-SENTINEL").expect("写外部占位");
        *seen_hook.lock().expect("seen 锁") = Some(p.to_path_buf());
    });
    set_route_started_hook_for_tests(Some(hook));
    let _guard = RouteStartedHookGuard;

    let good = cfg(&out, Vec::new());
    let bad = MultiRouteConfig {
        target: RouteTarget::Window { hwnd: 0 },
        video: VideoCfg {
            canvas_w: 320,
            canvas_h: 180,
            fps: 30,
            cursor: false,
        },
        audio: Vec::new(),
        output_dir: out.clone(),
    };
    let err = match start_routes_with_options_rate_and_container(
        vec![good, bad],
        None,
        false,
        48_000,
        OutputContainer::Mp4,
    ) {
        Err(e) => e,
        Ok(_) => panic!("第二路启动失败必须回滚整批"),
    };
    println!("[rollback-external] err={err:?}");
    let placeholder = seen
        .lock()
        .expect("seen 锁")
        .clone()
        .expect("P1-1：第一路启动后应触发外部占位注入");
    assert_eq!(
        std::fs::read(&placeholder).expect("外部占位应仍存在"),
        b"EXTERNAL-SENTINEL",
        "P1-1：回滚失败分支绝不能删除预留路径上的外部文件（{}）",
        placeholder.display()
    );
    let media = media_files(&out);
    assert_eq!(
        media,
        vec![placeholder.clone()],
        "P1-1：除外部占位外不得有回滚残留成品（得到 {media:?}）"
    );
    assert!(
        tmp_files(&out).is_empty(),
        "P1-2：回滚返回后不得遗留 .tmp（得到 {:?}）",
        tmp_files(&out)
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ─────────────────────────── REQ-18：恢复与合并 ───────────────────────────

fn make_segment_session(root: &Path, rate: u32, with_audio: bool) -> PathBuf {
    let session = root.join(".screenrec_tmp").join(format!(
        "sess-{rate}-{}",
        if with_audio { "a" } else { "v" }
    ));
    std::fs::create_dir_all(&session).expect("session");
    let pattern = session.join("out0_%05d.mkv");
    let mut cmd = Command::new(ffbin("ffmpeg.exe"));
    cmd.args([
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=160x120:r=10:d=11",
    ]);
    if with_audio {
        cmd.args([
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=440:sample_rate={rate}:duration=11"),
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-c:a",
            "aac",
            "-ar",
            &rate.to_string(),
            "-ac",
            "2",
        ]);
    }
    cmd.args([
        "-c:v",
        "mpeg4",
        "-g",
        "10",
        "-force_key_frames",
        "expr:gte(t,n_forced*5)",
        "-f",
        "segment",
        "-segment_time",
        "5",
        "-reset_timestamps",
        "1",
        "-segment_format",
        "matroska",
    ])
    .arg(&pattern);
    let st = cmd.status().expect("spawn ffmpeg 生成会话段");
    assert!(st.success(), "生成会话段失败（{st}）");

    let manifest = SessionManifest {
        version: 1,
        session_id: session
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        created_at_unix_ms: 0,
        start_qpc: 0,
        routes: vec![ManifestRoute {
            route_id: "route-0".into(),
            identity: RouteIdentity {
                pid: 0,
                created_at_unix_ms: 0,
                name: "Recovered".into(),
                path: PathBuf::from("test"),
            },
            streams: if with_audio {
                vec!["v".into(), "a0".into()]
            } else {
                vec!["v".into()]
            },
            segments: Vec::new(),
            state: "recording".into(),
            final_path: None,
        }],
    };
    save_manifest_atomic(&session, &manifest).expect("写 manifest");
    session
}

#[test]
#[ignore = "L2：媒体侧（无交互）；恢复产出 MP4 容器 + 四档率流拷贝 + 幂等复用"]
fn it_recovery_container_mp4() {
    let root = fresh_dir("recovery");
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("out");
    for rate in [44_100u32, 48_000, 88_200, 96_000] {
        let session = make_segment_session(&out, rate, true);
        let outcome = sr_pipeline::recovery::finalize_session_with_options_and_container(
            &session,
            true,
            OutputContainer::Mp4,
        )
        .unwrap_or_else(|e| panic!("REQ-18：恢复收尾（{rate}）应成功（得到 {e:?}）"));
        assert_eq!(outcome.route_finals.len(), 1);
        let final_path = outcome.route_finals[0].1.clone();
        assert_eq!(
            final_path.extension().and_then(|x| x.to_str()),
            Some("mp4"),
            "REQ-18：恢复产物容器应与设置一致（得到 {}）",
            final_path.display()
        );
        assert_mp4_family(&final_path);
        let streams = probe_all_audio_streams(&final_path);
        assert_eq!(streams.len(), 1, "恢复应保留音轨数（得到 {streams:?}）");
        assert_eq!(
            streams[0].1, rate,
            "REQ-18：恢复必须流拷贝既有采样率 {rate}（不得用当前设置重编码）"
        );
        assert_eq!(streams[0].2, 2);
        assert!(decode_clean(&final_path), "恢复 MP4 应可解码");

        // 幂等：设置改为 mkv 后再收尾 → 复用既有 mp4，不生成另一容器。
        let before = media_files(&out).len();
        let again = sr_pipeline::recovery::finalize_session_with_options_and_container(
            &session,
            true,
            OutputContainer::Mkv,
        )
        .expect("REQ-18：已有有效 final_path 应幂等复用");
        assert_eq!(
            again.route_finals[0].1, final_path,
            "REQ-18：有效 final_path 应复用，不因设置变化再生成另一容器"
        );
        assert_eq!(
            media_files(&out).len(),
            before,
            "REQ-18：不得生成第二个容器成品（得到 {:?}）",
            media_files(&out)
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "L2：媒体侧（无交互）；remux 暂停时活动探测=true、第二次收尾被拒"]
fn it_recovery_active_during_remux() {
    let root = fresh_dir("active-remux");
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("out");
    let session = make_segment_session(&out, 48_000, false);
    set_remux_delay_for_tests(Duration::from_secs(4));
    let _guard = DelayGuard;

    let s = session.clone();
    let worker = std::thread::spawn(move || {
        sr_pipeline::recovery::finalize_session_with_options_and_container(
            &s,
            true,
            OutputContainer::Mp4,
        )
    });
    let active = wait_until(Duration::from_secs(15), Duration::from_millis(100), || {
        scan_sessions(&out)
            .into_iter()
            .find(|x| x.dir == session)
            .filter(|x| x.active)
            .map(|_| ())
    });
    assert!(
        active.is_some(),
        "REQ-17：remux 暂停期间扫描必须为 active（占用标记 + 注册表）"
    );
    let second = sr_pipeline::recovery::finalize_session_with_options_and_container(
        &session,
        true,
        OutputContainer::Mp4,
    );
    assert!(
        matches!(
            second,
            Err(sr_pipeline::recovery::RecoveryError::Active { .. })
        ),
        "REQ-17：同一会话第二次收尾必须被拒（得到 {second:?}）"
    );
    worker.join().expect("worker join").expect("首次收尾应成功");
    let _ = std::fs::remove_dir_all(&root);
}

fn make_media_fixture(path: &Path, color: &str, hz: u32, container: &str) {
    let st = Command::new(ffbin("ffmpeg.exe"))
        .args([
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:s=160x120:r=10:d=1"),
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency={hz}:sample_rate=48000:duration=1"),
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
            "-f",
            container,
        ])
        .arg(path)
        .status()
        .expect("spawn ffmpeg 生成合并夹具");
    assert!(st.success(), "生成合并夹具失败（{st}）");
}

#[test]
#[ignore = "L2：媒体侧（无交互）；MP4+MP4、MP4+MKV → MKV（容器探测接纳）"]
fn it_merge_mp4_inputs() {
    let root = fresh_dir("merge");
    let a = root.join("a.mp4");
    let b = root.join("b.mp4");
    let c = root.join("c.mkv");
    make_media_fixture(&a, "red", 440, "mp4");
    make_media_fixture(&b, "blue", 880, "mp4");
    make_media_fixture(&c, "green", 660, "matroska");
    let bytes = |p: &Path| std::fs::read(p).expect("读夹具");

    let out_mp4_mp4 = root.join("merged_mp4_mp4.mkv");
    let outcome = sr_merge::merge::merge_recordings(sr_merge::merge::MergeConfig {
        inputs: vec![a.clone(), b.clone()],
        out: out_mp4_mp4.clone(),
    })
    .unwrap_or_else(|e| panic!("REQ-18：MP4+MP4 应参与合并（得到 {e:?}）"));
    assert_eq!(outcome.out, out_mp4_mp4);
    assert_mkv_family(&out_mp4_mp4);
    assert_eq!(outcome.video_tracks, 2, "MP4+MP4 应保留两条视频轨");
    assert_eq!(outcome.audio_tracks, 2, "MP4+MP4 应保留两条音轨");
    assert!(decode_clean(&out_mp4_mp4), "合并产物应可解码");

    let out_mp4_mkv = root.join("merged_mp4_mkv.mkv");
    let outcome = sr_merge::merge::merge_recordings(sr_merge::merge::MergeConfig {
        inputs: vec![a.clone(), c.clone()],
        out: out_mp4_mkv.clone(),
    })
    .unwrap_or_else(|e| panic!("REQ-18：MP4+MKV 应参与合并（得到 {e:?}）"));
    assert_eq!(outcome.video_tracks, 2);
    assert_eq!(outcome.audio_tracks, 2);
    assert_mkv_family(&out_mp4_mkv);
    assert_eq!(std::fs::read(&a).expect("a"), bytes(&a), "输入不得被改写");
    assert_eq!(std::fs::read(&c).expect("c"), bytes(&c), "输入不得被改写");
    let _ = std::fs::remove_dir_all(&root);
}
