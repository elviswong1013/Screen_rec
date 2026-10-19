//! L2（M18/P4）：区域视频录制（REQ-21..24）。
//!
//! 运行：
//! ```text
//! cargo test -p sr-itests --test region_video -- --ignored --nocapture --test-threads=1
//! ```
//! 前置：交互式会话 + WGC/编码器 + 锁定 ffmpeg n8.1；端点/麦克风分项缺失记 BLOCKED。
//!
//! 覆盖：48k/96k 裁剪尺寸与内容（选区内外不同图案）、失效闩锁交错（成功/收尾失败）、
//! 显示器失效保留其它路、强杀恢复、MP4、异构尺寸合并、区域+窗口并发、尺寸拒绝。
//! 裁剪内容判据：区域选区在纯红矩形内，选区外全部纯蓝——任何整屏缩放/裁错位都会
//! 把蓝色带进帧（不依赖 PNG 阈值）。

mod common;

use std::f64::consts::PI;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc, Barrier, Mutex};
use std::time::Duration;

use rustfft::{num_complex::Complex, FftPlanner};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, UpdateWindow, HBRUSH, HDC,
    HGDIOBJ, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    PostMessageW, PostQuitMessage, RegisterClassW, ShowWindow, TranslateMessage, MSG,
    SHOW_WINDOW_CMD, WINDOW_EX_STYLE, WM_CLOSE, WM_ERASEBKGND, WM_PAINT, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

use common::*;
use sr_audio::wasapi::AudioSourceSpec;
use sr_pipeline::naming::OutputContainer;
use sr_pipeline::recovery;
use sr_pipeline::route::{
    set_region_latch_hook_for_tests, set_region_size_probe_override_for_tests,
    start_routes_with_options_rate_and_container, MultiRouteConfig, RegionRect, RouteError,
    RouteHandle, RouteTarget, VideoCfg,
};

static GATE: Mutex<()> = Mutex::new(());

const NFFT: usize = 32768;
const TONE_PRESENT_DB: f64 = 20.0;
const TONE_ISOLATED_DB: f64 = -40.0;

// ─────────────────────────── 基础工具 ───────────────────────────

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
    let d = std::env::temp_dir().join(format!("sr_m18_region_{tag}_{}", std::process::id()));
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
        "REQ-19：成品容器族应为 mov/mp4/m4a/3gp/3g2/mj2（得到 {name:?}）"
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

fn probe_all_video_streams(path: &Path) -> Vec<(u32, u32)> {
    let s = ffprobe_csv(
        path,
        &[
            "-select_streams",
            "v",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0",
        ],
    );
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let p: Vec<&str> = l.split(',').collect();
            assert_eq!(p.len(), 2, "视频流 csv 形状（得到 {l:?}）");
            (p[0].parse().expect("w"), p[1].parse().expect("h"))
        })
        .collect()
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

