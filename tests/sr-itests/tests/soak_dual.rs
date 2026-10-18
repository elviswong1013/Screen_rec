//! P1-2(b)：双路长时 soak（REQ-27/P1-2）——2 路均带音频，620s。
//!
//! 断言：两路成品存在、1v+1a（h264/1920x1080@30 + aac48k stereo）、时长≈span（±3s）、
//! 视频包数≈时长×30、整文件可解码、收尾后无 ffmpeg sidecar 残留。
//!
//! 音频源 = 本测试进程（`App{pid}`）：进程内 WASAPI 渲染 1Hz 提示音，供两路各自
//! 进程环回采集（两条独立客户端）。调试可用 `SR_M11_SOAK_DUAL_SECS` 覆盖时长。

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use sr_audio::wasapi::AudioSourceSpec;
use sr_pipeline::route::{
    start_routes_with_encoder, Finalized, MultiRouteConfig, RouteError, RouteTarget, VideoCfg,
};

static GATE: Mutex<()> = Mutex::new(());

const CANVAS: (u32, u32) = (1920, 1080);
const FPS: u32 = 30;

fn soak_secs() -> u64 {
    std::env::var("SR_M11_SOAK_DUAL_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|s| *s >= 15)
        .unwrap_or(620)
}

fn ffbin(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin")
        .join(name)
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
        "ffprobe 失败: {}",
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
    .unwrap_or(0.0)
}

fn probe_packets(path: &Path) -> u64 {
    ffprobe_csv(
        path,
        &[
            "-select_streams",
            "v:0",
            "-count_packets",
            "-show_entries",
            "stream=nb_read_packets",
            "-of",
            "csv=p=0",
        ],
    )
    .parse()
    .unwrap_or(0)
}

fn decode_clean(path: &Path) -> bool {
    let out = Command::new(ffbin("ffmpeg.exe"))
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn ffmpeg：{e}")));
    if !out.status.success() || !out.stderr.is_empty() {
        println!(
            "[soak-dual] decode stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    out.status.success() && out.stderr.is_empty()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr_m11_soak_dual_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("创建临时目录失败");
    d
}

/// 进程内 WASAPI 渲染 1Hz 提示音（保证两路进程环回都有音频包）。
struct ToneSource {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ToneSource {
    fn start() -> Result<ToneSource, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let (tx, rx) = mpsc::channel::<Result<(), String>>();
        let thread = std::thread::Builder::new()
            .name("sr-m11-soak-dual-tone".into())
            .spawn(move || {
                use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};
                let _ = wasapi::initialize_mta();
                let init = (|| -> Result<(), String> {
                    let en =
                        DeviceEnumerator::new().map_err(|e| format!("DeviceEnumerator: {e:?}"))?;
                    let dev = en
                        .get_default_device(&Direction::Render)
                        .map_err(|e| format!("render device: {e:?}"))?;
                    let mut client = dev
                        .get_iaudioclient()
                        .map_err(|e| format!("IAudioClient: {e:?}"))?;
                    let fmt = WaveFormat::new(32, 32, &SampleType::Float, 48_000, 2, None);
                    client
                        .initialize_client(
                            &fmt,
                            &Direction::Render,
                            &StreamMode::PollingShared {
                                autoconvert: true,
                                buffer_duration_hns: 0,
                            },
                        )
                        .map_err(|e| format!("initialize_client: {e:?}"))?;
                    let render = client
                        .get_audiorenderclient()
                        .map_err(|e| format!("render: {e:?}"))?;
                    client.start_stream().map_err(|e| format!("start: {e:?}"))?;
                    let _ = tx.send(Ok(()));
                    let mut pos: u64 = 0;
                    while !stop_thread.load(Ordering::Acquire) {
                        let space = client.get_available_space_in_frames().unwrap_or(0) as usize;
                        if space == 0 {
                            std::thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                        let mut buf = Vec::with_capacity(space * 8);
                        for i in 0..space {
                            let t = (pos + i as u64) as f64 / 48_000.0;
                            let phase = t.fract();
                            let v = if phase < 0.08 {
                                0.3 * (2.0 * std::f64::consts::PI * 660.0 * phase).sin()
                            } else {
                                0.0
                            };
                            let b = (v as f32).to_le_bytes();
                            buf.extend_from_slice(&b);
                            buf.extend_from_slice(&b);
                        }
                        if render.write_to_device(space, &buf, None).is_err() {
                            return Err("write_to_device 失败".into());
                        }
                        pos += space as u64;
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    let _ = client.stop_stream();
                    wasapi::deinitialize();
                    Ok(())
                })();
                if let Err(e) = init {
                    let _ = tx.send(Err(e));
                }
            })
            .expect("spawn tone source");
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(ToneSource {
                stop,
                thread: Some(thread),
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("渲染源 10s 未就绪".into()),
        }
    }
}

impl Drop for ToneSource {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn wide_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

fn ffmpeg_children_of_parent() -> Vec<u32> {
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).expect("snapshot");
        let mut out = Vec::new();
        let mut pe = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut pe).is_ok() {
            loop {
                if pe.th32ParentProcessID == std::process::id()
                    && wide_to_string(&pe.szExeFile)
                        .to_lowercase()
                        .contains("ffmpeg")
                {
                    out.push(pe.th32ProcessID);
                }
                if Process32NextW(snap, &mut pe).is_err() {
                    break;
                }
            }
        }
        out
    }
}

fn cfg(out: &Path, pid: u32, created: u64) -> MultiRouteConfig {
    MultiRouteConfig {
        target: RouteTarget::Monitor { index: 0 },
        video: VideoCfg {
            canvas_w: CANVAS.0,
            canvas_h: CANVAS.1,
            fps: FPS,
            cursor: false,
        },
        audio: vec![AudioSourceSpec::App {
            pid,
            created_at_unix_ms: created,
        }],
        output_dir: out.to_path_buf(),
    }
}

#[test]
#[ignore = "L3：2 路 × 620s 均带音频 soak（P1-2b，REQ-27）"]
fn manual_soak_dual_routes_audio_10min() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let tone =
        ToneSource::start().unwrap_or_else(|e| blocked(&format!("WASAPI 渲染源不可用：{e}")));
    let pid = std::process::id();
    let created = self_created_unix_ms();
    let out = fresh_dir("out");
    let cfgs = vec![cfg(&out, pid, created), cfg(&out, pid, created)];
    let handles = match start_routes_with_encoder(cfgs, None) {
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
        Err(e) => panic!("双路启动应成功（得到 {e:?}）"),
    };
    let target = soak_secs();
    let t0 = Instant::now();
    println!("[soak-dual] 两路已启动（1920x1080@30 + App 音频）；目标 {target}s");
    let mut next = 30.0f64;
    loop {
        let e = t0.elapsed().as_secs_f64();
        if e >= next {
            println!("[soak-dual] elapsed={e:.1}s / 目标 {target}s");
            next += 30.0;
        }
        if e >= target as f64 {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let spans = [t0.elapsed().as_secs_f64(), t0.elapsed().as_secs_f64()];
    for h in &handles {
        h.request_stop();
    }
    let finals: Vec<Finalized> = handles
        .iter()
        .map(|h| {
            h.wait_finalized(Duration::from_secs(240))
                .expect("双路收尾应成功")
        })
        .collect();

    for (i, (fin, span)) in finals.iter().zip(spans.iter()).enumerate() {
        let label = ["A", "B"][i];
        let dur = probe_duration(&fin.final_path);
        let packets = probe_packets(&fin.final_path);
        let v = ffprobe_csv(
            &fin.final_path,
            &[
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_name,width,height",
                "-of",
                "csv=p=0",
            ],
        );
        let a = ffprobe_csv(
            &fin.final_path,
            &[
                "-select_streams",
                "a:0",
                "-show_entries",
                "stream=codec_name,sample_rate,channels",
                "-of",
                "csv=p=0",
            ],
        );
        println!(
            "[soak-dual] 路{label}: span={span:.3}s dur={dur:.3}s packets={packets} v={v} a={a} out={}",
            fin.final_path.display()
        );
        assert!(fin.final_path.exists());
        assert_eq!(v, "h264,1920,1080", "路{label} 视频形状");
        assert_eq!(a, "aac,48000,2", "路{label} 音轨形状");
        assert!(
            (dur - span).abs() <= 3.0,
            "路{label} 时长≈span（{dur:.3} vs {span:.3}）"
        );
        let expected = (dur * FPS as f64) as u64;
        assert!(
            packets.abs_diff(expected) <= expected / 50,
            "路{label} 视频包数应≈{expected}（得到 {packets}）"
        );
        assert!(decode_clean(&fin.final_path), "路{label} 整文件应可解码");
        let min_span = if target >= 600 {
            600.0
        } else {
            target as f64 * 0.9
        };
        assert!(
            span >= &min_span,
            "路{label} 录制段应≥{min_span}s（得到 {span:.3}s）"
        );
    }

    // 无 sidecar 残留。
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if ffmpeg_children_of_parent().is_empty() {
            break;
        }
        if Instant::now() >= deadline {
            panic!(
                "收尾后仍有 ffmpeg sidecar 残留：{:?}",
                ffmpeg_children_of_parent()
            );
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    drop(tone);
    println!("[soak-dual] 结论：2 路 × {target}s 均带音频完成；时长/形状/可解码/无残留 OK");
}
