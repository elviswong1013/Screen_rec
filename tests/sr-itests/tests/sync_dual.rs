//! P1-2(a)：双路长时内容级同步（REQ-25/26）——2 路、每路不同应用音频。
//!
//! 参考源（**两个子进程**，各自单一内容时钟）：子进程 `it_sync_dual_child_mode` 用 WASAPI
//! 渲染 1Hz 短音（440/880Hz，80ms）并绘制同钟驱动的 2s 周期秒表条窗口；父进程按
//! `App{pid=子进程}` 采集各自音频、按 HWND 采集各自视频 → 两路互不串音。
//!
//! 逐路断言（610s）：|offset|≤150ms、末20%−首20%≤100ms、|斜率|≤0.20ms/s、
//! concat 边界 ±0.5s 样本≤150ms 且 ≥100、短音≥580、样本≥1000、1v+1a/1080p30/aac48k、
//! 时长≈span（±3s）、整文件可解码。
//!
//! 调试：`SR_M11_DUAL_SECS=<秒>` 覆盖录制时长（默认 610；覆盖率阈值等比缩放）。

mod common;

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, Once, OnceLock};
use std::time::{Duration, Instant};

use common::*;
use sr_audio::wasapi::AudioSourceSpec;
use sr_pipeline::route::{
    start_routes_with_options_and_rate, Finalized, MultiRouteConfig, RouteError, RouteTarget,
    VideoCfg,
};

static GATE: Mutex<()> = Mutex::new(());

// ─────────────────────────── 测试源常量（与 sync.rs 同口径） ───────────────────────────

const BAR_PERIOD_SECS: f64 = 2.0;
const BAR_SRC_W_PX: f64 = 640.0;
const BAR_W_PX: f64 = 20.0;
const BAR_TRAVEL_PX: f64 = 620.0;
const CLOCK_W: u32 = 64;
const CLOCK_H: u32 = 36;
const BAR_MIN_LUMA: f64 = 100.0;
const BEEP_RMS_MIN: f32 = 0.05;
const BEEP_SAMPLE_MIN: f32 = 0.02;
const SYNC_CANVAS: (u32, u32) = (1920, 1080);
const SYNC_FPS: u32 = 30;
const MAX_OFFSET_MS: f64 = 150.0;
const MAX_HEAD_TAIL_DRIFT_MS: f64 = 100.0;
const MAX_SLOPE_MS_PER_S: f64 = 0.20;

fn record_secs() -> u64 {
    std::env::var("SR_M11_DUAL_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|s| *s >= 15)
        .unwrap_or(610)
}
fn scaled(base: usize) -> usize {
    ((base as f64) * (record_secs() as f64) / 610.0).round() as usize
}

fn ffbin(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin")
        .join(name)
}

fn fresh_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr_m11_dual_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("创建临时目录失败");
    d
}

// ─────────────────────────── 子进程：WASAPI 渲染 + 秒表条窗口 ───────────────────────────

const CHILD_ENV: &str = "SR_M11_DUAL_CHILD_FREQ";
const CHILD_TEST: &str = "it_sync_dual_child_mode";
static CONTENT_US: OnceLock<Arc<AtomicU64>> = OnceLock::new();

fn content_clock() -> Arc<AtomicU64> {
    CONTENT_US
        .get_or_init(|| Arc::new(AtomicU64::new(0)))
        .clone()
}

const TONE_RATE: u64 = 48_000;
const CLOCK_WIN_W: i32 = 640;
const CLOCK_WIN_H: i32 = 360;