/// 解码指定时间点的一帧 RGB24。
fn decode_video_rgb24(path: &Path, at_secs: f64) -> (u32, u32, Vec<u8>) {
    let (_, w, h) = probe_video_stream(path);
    let out = Command::new(ffbin("ffmpeg.exe"))
        .args(["-v", "error", "-nostdin", "-ss", &at_secs.to_string(), "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .unwrap_or_else(|e| blocked(&format!("无法解码区域帧：{e}")));
    assert!(
        out.status.success(),
        "区域帧解码失败（exit={}）",
        out.status
    );
    assert_eq!(
        out.stdout.len(),
        w as usize * h as usize * 3,
        "RGB24 帧字节数应为 {w}x{h}x3（得到 {}）",
        out.stdout.len()
    );
    (w, h, out.stdout)
}

/// 区域内容判据：帧中心/边缘采样必须为纯红（选区外纯蓝；任何裁错位都会带蓝）。
fn assert_frame_is_red(path: &Path, at_secs: f64) {
    let (w, h, rgb) = decode_video_rgb24(path, at_secs);
    let sample = |x: u32, y: u32| -> (u8, u8, u8) {
        let i = (y as usize * w as usize + x as usize) * 3;
        (rgb[i], rgb[i + 1], rgb[i + 2])
    };
    for (x, y, tag) in [
        (w / 2, h / 2, "中心"),
        (w / 8, h / 8, "左上"),
        (w - w / 8, h - h / 8, "右下"),
        (w - w / 8, h / 8, "右上"),
    ] {
        let (r, g, b) = sample(x, y);
        assert!(
            r >= 200 && g <= 40 && b <= 40,
            "REQ-22 裁剪内容：{tag} 采样应为选区内的红色（得到 RGB({r},{g},{b})；\
             蓝色 = 整屏缩放/裁错位）"
        );
    }
    println!("[region] 帧内容 OK（{w}x{h} @{at_secs:.1}s，纯红选区）");
}

// ─────────────────────────── FFT（音轨内容） ───────────────────────────

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

fn tone_levels(x: &[f32], rate: u32, hz: f64) -> (f64, f64) {
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
    (band_peak(hz, 5.0), floor_db)
}

fn assert_tone_present(x: &[f32], rate: u32, hz: f64, label: &str) {
    let (tone, floor) = tone_levels(x, rate, hz);
    println!("[fft:{label}] rate={rate} tone({hz})={tone:.1}dB floor={floor:.1}dB");
    assert!(
        tone - floor >= TONE_PRESENT_DB,
        "{label}：应以 {rate}Hz 检出 {hz}Hz（差 {:.1}dB）",
        tone - floor
    );
}

fn assert_tone_not_present(x: &[f32], rate: u32, target_hz: f64, other_hz: f64, label: &str) {
    let (target, _) = tone_levels(x, rate, target_hz);
    let (other, _) = tone_levels(x, rate, other_hz);
    println!(
        "[fft:{label}] target({target_hz})={target:.1}dB other({other_hz})={other:.1}dB diff={:.1}dB",
        other - target
    );
    assert!(
        other - target <= TONE_ISOLATED_DB,
        "{label}：{other_hz}Hz 不应泄漏进隔离轨（相对 {target_hz}Hz 差 {:.1}dB）",
        other - target
    );
}

// ─────────────────────────── 测试图案窗口（物理像素） ───────────────────────────

static PATTERN_REGION: std::sync::OnceLock<Mutex<Option<(i32, i32, u32, u32)>>> =
    std::sync::OnceLock::new();

fn pattern_region_slot() -> &'static Mutex<Option<(i32, i32, u32, u32)>> {
    PATTERN_REGION.get_or_init(|| Mutex::new(None))
}

static PATTERN_CLASS: std::sync::Once = std::sync::Once::new();

unsafe extern "system" fn pattern_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc: HDC = BeginPaint(hwnd, &mut ps);
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            // 选区外纯蓝。
            let blue: HBRUSH = CreateSolidBrush(COLORREF(0x00FF_0000)); // BGR: 蓝
            let red: HBRUSH = CreateSolidBrush(COLORREF(0x0000_00FF)); // BGR: 红
            let _ = FillRect(hdc, &client, blue);
            if let Ok(region) = pattern_region_slot().lock() {
                if let Some((x, y, w, h)) = *region {
                    let rr = RECT {
                        left: x,
                        top: y,
                        right: x + w as i32,
                        bottom: y + h as i32,
                    };
                    let _ = FillRect(hdc, &rr, red);
                }
            }
            let _ = DeleteObject(HGDIOBJ(blue.0));
            let _ = DeleteObject(HGDIOBJ(red.0));
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn register_pattern_class() {
    PATTERN_CLASS.call_once(|| unsafe {
        let hinst = GetModuleHandleW(None).ok().expect("GetModuleHandleW");
        let class_name = HSTRING::from("SR_M18_PATTERN_WND");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(pattern_wndproc),
            hInstance: hinst.into(),
            lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
            ..Default::default()
        };
        let atom = RegisterClassW(&wc);
        assert_ne!(atom, 0, "RegisterClassW 失败");
    });
}

struct PatternWindow {
    hwnd: u64,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Drop for PatternWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = PostMessageW(
                Some(HWND(self.hwnd as usize as *mut _)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            );
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

/// 创建顶层纯色图案窗口：整个窗口蓝，`region_rel`（窗口内物理像素）纯红。
fn spawn_pattern_window(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    region_rel: (i32, i32, u32, u32),
) -> PatternWindow {
    register_pattern_class();
    *pattern_region_slot().lock().expect("pattern region 锁") = Some(region_rel);
    let (tx, rx) = mpsc::channel::<u64>();
    let join = std::thread::Builder::new()
        .name("sr-m18-pattern".into())
        .spawn(move || unsafe {
            let hinst = GetModuleHandleW(None).ok().expect("GetModuleHandleW");
            let class = HSTRING::from("SR_M18_PATTERN_WND");
            let title = HSTRING::from("SR_M18_PATTERN");
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(WS_EX_TOPMOST.0 | WS_EX_TOOLWINDOW.0),
                PCWSTR::from_raw(class.as_ptr()),
                PCWSTR::from_raw(title.as_ptr()),
                WS_POPUP | WS_VISIBLE,
                x,
                y,
                w,
                h,
                None,
                None,
                Some(hinst.into()),
                None,
            )
            .expect("CreateWindowExW 失败");
            let _ = ShowWindow(hwnd, SHOW_WINDOW_CMD(5)); // SW_SHOW
            let _ = UpdateWindow(hwnd);
            let _ = tx.send(hwnd.0 as usize as u64);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        })
        .expect("spawn pattern window 线程");
    let hwnd = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("图案窗口创建超时");
    std::thread::sleep(Duration::from_millis(300));
    PatternWindow {
        hwnd,
        join: Some(join),
    }
}

// ─────────────────────────── 显示器 / 配置 ───────────────────────────

fn enable_per_monitor_dpi() {
    // 已有感知上下文时可能返回 false；测试进程内只需第一次生效。
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

fn monitor0() -> sr_enum::monitors::MonitorEntry {
    let monitors = sr_enum::monitors::list_monitors()
        .unwrap_or_else(|e| blocked(&format!("显示器枚举失败：{e}")));
    cameras_ok_monitor(
        monitors
            .into_iter()
            .find(|m| m.index == 0)
            .unwrap_or_else(|| blocked("无显示器（index 0 缺失）")),
    )
}

fn cameras_ok_monitor(m: sr_enum::monitors::MonitorEntry) -> sr_enum::monitors::MonitorEntry {
    if m.width_px < 900 || m.height_px < 700 {
        blocked(&format!(
            "显示器 0 物理尺寸不足（{}x{}；区域用例需 ≥900x700）",
            m.width_px, m.height_px
        ));
    }
    m
}

fn display_key(m: &sr_enum::monitors::MonitorEntry) -> String {
    m.device_path
        .clone()
        .or_else(|| m.gdi_device_name.clone())
        .unwrap_or_else(|| blocked("显示器缺少稳定 device_path/GDI szDevice"))
}

/// 图案窗口 + 区域矩形（桌面物理坐标；选区完全在红块内）。
/// 返回 (窗口守卫, 区域矩形)。
fn pattern_setup(m: &sr_enum::monitors::MonitorEntry) -> (PatternWindow, RegionRect) {
    let win_x = m.x + 100;
    let win_y = m.y + 100;
    let win_w = 900;
    let win_h = 600;
    // 选区相对窗口 (100,100) 402x300（吸附后不变；非 16:9）。
    let region_rel = (100, 100, 402u32, 300u32);
    let win = spawn_pattern_window(win_x, win_y, win_w, win_h, region_rel);
    let rect = RegionRect {
        x: win_x + region_rel.0,
        y: win_y + region_rel.1,
        w: region_rel.2 + 1, // 403 → 向下取偶 402
        h: region_rel.3 + 1, // 301 → 向下取偶 300
    };
    (win, rect)
}

fn region_cfg(
    out: &Path,
    m: &sr_enum::monitors::MonitorEntry,
    rect: RegionRect,
    audio: Vec<AudioSourceSpec>,
) -> MultiRouteConfig {
    MultiRouteConfig {
        target: RouteTarget::Region {
            display_device: display_key(m),
            rect,
        },
        // canvas 故意给 1920x1080：区域路必须忽略它，以选区尺寸编码。
        video: VideoCfg {
            canvas_w: 1920,
            canvas_h: 1080,
            fps: 30,
            cursor: false,
        },
        audio,
        output_dir: out.to_path_buf(),
    }
}

fn start_one(
    cfg: MultiRouteConfig,
    rate: u32,
    keep: bool,
    container: OutputContainer,
) -> Result<RouteHandle, RouteError> {
    start_routes_with_options_rate_and_container(vec![cfg], None, keep, rate, container)
        .map(|mut v| v.remove(0))
}

fn start_or_blocked(
    cfg: MultiRouteConfig,
    rate: u32,
    keep: bool,
    container: OutputContainer,
) -> RouteHandle {
    match start_one(cfg, rate, keep, container) {
        Ok(h) => h,
        Err(e @ RouteError::EncoderUnavailable)
        | Err(e @ RouteError::ConsentDenied)
        | Err(e @ RouteError::LoopbackUnavailable { .. }) => {
            blocked(&format!("启动环境缺失：{e:?}"))
        }
        Err(e) => panic!("区域路由启动应成功（得到 {e:?}）"),
    }
}

fn stop_ok(handle: RouteHandle, label: &str) -> sr_pipeline::route::Finalized {
    handle.request_stop();
    handle
        .wait_finalized(Duration::from_secs(240))
        .unwrap_or_else(|e| panic!("{label} 收尾应成功（得到 {e:?}）"))
}

// ─────────────────────────── 音调源（进程/系统） ───────────────────────────

fn gen_tone_wav(dir: &Path, name: &str, hz: u32, secs: u32) -> PathBuf {
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
            &format!("sine=frequency={hz}:sample_rate=48000:duration={secs},volume=0.25"),
            "-ac",
            "2",
            "-c:a",
            "pcm_f32le",
        ])
        .arg(&out)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap_or_else(|e| blocked(&format!("无法生成测试音：{e}")));
    assert!(st.success(), "生成 {hz}Hz 测试音失败");
    out
}

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

fn default_render_id() -> String {
    sr_audio::wasapi::default_render_device_id().unwrap_or_else(|| blocked("无默认渲染端点"))
}