struct ToneSource {
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ToneSource {
    fn start(clock: Arc<AtomicU64>, freq: f64) -> Result<ToneSource, String> {
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_thread = stop.clone();
        let (tx, rx) = mpsc::channel::<Result<(), String>>();
        let thread = std::thread::Builder::new()
            .name("sr-m11-tone".into())
            .spawn(move || {
                use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};
                let _ = wasapi::initialize_mta();
                let init = (|| -> Result<(), String> {
                    let enumerator =
                        DeviceEnumerator::new().map_err(|e| format!("DeviceEnumerator: {e:?}"))?;
                    let device = enumerator
                        .get_default_device(&Direction::Render)
                        .map_err(|e| format!("默认渲染端点: {e:?}"))?;
                    let mut client = device
                        .get_iaudioclient()
                        .map_err(|e| format!("IAudioClient: {e:?}"))?;
                    let format = WaveFormat::new(32, 32, &SampleType::Float, 48_000, 2, None);
                    client
                        .initialize_client(
                            &format,
                            &Direction::Render,
                            &StreamMode::PollingShared {
                                autoconvert: true,
                                buffer_duration_hns: 0,
                            },
                        )
                        .map_err(|e| format!("initialize_client(Render): {e:?}"))?;
                    let render = client
                        .get_audiorenderclient()
                        .map_err(|e| format!("render client: {e:?}"))?;
                    client
                        .start_stream()
                        .map_err(|e| format!("start_stream: {e:?}"))?;
                    let _ = tx.send(Ok(()));
                    let mut pos: u64 = 0;
                    while !stop_thread.load(Ordering::Acquire) {
                        let space = client.get_available_space_in_frames().unwrap_or(0) as usize;
                        if space == 0 {
                            std::thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                        let mut buf = Vec::with_capacity(space * 2 * 4);
                        for i in 0..space {
                            let t = (pos + i as u64) as f64 / TONE_RATE as f64;
                            let phase = t.fract();
                            let v = if phase < 0.08 {
                                0.35 * (2.0 * std::f64::consts::PI * freq * phase).sin()
                            } else {
                                0.0
                            };
                            let b = (v as f32).to_le_bytes();
                            buf.extend_from_slice(&b);
                            buf.extend_from_slice(&b);
                        }
                        if render.write_to_device(space, &buf, None).is_err() {
                            return Err("write_to_device 失败".to_string());
                        }
                        pos += space as u64;
                        clock.store(pos * 1_000_000 / TONE_RATE, Ordering::Release);
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

const CLOCK_WIN_CLASS: &str = "SR_M11_DUAL_CLOCK_WND";

unsafe extern "system" fn clock_wndproc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wp: windows::Win32::Foundation::WPARAM,
    lp: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::{COLORREF, LRESULT, RECT};
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, InvalidateRect, HGDIOBJ,
        PAINTSTRUCT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, GetClientRect, KillTimer, PostQuitMessage, WM_DESTROY, WM_PAINT, WM_TIMER,
    };
    unsafe {
        match msg {
            WM_TIMER => {
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                let w = (rc.right - rc.left).max(1);
                let bg = CreateSolidBrush(COLORREF(16 | (16 << 8) | (16 << 16)));
                FillRect(hdc, &rc, bg);
                let _ = DeleteObject(HGDIOBJ(bg.0));

                let us = CONTENT_US
                    .get()
                    .map(|c| c.load(Ordering::Acquire))
                    .unwrap_or(0);
                let secs = us as f64 / 1_000_000.0;
                let pos = (secs % BAR_PERIOD_SECS) / BAR_PERIOD_SECS;
                let x0 = (pos * BAR_TRAVEL_PX) as i32;
                let bar = CreateSolidBrush(COLORREF(255 | (255 << 8) | (255 << 16)));
                let bar_rc = RECT {
                    left: x0,
                    top: 0,
                    right: (x0 + BAR_W_PX as i32).min(w),
                    bottom: (rc.bottom - rc.top).max(1),
                };
                FillRect(hdc, &bar_rc, bar);
                let _ = DeleteObject(HGDIOBJ(bar.0));
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = KillTimer(Some(hwnd), 1);
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

fn child_mode() -> ! {
    let freq: f64 = std::env::var(CHILD_ENV)
        .expect("child freq")
        .parse()
        .expect("freq");
    let clock = content_clock();
    let _tone = match ToneSource::start(clock, freq) {
        Ok(t) => t,
        Err(e) => {
            println!("ERR tone: {e}");
            std::process::exit(3);
        }
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DispatchMessageW, GetMessageW, RegisterClassW, SetTimer, TranslateMessage,
        MSG, WNDCLASSW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
    };
    static REG: Once = Once::new();
    unsafe {
        REG.call_once(|| {
            let hinst = GetModuleHandleW(None).ok().expect("GetModuleHandleW");
            let cn = windows::core::HSTRING::from(CLOCK_WIN_CLASS);
            let wc = WNDCLASSW {
                lpfnWndProc: Some(clock_wndproc),
                hInstance: hinst.into(),
                lpszClassName: windows::core::PCWSTR::from_raw(cn.as_ptr()),
                ..Default::default()
            };
            assert_ne!(RegisterClassW(&wc), 0, "RegisterClassW 失败");
        });
        let hinst = GetModuleHandleW(None).ok().expect("GetModuleHandleW");
        let cn = windows::core::HSTRING::from(CLOCK_WIN_CLASS);
        let title = windows::core::HSTRING::from(format!("SR-M11-DUAL-{freq:.0}"));
        let x = if freq < 660.0 { 220 } else { 900 };
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST,
            windows::core::PCWSTR::from_raw(cn.as_ptr()),
            windows::core::PCWSTR::from_raw(title.as_ptr()),
            WS_POPUP | WS_VISIBLE,
            x,
            220,
            CLOCK_WIN_W,
            CLOCK_WIN_H,
            None,
            None,
            Some(hinst.into()),
            None,
        )
        .expect("CreateWindowExW 失败");
        SetTimer(Some(hwnd), 1, 16, None);
        println!("READY {} {}", hwnd.0 as usize as u64, freq);
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
        }
    }
    std::process::exit(0);
}

#[test]
#[ignore = "P1-2 子进程分支（由 it_sync_dual_routes_content_10min 以环境变量触发）"]
fn it_sync_dual_child_mode() {
    if std::env::var(CHILD_ENV).is_ok() {
        child_mode();
    }
}

// ─────────────────────────── 父进程：采集/解码/内容测量 ───────────────────────────

fn probe_csv(path: &Path, args: &[&str]) -> String {
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
    probe_csv(
        path,
        &["-show_entries", "format=duration", "-of", "csv=p=0"],
    )
    .parse()
    .unwrap_or(0.0)
}

fn probe_streams(path: &Path, selector: &str) -> usize {
    probe_csv(
        path,
        &[
            "-select_streams",
            selector,
            "-show_entries",
            "stream=codec_type",
            "-of",
            "csv=p=0",
        ],
    )
    .lines()
    .filter(|l| !l.trim().is_empty())
    .count()
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
    out.status.success() && out.stderr.is_empty()
}

fn decode_frames_gray(path: &Path, w: u32, h: u32) -> Vec<Vec<u8>> {
    let out = Command::new(ffbin("ffmpeg.exe"))
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args([
            "-vf",
            &format!("scale={w}:{h}"),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "gray",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn ffmpeg 灰度解码：{e}")));
    assert!(
        out.status.success(),
        "灰度解码失败: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let fl = (w * h) as usize;
    assert!(
        !out.stdout.is_empty() && out.stdout.len() % fl == 0,
        "灰度帧字节数异常"
    );
    out.stdout.chunks(fl).map(<[u8]>::to_vec).collect()
}

fn decode_audio_mono_at(path: &Path, rate: u32) -> Vec<f32> {
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
            &rate.to_string(),
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn ffmpeg 音轨解码：{e}")));
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

fn bar_position(frame: &[u8], w: usize, h: usize) -> Option<f64> {
    let mut col_mean = vec![0.0f64; w];
    for x in 0..w {
        let mut sum = 0u32;
        for y in 0..h {
            sum += frame[y * w + x] as u32;
        }
        col_mean[x] = sum as f64 / h as f64;
    }
    let max = col_mean.iter().cloned().fold(0.0f64, f64::max);
    if max < BAR_MIN_LUMA {
        return None;
    }
    let thr = max * 0.5;
    let (mut num, mut den) = (0.0f64, 0.0f64);
    for (x, m) in col_mean.iter().enumerate() {
        if *m >= thr {
            num += x as f64 * m;
            den += m;
        }
    }
    if den <= 0.0 {
        return None;
    }
    let centroid_src = (num / den) * (BAR_SRC_W_PX / w as f64);
    Some(((centroid_src - BAR_W_PX / 2.0) / BAR_TRAVEL_PX).clamp(0.0, 1.0))
}

fn detect_beep_onsets(samples: &[f32], rate: u32) -> Vec<f64> {
    let rate = rate as usize;
    let win = (rate / 200).max(1);
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + win <= samples.len() {
        let rms = (samples[i..i + win].iter().map(|s| s * s).sum::<f32>() / win as f32).sqrt();
        if rms >= BEEP_RMS_MIN {
            let lo = i.saturating_sub(win * 2);
            let mut onset = i;
            for k in lo..i {
                if samples[k].abs() > BEEP_SAMPLE_MIN {
                    onset = k;
                    break;
                }
            }
            out.push(onset as f64 / rate as f64);
            i += rate / 10;
        } else {
            i += win;
        }
    }
    out
}

struct SyncPoint {
    t: f64,
    offset_ms: f64,
}

fn clock_offsets(path: &Path, span: f64) -> Vec<SyncPoint> {
    let frames = decode_frames_gray(path, CLOCK_W, CLOCK_H);
    let beeps = detect_beep_onsets(&decode_audio_mono_at(path, 8_000), 8_000);
    assert!(
        !frames.is_empty() && !beeps.is_empty(),
        "内容时钟：帧/短音不得为空"
    );
    let fps = SYNC_FPS as f64;
    let mut out = Vec::new();
    let mut prev: Option<&Vec<u8>> = None;
    for (i, frame) in frames.iter().enumerate() {
        let changed = match prev {
            Some(p) => p != frame,
            None => true,
        };
        prev = Some(frame);
        if !changed {
            continue;
        }
        let t = i as f64 / fps;
        if t < 2.0 || t > span - 2.0 {
            continue;
        }
        let Some(pos) = bar_position(frame, CLOCK_W as usize, CLOCK_H as usize) else {
            continue;
        };
        let phi_v = (pos * BAR_PERIOD_SECS) % 1.0;
        let idx = beeps.partition_point(|b| *b <= t);
        if idx == 0 {
            continue;
        }
        let b = beeps[idx - 1];
        if t - b > 1.0 {
            continue;
        }
        let mut d = ((t - b) % 1.0) - phi_v;
        while d < -0.5 {
            d += 1.0;
        }
        while d >= 0.5 {
            d -= 1.0;
        }
        out.push(SyncPoint {
            t,
            offset_ms: d * 1e3,
        });
    }
    out
}

fn stats(points: &[SyncPoint]) -> (f64, f64, f64, f64, f64) {
    let n = points.len().max(1);
    let offs: Vec<f64> = points.iter().map(|p| p.offset_ms).collect();
    let mean = offs.iter().sum::<f64>() / n as f64;
    let max_abs = offs.iter().map(|o| o.abs()).fold(0.0f64, f64::max);
    let hn = (points.len() / 5).max(1);
    let head = offs[..hn].iter().sum::<f64>() / hn as f64;
    let tail = offs[points.len() - hn..].iter().sum::<f64>() / hn as f64;
    let (mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0);
    for p in points {
        sx += p.t;
        sy += p.offset_ms;
        sxx += p.t * p.t;
        sxy += p.t * p.offset_ms;
    }
    let den = n as f64 * sxx - sx * sx;
    let slope = if den.abs() > 1e-9 {
        (n as f64 * sxy - sx * sy) / den
    } else {
        0.0
    };
    (mean, max_abs, head, tail, slope)
}

/// `expected_audio`：音轨 (codec,sample_rate,channels) 形状（48k 基线 / 44.1k/96k 变体）。
fn assert_sync_with_audio_shape(path: &Path, span: f64, label: &str, expected_audio: &str) {
    assert_eq!(probe_streams(path, "v"), 1, "{label}: 应恰 1 视频轨");
    assert_eq!(probe_streams(path, "a"), 1, "{label}: 应恰 1 音轨");
    let v = probe_csv(
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
    assert!(
        v.starts_with("h264,1920,1080"),
        "{label}: 视频形状（得到 {v}）"
    );
    let a = probe_csv(
        path,
        &[
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=codec_name,sample_rate,channels",
            "-of",
            "csv=p=0",
        ],
    );
    assert_eq!(a, expected_audio, "{label}: 音轨形状");
    let dur = probe_duration(path);
    assert!(
        (dur - span).abs() <= 3.0,
        "{label}: 时长≈span（{dur:.3} vs {span:.3}）"
    );
    assert!(decode_clean(path), "{label}: 整文件应可解码");

    let points = clock_offsets(path, span);
    let beeps = detect_beep_onsets(&decode_audio_mono_at(path, 8_000), 8_000).len();
    let (mean, max_abs, head, tail, slope) = stats(&points);
    let boundary_max = points
        .iter()
        .filter(|p| {
            let m = p.t % 5.0;
            m < 0.5 || m > 4.5
        })
        .map(|p| p.offset_ms.abs())
        .fold(0.0f64, f64::max);
    let boundary_n = points
        .iter()
        .filter(|p| {
            let m = p.t % 5.0;
            m < 0.5 || m > 4.5
        })
        .count();
    println!(
        "[dual] {label}: span={span:.3}s dur={dur:.3}s samples={} beeps={beeps} mean={mean:.1}ms max={max_abs:.1}ms \
         head={head:.1} tail={tail:.1} slope={slope:.4}ms/s boundary n={boundary_n} max={boundary_max:.1}ms",
        points.len()
    );
    assert!(
        beeps >= scaled(580),
        "{label}: 短音≥{}（得到 {beeps}）",
        scaled(580)
    );
    assert!(
        points.len() >= scaled(1000),
        "{label}: 样本≥{}（得到 {}）",
        scaled(1000),
        points.len()
    );
    assert!(
        max_abs <= MAX_OFFSET_MS,
        "{label}: max|offset|≤150ms（得到 {max_abs:.1}）"
    );
    assert!(
        (tail - head).abs() <= MAX_HEAD_TAIL_DRIFT_MS,
        "{label}: 首末差≤100ms"
    );
    assert!(
        slope.abs() <= MAX_SLOPE_MS_PER_S,
        "{label}: 斜率≤0.20ms/s（得到 {slope:.4}）"
    );
    assert!(
        boundary_n >= scaled(100),
        "{label}: 边界样本≥{}（得到 {boundary_n}）",
        scaled(100)
    );
    assert!(
        boundary_max <= MAX_OFFSET_MS,
        "{label}: 边界 max≤150ms（得到 {boundary_max:.1}）"
    );
}

struct ChildSrc {
    child: Child,
    hwnd: u64,
    pid: u32,
}
impl Drop for ChildSrc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_source(freq: f64, logs: &Path, tag: &str) -> ChildSrc {
    let exe = std::env::current_exe().expect("current_exe");
    let mut child = Command::new(exe)
        .args(["--exact", CHILD_TEST, "--ignored", "--nocapture"])
        .env(CHILD_ENV, format!("{freq}"))
        .stdout(Stdio::piped())
        .stderr(Stdio::from(
            std::fs::File::create(logs.join(format!("{tag}.err"))).expect("err log"),
        ))
        .spawn()
        .expect("spawn dual source child");
    let pid = child.id();
    let stdout = child.stdout.take().expect("child stdout");
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut sent = false;
        // 持续排空子进程 stdout（读取端关闭会让测试 harness 因 BrokenPipe 终止，
        // 渲染线程随之消失 → 环回只剩静音）。
        for line in reader.lines().map_while(Result::ok) {
            if !sent && (line.starts_with("READY ") || line.starts_with("ERR ")) {
                let _ = tx.send(line);
                sent = true;
            }
        }
    });
    let line = rx
        .recv_timeout(Duration::from_secs(20))
        .unwrap_or_else(|_| panic!("子进程 {freq}Hz 20s 内未就绪"));
    if line.starts_with("ERR") {
        blocked(&format!(
            "测试内 WASAPI 渲染源初始化失败（{freq}Hz）：{line}"
        ));
    }
    let hwnd: u64 = line
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .expect("READY hwnd");
    println!("[dual] 源 {freq}Hz：pid={pid} hwnd={hwnd:#x}");
    ChildSrc { child, hwnd, pid }
}

/// 双路同步主流程（48k 基线与 44.1k/96k 变体共用；`expected_audio` 为音轨形状断言）。
fn run_dual(output_rate: u32, expected_audio: &str) -> (f64, Vec<Finalized>) {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let root = fresh_dir(&format!("sync_{output_rate}"));
    let logs = root.join("logs");
    std::fs::create_dir_all(&logs).expect("logs");
    let src_a = spawn_source(440.0, &logs, "a440");
    let src_b = spawn_source(880.0, &logs, "b880");
    std::thread::sleep(Duration::from_millis(1200)); // 渲染稳定

    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("out");
    let created = |pid: u32| {
        process_created_unix_ms(pid)
            .unwrap_or_else(|| blocked(&format!("无法读取子进程创建时间（pid={pid}）")))
    };
    let make_cfg = |hwnd: u64, pid: u32| MultiRouteConfig {
        target: RouteTarget::Window { hwnd },
        video: VideoCfg {
            canvas_w: SYNC_CANVAS.0,
            canvas_h: SYNC_CANVAS.1,
            fps: SYNC_FPS,
            cursor: false,
        },
        audio: vec![AudioSourceSpec::App {
            pid,
            created_at_unix_ms: created(pid),
        }],
        output_dir: out.clone(),
    };
    let cfgs = vec![
        make_cfg(src_a.hwnd, src_a.pid),
        make_cfg(src_b.hwnd, src_b.pid),
    ];
    let handles = match start_routes_with_options_and_rate(cfgs, None, false, output_rate) {
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
    let t0 = Instant::now();
    let target = record_secs();
    println!("[dual] 两路已启动：每路 {target}s（应用音频 440/880Hz；输出率 {output_rate}）");
    let mut next = 30.0f64;
    loop {
        let e = t0.elapsed().as_secs_f64();
        if e >= next {
            println!("[dual] elapsed={e:.1}s / 目标 {target}s");
            next += 30.0;
        }
        if e >= target as f64 {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let span = t0.elapsed().as_secs_f64();
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
    println!(
        "[dual] span={span:.3}s；成品：{} / {}",
        finals[0].final_path.display(),
        finals[1].final_path.display()
    );
    assert_sync_with_audio_shape(
        &finals[0].final_path,
        span,
        "route-A(440Hz)",
        expected_audio,
    );
    assert_sync_with_audio_shape(
        &finals[1].final_path,
        span,
        "route-B(880Hz)",
        expected_audio,
    );
    drop(src_a);
    drop(src_b);
    println!(
        "[dual] 结论：两路应用音频内容级同步均通过（max≤150ms、不累积、边界覆盖；音轨 {expected_audio}）"
    );
    (span, finals)
}

#[test]
#[ignore = "L2：2 路 × 610s，每路不同应用音频；逐路内容级同步（P1-2a，REQ-25/26）"]
fn it_sync_dual_routes_content_10min() {
    let _ = run_dual(48_000, "aac,48000,2");
}

/// M15/REQ-5：96k 输出变体——内容级同步阈值与 48k 基线相同，音轨形状断言
/// `aac,96000,2`（48k 基线用例保持 `aac,48000,2` 不变）。
#[test]
#[ignore = "L2：2 路 × 610s（SR_M11_DUAL_SECS 可缩短），96k 输出变体"]
fn it_sync_dual_routes_content_10min_96k() {
    let _ = run_dual(96_000, "aac,96000,2");
}

/// M15c/REQ-5：44.1k 输出变体——内容级同步阈值与 48k 基线相同，音轨形状断言
/// `aac,44100,2`（非整数基础比率路径）。
#[test]
#[ignore = "L2：2 路 × 610s（SR_M11_DUAL_SECS 可缩短），44.1k 输出变体"]
fn it_sync_dual_routes_content_10min_44100() {
    let _ = run_dual(44_100, "aac,44100,2");
}