fn non_default_endpoint() -> Option<(String, String)> {
    let default = default_render_id().to_ascii_lowercase();
    sr_enum::render_endpoints::list_render_endpoints()
        .ok()
        .and_then(|list| {
            list.into_iter()
                .find(|e| e.is_active && e.device_id.to_ascii_lowercase() != default)
                .map(|e| (e.device_id, e.friendly_name))
        })
}

// ─────────────────────────── 用例：48k/96k 矩形裁剪 ───────────────────────────

fn region_rect_case(rate: u32, container: OutputContainer, tag: &str) {
    enable_per_monitor_dpi();
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();
    let m = monitor0();
    let root = fresh_dir(tag);
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("out");

    let (_win, rect) = pattern_setup(&m);
    let handle = start_or_blocked(
        region_cfg(&out, &m, rect, vec![AudioSourceSpec::System]),
        rate,
        false,
        container,
    );
    // 静止桌面保持 8s：无新帧不等于失效（不得产生 stop_notice）。
    std::thread::sleep(Duration::from_secs(8));
    let fin = stop_ok(handle, "区域矩形");

    assert!(
        fin.stop_notice.is_none(),
        "静止桌面/无新帧不得判失效（得到 {:?}）",
        fin.stop_notice
    );
    assert!(fin.final_path.exists(), "成品应存在");
    assert_eq!(
        probe_video_stream(&fin.final_path),
        ("h264".to_string(), 402, 300),
        "REQ-21/22：输出尺寸 = 选区物理像素向下取偶（canvas 不生效）"
    );
    let dur = probe_duration(&fin.final_path);
    assert!((7.0..=12.0).contains(&dur), "时长应≈8s（得到 {dur:.2}）");
    assert!(decode_clean(&fin.final_path), "整文件应可解码");
    assert_frame_is_red(&fin.final_path, 1.0);
    assert_frame_is_red(&fin.final_path, 5.0);

    let streams = probe_all_audio_streams(&fin.final_path);
    assert_eq!(streams.len(), 1, "System 应恰 1 音轨（得到 {streams:?}）");
    assert_eq!(streams[0].1, rate, "成品音轨采样率 = 设置输出率");
    if container == OutputContainer::Mp4 {
        assert_mp4_family(&fin.final_path);
    } else {
        assert_mkv_family(&fin.final_path);
    }
    assert!(
        !fin.segment_list.parent().expect("会话目录").exists(),
        "keep=false：成功后应清理原段"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "L2：交互式会话 + WGC/编码器；区域 48k 尺寸/裁剪内容/静止桌面不失效"]
fn it_region_video_rect_48k() {
    region_rect_case(48_000, OutputContainer::Mkv, "rect48");
}

#[test]
#[ignore = "L2：交互式会话 + WGC/编码器；区域 96k 尺寸/裁剪内容"]
fn it_region_video_rect_96k() {
    region_rect_case(96_000, OutputContainer::Mkv, "rect96");
}

#[test]
#[ignore = "L2：区域录制 MP4 成品（容器族/尺寸/解码）"]
fn it_region_container_mp4() {
    region_rect_case(48_000, OutputContainer::Mp4, "mp4");
}

// ─────────────────────────── 用例：尺寸拒绝 ───────────────────────────

struct ProbeOverride;
impl Drop for ProbeOverride {
    fn drop(&mut self) {
        set_region_size_probe_override_for_tests(None);
    }
}

#[test]
#[ignore = "L2：编码器不支持区域尺寸 → 启动失败（不缩放/补边）"]
fn it_region_size_unsupported_rejects() {
    enable_per_monitor_dpi();
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();
    let m = monitor0();
    let root = fresh_dir("unsupported");
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("out");
    let rect = RegionRect {
        x: m.x + 100,
        y: m.y + 100,
        w: 400,
        h: 300,
    };

    // 注入“编码器不支持该尺寸”，断言启动失败且不产生任何段/成品。
    let _override = ProbeOverride;
    set_region_size_probe_override_for_tests(Some(false));
    let err = match start_one(
        region_cfg(&out, &m, rect, vec![AudioSourceSpec::System]),
        48_000,
        false,
        OutputContainer::Mkv,
    ) {
        Ok(_) => panic!("编码器不支持尺寸必须启动失败"),
        Err(e) => e,
    };
    assert!(
        matches!(
            err,
            RouteError::RegionSizeUnsupported {
                width: 400,
                height: 300
            }
        ),
        "应返回 RegionSizeUnsupported（得到 {err:?}）"
    );
    let leftovers: Vec<PathBuf> = std::fs::read_dir(&out)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    assert!(
        leftovers.is_empty(),
        "不得缩放/补边或产生半成品（得到 {leftovers:?}）"
    );
    assert!(
        !out.join(".screenrec_tmp").exists(),
        "启动失败不得留下会话目录"
    );

    // 最小尺寸 <64×64：几何拒绝。
    let err = match start_one(
        region_cfg(
            &out,
            &m,
            RegionRect {
                x: m.x,
                y: m.y,
                w: 63,
                h: 100,
            },
            vec![AudioSourceSpec::System],
        ),
        48_000,
        false,
        OutputContainer::Mkv,
    ) {
        Ok(_) => panic!("<64×64 必须拒绝"),
        Err(e) => e,
    };
    assert!(
        matches!(err, RouteError::TargetInvalid { .. }),
        "应返回 TargetInvalid（得到 {err:?}）"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ─────────────────────────── 用例：失效 / 闩锁 / 并发 ───────────────────────────

struct HookReset;
impl Drop for HookReset {
    fn drop(&mut self) {
        set_region_latch_hook_for_tests(None);
    }
}

#[test]
#[ignore = "L2：REQ-24 闩锁交错（worker 读空→生产者闩锁→capture ended→重读；成功/收尾失败）"]
fn it_region_latch_interleaving() {
    enable_per_monitor_dpi();
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();
    let m = monitor0();
    let root = fresh_dir("latch");
    let rect = RegionRect {
        x: m.x + 100,
        y: m.y + 100,
        w: 400,
        h: 300,
    };
    let _reset = HookReset;

    for (label, fail_finalize) in [("success", false), ("finalize-failed", true)] {
        let out = root.join(label);
        std::fs::create_dir_all(&out).expect("out");
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let e = entered.clone();
        let r = release.clone();
        set_region_latch_hook_for_tests(Some(Arc::new(move || {
            e.wait();
            r.wait();
        })));

        let handle = start_or_blocked(
            region_cfg(&out, &m, rect, vec![AudioSourceSpec::System]),
            48_000,
            false,
            OutputContainer::Mkv,
        );
        handle.force_capture_ended_for_tests();
        entered.wait(); // worker 已读空闩锁并停在收尾判定前
        handle.latch_region_loss_for_tests("显示器已移除（屏障注入）");
        if fail_finalize {
            handle.force_finalize_failure_for_tests();
        }
        release.wait();

        let result = handle.wait_finalized(Duration::from_secs(240));
        match (fail_finalize, &result) {
            (false, Ok(fin)) => {
                let notice = fin.stop_notice.as_ref().expect("成功分支必须保留 notice");
                assert_eq!(notice.code, "E_TARGET_LOST", "{label}");
                assert!(notice.detail.contains("屏障注入"), "{label}: {notice:?}");
            }
            (true, Err(RouteError::FinalizeFailed { .. })) => {
                let notice = handle.stop_notice().expect("收尾失败分支仍须保留 notice");
                assert_eq!(notice.code, "E_TARGET_LOST", "{label}");
                assert!(notice.detail.contains("屏障注入"), "{label}: {notice:?}");
            }
            other => panic!("{label}：结果不符合预期（{other:?}）"),
        }
        println!("[region-latch] {label} OK");
        set_region_latch_hook_for_tests(None);
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "L2：区域失效保留其它路（E_TARGET_LOST + notice；窗口路继续）"]
fn it_region_display_lost_keeps_others() {
    enable_per_monitor_dpi();
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();
    let m = monitor0();
    let root = fresh_dir("lost");
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("out");
    let _win = ProbeWindow::new("SR_M18_OTHER_WINDOW");
    let window_hwnd = _win.hwnd_u64;

    let region = start_or_blocked(
        region_cfg(
            &out,
            &m,
            RegionRect {
                x: m.x + 100,
                y: m.y + 100,
                w: 400,
                h: 300,
            },
            vec![AudioSourceSpec::System],
        ),
        48_000,
        false,
        OutputContainer::Mkv,
    );
    let other = start_or_blocked(
        MultiRouteConfig {
            target: RouteTarget::Window { hwnd: window_hwnd },
            video: VideoCfg {
                canvas_w: 640,
                canvas_h: 360,
                fps: 30,
                cursor: false,
            },
            audio: Vec::new(),
            output_dir: out.clone(),
        },
        48_000,
        false,
        OutputContainer::Mkv,
    );

    std::thread::sleep(Duration::from_secs(3));
    region.force_capture_ended_for_tests();
    region.latch_region_loss_for_tests("显示器已移除（注入）");
    let fin = region
        .wait_finalized(Duration::from_secs(240))
        .unwrap_or_else(|e| panic!("失效区域路收尾应成功（得到 {e:?}）"));
    let notice = fin.stop_notice.as_ref().expect("失效必须保留 notice");
    assert_eq!(notice.code, "E_TARGET_LOST");
    assert!(fin.final_path.exists(), "已录内容应保留并收尾");

    // 其它路不受影响：继续一段时间后正常停止。
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        other.stop_notice().is_none(),
        "其它路不得被区域失效连带停止"
    );
    let other_fin = stop_ok(other, "窗口路");
    assert!(other_fin.final_path.exists());
    assert!(other_fin.stop_notice.is_none());
    let _ = std::fs::remove_dir_all(&root);
}

// ─────────────────────────── 用例：强杀恢复 ───────────────────────────

const CHILD_ENV: &str = "SR_M18_REGION_CHILD";
const CHILD_OUTPUT_ENV: &str = "SR_M18_REGION_CHILD_OUTPUT";
const CHILD_MARKER_ENV: &str = "SR_M18_REGION_CHILD_MARKER";
const CHILD_TEST: &str = "it_region_video_recovery";

fn child_region_main(output_dir: &Path, marker: &Path) -> ! {
    enable_per_monitor_dpi();
    let m = monitor0();
    let d = chrono_like_stamp();
    let _ = std::fs::create_dir_all(output_dir);
    let handle = start_one(
        region_cfg(
            output_dir,
            &m,
            RegionRect {
                x: m.x + 100,
                y: m.y + 100,
                w: 400,
                h: 300,
            },
            vec![AudioSourceSpec::System],
        ),
        48_000,
        true,
        OutputContainer::Mkv,
    );
    match handle {
        Ok(handle) => {
            let _ = std::fs::write(marker, format!("ok {d} {}", std::process::id()));
            println!("[child] 区域录制已启动；阻塞在 wait_finalized…");
            let _ = handle.wait_finalized(Duration::from_secs(600));
            std::process::exit(0);
        }
        Err(e) => {
            let _ = std::fs::write(marker, format!("err {e:?}"));
            std::process::exit(2);
        }
    }
}

fn chrono_like_stamp() -> String {
    format!("{}", std::process::id())
}

fn wait_marker(marker: &Path) -> String {
    let content = wait_until(Duration::from_secs(30), Duration::from_millis(200), || {
        std::fs::read_to_string(marker)
            .ok()
            .filter(|s| !s.trim().is_empty())
    })
    .unwrap_or_else(|| panic!("录制子进程就绪超时（{}）", marker.display()));
    content.trim().to_string()
}

#[test]
#[ignore = "L2：强杀区域录制宿主子进程 → 恢复 → 尺寸/全部流/解码正确"]
fn it_region_video_recovery() {
    if std::env::var(CHILD_ENV).as_deref() == Ok("record") {
        let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
        let output = PathBuf::from(std::env::var(CHILD_OUTPUT_ENV).expect("输出目录"));
        let marker = PathBuf::from(std::env::var(CHILD_MARKER_ENV).expect("标记"));
        child_region_main(&output, &marker);
    }

    enable_per_monitor_dpi();
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();
    let root = fresh_dir("recovery");
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("out");
    let marker = root.join("child.marker");
    let exe = std::env::current_exe().expect("current_exe");
    let log = std::fs::File::create(root.join("child.log")).expect("child log");
    let mut child = Command::new(exe)
        .args(["--exact", CHILD_TEST, "--ignored", "--nocapture"])
        .env(CHILD_ENV, "record")
        .env(CHILD_OUTPUT_ENV, &out)
        .env(CHILD_MARKER_ENV, &marker)
        .stdout(Stdio::from(log.try_clone().expect("log clone")))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("spawn 区域录制子进程");
    let content = wait_marker(&marker);
    if content.starts_with("err ") {
        let _ = child.kill();
        let _ = child.wait();
        blocked(&format!("区域录制子进程环境缺失：{content}"));
    }
    std::thread::sleep(Duration::from_secs(14));
    let pid = child.id();
    let st = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("taskkill");
    assert!(st.success(), "taskkill /T /F 失败");
    let _ = child.wait();
    std::thread::sleep(Duration::from_millis(1500));

    let sessions = recovery::scan_sessions(&out);
    assert!(!sessions.is_empty(), "强杀后应发现可恢复会话");
    let session = &sessions[0];
    assert!(!session.active, "宿主死亡后会话不应仍活动");
    let outcome = recovery::finalize_session_with_options_and_container(
        &session.dir,
        false,
        OutputContainer::Mkv,
    )
    .unwrap_or_else(|e| panic!("区域恢复收尾失败：{e:?}"));
    assert_eq!(outcome.route_finals.len(), 1, "应恢复 1 路");
    let (_, path) = &outcome.route_finals[0];
    assert!(path.exists(), "恢复成品应存在");
    assert_eq!(
        probe_video_stream(path),
        ("h264".to_string(), 400, 300),
        "REQ-24：恢复成品尺寸 = 区域尺寸"
    );
    let audio = probe_all_audio_streams(path);
    assert_eq!(audio.len(), 1, "全部流（1 音轨）应恢复（得到 {audio:?}）");
    assert!(decode_clean(path), "恢复成品应可解码");
    let _ = std::fs::remove_dir_all(&root);
}

// ─────────────────────────── 用例：异构尺寸合并 / 并发 ───────────────────────────

#[test]
#[ignore = "L2：区域 + 窗口并发录制（两路独立收尾）"]
fn it_region_concurrent_window() {
    enable_per_monitor_dpi();
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();
    let m = monitor0();
    let root = fresh_dir("concurrent");
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("out");
    let win = ProbeWindow::new("SR_M18_CONCURRENT");
    let region = start_or_blocked(
        region_cfg(
            &out,
            &m,
            RegionRect {
                x: m.x + 100,
                y: m.y + 100,
                w: 400,
                h: 300,
            },
            vec![AudioSourceSpec::System],
        ),
        48_000,
        false,
        OutputContainer::Mkv,
    );
    let window = start_or_blocked(
        MultiRouteConfig {
            target: RouteTarget::Window { hwnd: win.hwnd_u64 },
            video: VideoCfg {
                canvas_w: 640,
                canvas_h: 360,
                fps: 30,
                cursor: false,
            },
            audio: Vec::new(),
            output_dir: out.clone(),
        },
        48_000,
        false,
        OutputContainer::Mkv,
    );
    std::thread::sleep(Duration::from_secs(6));
    let rf = stop_ok(region, "区域路");
    let wf = stop_ok(window, "窗口路");
    assert_eq!(
        probe_video_stream(&rf.final_path),
        ("h264".to_string(), 400, 300)
    );
    assert_eq!(
        probe_video_stream(&wf.final_path),
        ("h264".to_string(), 640, 360)
    );
    assert!(decode_clean(&rf.final_path) && decode_clean(&wf.final_path));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "L2：异构尺寸区域成品合并（输出 MKV / 两条视频流）"]
fn it_region_merge() {
    enable_per_monitor_dpi();
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();
    let m = monitor0();
    let root = fresh_dir("merge");
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("out");
    let (_win, rect) = pattern_setup(&m);

    let mut inputs = Vec::new();
    for (i, (w, h)) in [(402u32, 300u32), (320, 240)].into_iter().enumerate() {
        let r = RegionRect {
            x: rect.x,
            y: rect.y,
            w,
            h,
        };
        let handle = start_or_blocked(
            region_cfg(&out, &m, r, vec![AudioSourceSpec::System]),
            48_000,
            false,
            OutputContainer::Mkv,
        );
        std::thread::sleep(Duration::from_secs(4));
        let fin = stop_ok(handle, &format!("合并输入 {i}"));
        inputs.push(fin.final_path);
    }
    let merged = out.join("merged.mkv");
    sr_merge::merge::merge_recordings(sr_merge::merge::MergeConfig {
        inputs: inputs.clone(),
        out: merged.clone(),
    })
    .unwrap_or_else(|e| panic!("区域成品合并失败：{e:?}"));
    assert_mkv_family(&merged);
    let dims = probe_all_video_streams(&merged);
    assert_eq!(dims.len(), 2, "合并应保留 2 条视频流（得到 {dims:?}）");
    assert!(dims.contains(&(402, 300)) && dims.contains(&(320, 240)));
    assert!(decode_clean(&merged));
    let _ = std::fs::remove_dir_all(&root);
}

// ─────────────────────────── 用例：音轨矩阵（REQ-23） ───────────────────────────

#[test]
#[ignore = "L2：区域音轨矩阵（隔离仅应用/全音轨系统+应用/端点去重/零音源拒绝）"]
fn it_region_video_audio_modes() {
    enable_per_monitor_dpi();
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();
    let m = monitor0();
    let root = fresh_dir("audio-modes");
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("out");
    let (_win, rect) = pattern_setup(&m);
    let tone_dir = root.join("tones");
    std::fs::create_dir_all(&tone_dir).expect("tones");
    let target_wav = gen_tone_wav(&tone_dir, "app660.wav", 660, 30);
    let system_wav = gen_tone_wav(&tone_dir, "system880.wav", 880, 30);

    // ── 隔离：仅所选应用；系统 880Hz 不得泄漏 ──
    let app = spawn_ffplay(&target_wav);
    std::thread::sleep(Duration::from_millis(600));
    let system = spawn_ffplay(&system_wav);
    std::thread::sleep(Duration::from_millis(600));
    let created =
        process_created_unix_ms(app.0.id()).unwrap_or_else(|| blocked("无法读取 ffplay 创建时间"));
    let isolated = start_or_blocked(
        region_cfg(
            &out,
            &m,
            rect,
            vec![AudioSourceSpec::App {
                pid: app.0.id(),
                created_at_unix_ms: created,
            }],
        ),
        48_000,
        true,
        OutputContainer::Mkv,
    );
    std::thread::sleep(Duration::from_secs(8));
    let iso_fin = stop_ok(isolated, "区域隔离");
    let streams = probe_all_audio_streams(&iso_fin.final_path);
    assert_eq!(streams.len(), 1, "隔离=仅应用轨（得到 {streams:?}）");
    assert_tone_present(
        &decode_audio_mono_at(&iso_fin.final_path, 0, 48_000),
        48_000,
        660.0,
        "region-isolated-app",
    );
    assert_tone_not_present(
        &decode_audio_mono_at(&iso_fin.final_path, 0, 48_000),
        48_000,
        660.0,
        880.0,
        "region-isolated-no-system-leak",
    );

    // ── 全音轨：系统 + 应用（+ 端点去重） ──
    let endpoint = non_default_endpoint();
    let mut sources = vec![AudioSourceSpec::System];
    if let Some((id, name)) = &endpoint {
        println!("[region-audio] 追加非默认端点：{name} ({id})");
        sources.insert(
            0,
            AudioSourceSpec::RenderEndpoint {
                device_id: id.clone(),
            },
        );
    }
    sources.push(AudioSourceSpec::App {
        pid: app.0.id(),
        created_at_unix_ms: created,
    });
    let expected_audio = sources.len();
    let full = start_or_blocked(
        region_cfg(&out, &m, rect, sources),
        48_000,
        true,
        OutputContainer::Mkv,
    );
    std::thread::sleep(Duration::from_secs(8));
    let full_fin = stop_ok(full, "区域全音轨");
    let streams = probe_all_audio_streams(&full_fin.final_path);
    assert_eq!(
        streams.len(),
        expected_audio,
        "全音轨轨数应等于所选来源（端点≠系统；得到 {streams:?}）"
    );
    let sys_idx = if endpoint.is_some() { 1 } else { 0 };
    assert_tone_present(
        &decode_audio_mono_at(&full_fin.final_path, sys_idx, 48_000),
        48_000,
        880.0,
        "region-all-system-880",
    );
    assert_tone_present(
        &decode_audio_mono_at(&full_fin.final_path, sys_idx, 48_000),
        48_000,
        660.0,
        "region-all-system-has-app-660",
    );
    let app_idx = streams.len() - 1;
    assert_tone_present(
        &decode_audio_mono_at(&full_fin.final_path, app_idx, 48_000),
        48_000,
        660.0,
        "region-all-app",
    );
    assert_tone_not_present(
        &decode_audio_mono_at(&full_fin.final_path, app_idx, 48_000),
        48_000,
        660.0,
        880.0,
        "region-all-app-no-system-leak",
    );

    // ── 端点 == 系统（默认端点）→ 去重为单轨 ──
    let default_id = default_render_id();
    let dedup = start_or_blocked(
        region_cfg(
            &out,
            &m,
            rect,
            vec![
                AudioSourceSpec::RenderEndpoint {
                    device_id: default_id,
                },
                AudioSourceSpec::System,
            ],
        ),
        48_000,
        false,
        OutputContainer::Mkv,
    );
    std::thread::sleep(Duration::from_secs(4));
    let dedup_fin = stop_ok(dedup, "区域端点去重");
    let streams = probe_all_audio_streams(&dedup_fin.final_path);
    assert_eq!(
        streams.len(),
        1,
        "端点==系统应去重为单轨（得到 {streams:?}）"
    );

    // ── 麦克风：缺设备仅 BLOCKED 该分项（不阻断其它断言） ──
    match sr_audio::wasapi::list_capture_devices() {
        Ok(devices) if !devices.is_empty() => {
            let mic = start_or_blocked(
                region_cfg(
                    &out,
                    &m,
                    rect,
                    vec![
                        AudioSourceSpec::System,
                        AudioSourceSpec::Mic {
                            device: devices[0].id.clone(),
                        },
                    ],
                ),
                48_000,
                false,
                OutputContainer::Mkv,
            );
            std::thread::sleep(Duration::from_secs(4));
            let mic_fin = stop_ok(mic, "区域麦克风");
            let streams = probe_all_audio_streams(&mic_fin.final_path);
            assert_eq!(streams.len(), 2, "系统+麦克风 → 2 轨（得到 {streams:?}）");
        }
        _ => println!("[region-audio] BLOCKED 子项：本机无采集设备（麦克风分项不参与判定）"),
    }

    // ── 零音源：管线侧拒绝 ──
    let err = match start_one(
        region_cfg(&out, &m, rect, Vec::new()),
        48_000,
        false,
        OutputContainer::Mkv,
    ) {
        Ok(_) => panic!("区域零音源必须拒绝"),
        Err(e) => e,
    };
    assert!(
        matches!(err, RouteError::TargetInvalid { .. }),
        "应返回 TargetInvalid（得到 {err:?}）"
    );

    drop(system);
    drop(app);
    let _ = std::fs::remove_dir_all(&root);
}
