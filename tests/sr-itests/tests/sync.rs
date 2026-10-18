//! L2 半自动内容级测试：时间轴同步（REQ-25）与固定画布 resize（REQ-13）
//! （specs/001-screen-recorder.md §2.3 REQ-13、§2.6 REQ-25、§4 测试映射、§8 M0 冻结
//! S6/S1；docs/milestones/m10-rehearsal-report.md §7-1/§7-2 缺口补齐）。
//!
//! 运行方式（L2/L3 门：`#[ignore]`，默认 `cargo test` 不执行）：
//! ```text
//! cargo test -p sr-itests --test sync -- --ignored --nocapture --test-threads=1
//! cargo test -p sr-itests --test sync it_sync_content_10min -- --ignored --nocapture
//! cargo test -p sr-itests --test sync it_window_resize_canvas_stable -- --ignored --nocapture
//! ```
//! 前置条件：交互式用户会话 + WGC + 可用编码器（D4 链）+ 锁定 ffmpeg n8.1（§8）；
//! 缺失 → BLOCKED（fail），不静默 skip（§4）。
//!
//! ## it_sync_content_10min（REQ-25 内容级 ≤150ms 且不累积）
//!
//! 参考源（**测试进程内**，单一内容时钟）：WASAPI 渲染线程向默认渲染端点写 48kHz/f32
//! 短音（每 1.000s 一次 80ms 880Hz）；GDI 内容时钟窗口绘制 **2s 周期横向秒表条**
//! （640x360，条宽 20px、行程 620px）。两者由**同一原子内容时钟**（已写出样本数）驱动
//! → 源信号本身严格同刻，不依赖外部播放器/屏幕呈现时序。录制 **610s**（≥10 分钟）。
//! 调试/快速复现可用 `SR_SYNC_RECORD_SECS=<秒>` 覆盖录制时长（默认 610；覆盖率阈值
//! 按同一比例缩放，610s 时与冻结阈值逐项一致）。
//! 另先用锁定 ffmpeg 生成的同结构参考片跑一遍解码/相位自检（§4.2：19.1ms/31.7ms）。
//!
//! 测量（内容时钟法，对捕获帧率不敏感——本机 WGC 有效更新率实测约 3.8–4.5 帧/s，
//! 100ms 短闪的 onset 无法分辨 150ms；见 docs/milestones/m10b-sync-report.md）：
//! - 视频：解码成片为 64x36 灰度帧，求亮条列加权中心 `pos` → 源时间相位
//!   `φ_v = (pos × 2s) mod 1s`（2s 秒表条，相位即源时间 mod 1s）；
//! - 音频：解码音轨 8kHz，5ms RMS 包络检短音 onset；成片时刻 t 的源时间相位
//!   `φ_a = (t − 前一个短音 onset) mod 1s`（短音每 1s 一次）；
//! - 每帧偏差 `offset = wrap1s(φ_a − φ_v)`（取 ±0.5s 主值；150ms 门槛下无歧义）。
//! 只取**内容发生变化**的帧（新鲜捕获帧），避免 CFR 重复帧把旧内容时间重复计入。
//!
//! 断言（硬阈值，不削弱）：
//! - 任意样本 `|offset| ≤ 150ms`（REQ-25 最大内容偏差）；
//! - 不累积：末 20% 与首 20% 均值差 `≤ 100ms`，线性斜率 `|slope| ≤ 0.20 ms/s`；
//! - concat 边界：落在 5s 段边界 ±0.5s 的样本同样 `|offset| ≤ 150ms`（且覆盖 ≥100 个）；
//! - 覆盖率：短音 ≥580、内容时钟样本 ≥1000（采集停滞会暴露为覆盖率不足）；
//! - 成片形状：1 视频轨（h264/1080p/30）+ 1 音轨（aac/48k/立体声）、时长≈录制段、整文件可解码。
//!
//! 说明：视频内容时间量化到源 1/30s（±33ms），秒表条在 64px 采样下 ≈2ms/px，短音 onset
//! 精度 ≈5ms；渲染端点缓冲与采集链路延迟计入测量值（常量；门槛 150ms）。
//!
//! ## it_window_resize_canvas_stable（REQ-13 固定画布）
//!
//! 录制中改变目标窗口尺寸与比例（640x480 → 800x600 → 960x360）、最小化 2s、恢复，最后
//! 正常停止。断言：输出画布恒为配置值（1280x720@30）、所有分段参数一致且各自可独立解码、
//! 成片时长覆盖整个（含最小化）录制段、抽样帧全程非黑、成片可解码。

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Once, OnceLock};
use std::time::{Duration, Instant};

use common::*;
use sr_audio::wasapi::AudioSourceSpec;
use sr_pipeline::route::{
    start_audio_route, start_route_with_options, AudioRouteConfig, Finalized, RouteConfig,
    RouteError, RouteHandle, RouteTarget, VideoCfg,
};

/// L2 用例串行门（同步用例 10 分钟 + resize 用例各自独占用例机；沿用 M4–M9 约定）。
static GATE: Mutex<()> = Mutex::new(());

fn gate() -> MutexGuard<'static, ()> {
    GATE.lock().unwrap_or_else(|e| e.into_inner())
}

// ─────────────────────────── 锁定构建 / 目录 ───────────────────────────

fn ffbin(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin")
        .join(name)
        .canonicalize()
        .unwrap_or_else(|e| {
            blocked(&format!(
                "无法定位锁定 n8.1 构建的 {name}.exe（canonicalize 失败：{e}；spec §8 冻结 sidecar 构建）"
            ))
        });
    if !p.exists() {
        blocked(&format!("锁定 ffmpeg n8.1 构建缺失：{}", p.display()));
    }
    p
}

fn fresh_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr_m10b_sync_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("创建临时目录失败");
    d
}

/// `start_audio_route` 的 BLOCKED 包装（环境缺失 → BLOCKED；其它 Err → panic）。
fn start_audio_route_ok(cfg: AudioRouteConfig) -> RouteHandle {
    match start_audio_route(cfg) {
        Ok(h) => h,
        Err(e) if matches!(e, RouteError::EncoderUnavailable { .. }) => {
            blocked(&format!("无可用编码器（§9 T1）：{e:?}"))
        }
        Err(e) if matches!(e, RouteError::ConsentDenied { .. }) => {
            blocked(&format!("WGC 授权被拒（E_CONSENT_DENIED）：{e:?}"))
        }
        Err(e) if matches!(e, RouteError::LoopbackUnavailable { .. }) => {
            blocked(&format!("进程环回不可用（E_LOOPBACK_UNAVAILABLE）：{e:?}"))
        }
        Err(e) => panic!("start_audio_route 应成功（得到 {e:?}）"),
    }
}

/// M14：需检查原段的用例显式启用 keep_segments（默认成功后原段被清理）。
fn start_route_with_keep_ok(cfg: RouteConfig) -> RouteHandle {
    match start_route_with_options(cfg, true) {
        Ok(h) => h,
        Err(e) if matches!(e, RouteError::EncoderUnavailable { .. }) => {
            blocked(&format!("无可用编码器（§9 T1）：{e:?}"))
        }
        Err(e) if matches!(e, RouteError::ConsentDenied { .. }) => {
            blocked(&format!("WGC 授权被拒（E_CONSENT_DENIED）：{e:?}"))
        }
        Err(e) => panic!("start_route_with_options 应成功（得到 {e:?}）"),
    }
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

/// 首个视频流 (codec, width, height, fps)。
fn probe_video_stream(path: &Path) -> (String, u32, u32, f64) {
    let s = ffprobe_csv(
        path,
        &[
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name,width,height,avg_frame_rate",
            "-of",
            "csv=p=0",
        ],
    );
    let parts: Vec<&str> = s.split(',').collect();
    assert_eq!(parts.len(), 4, "视频流 csv 形状（得到 {s:?}）");
    let fps = parts[3]
        .split('/')
        .map(|v| v.trim().parse::<f64>().unwrap_or(0.0))
        .collect::<Vec<_>>();
    let fps = if fps.len() == 2 && fps[1] != 0.0 {
        fps[0] / fps[1]
    } else {
        fps.first().copied().unwrap_or(0.0)
    };
    (
        parts[0].trim().to_string(),
        parts[1].trim().parse().expect("width"),
        parts[2].trim().parse().expect("height"),
        fps,
    )
}

/// 首音轨 (codec, sample_rate, channels)。
fn probe_audio_stream(path: &Path) -> (String, u32, u32) {
    let s = ffprobe_csv(
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
    let parts: Vec<&str> = s.split(',').collect();
    assert_eq!(parts.len(), 3, "音轨 csv 形状（得到 {s:?}）");
    (
        parts[0].trim().to_string(),
        parts[1].trim().parse().expect("sample_rate"),
        parts[2].trim().parse().expect("channels"),
    )
}

/// 整文件可解码（ffmpeg -v error -f null -；stderr 必须为空）。
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

/// 逐帧灰度序列（指定缩放；内容时钟解码用）。
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
        .unwrap_or_else(|e| blocked(&format!("无法 spawn 锁定 ffmpeg 逐帧灰度解码：{e}")));
    assert!(
        out.status.success(),
        "逐帧灰度解码失败（exit={}；stderr={}）",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let fl = (w * h) as usize;
    assert!(
        !out.stdout.is_empty(),
        "逐帧灰度解码不得为空（{}）",
        path.display()
    );
    assert_eq!(
        out.stdout.len() % fl,
        0,
        "灰度字节流应为整帧倍数（{} 字节）",
        out.stdout.len()
    );
    out.stdout.chunks(fl).map(<[u8]>::to_vec).collect()
}

/// 首音轨 → 单声道 f32（指定采样率；同步分析用 8kHz 足够且省内存）。
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
        .unwrap_or_else(|e| blocked(&format!("无法 spawn 锁定 ffmpeg 音轨解码：{e}")));
    assert!(
        out.status.success(),
        "音轨解码失败（exit={}；stderr={}）",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout.len() % 4, 0, "f32 字节流应为 4 字节倍数");
    out.stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

// ─────────────────────────── 内容级检测：内容时钟（秒表条 + 短音） ───────────────────────────

/// 秒表条周期（源秒）。
const BAR_PERIOD_SECS: f64 = 2.0;
/// 源窗口/秒表条几何（参考窗口 640x360；条宽 20px；行程 620px）。
const BAR_SRC_W_PX: f64 = 640.0;
const BAR_W_PX: f64 = 20.0;
const BAR_TRAVEL_PX: f64 = 620.0;
/// 内容时钟解码尺寸（像素）。
const CLOCK_W: u32 = 64;
const CLOCK_H: u32 = 36;
/// 亮条列均值下限（源：深灰底 ~16、条 ~235）。
const BAR_MIN_LUMA: f64 = 100.0;
const BEEP_RMS_MIN: f32 = 0.05;
const BEEP_SAMPLE_MIN: f32 = 0.02;

/// 竖条位置（源坐标系 0..1，`0` = 条左缘在 x=0、`1` = 条左缘在 x=620）。
/// 按列均值求阈值以上列的加权中心（= 条中心），再减去半条宽映射回左缘。
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
    let centroid_out = num / den;
    let centroid_src = centroid_out * (BAR_SRC_W_PX / w as f64);
    Some(((centroid_src - BAR_W_PX / 2.0) / BAR_TRAVEL_PX).clamp(0.0, 1.0))
}

/// 音轨短音 onset（秒；5ms RMS 包络 + 回溯首个样本）。
fn detect_beep_onsets(samples: &[f32], rate: u32) -> Vec<f64> {
    let rate = rate as usize;
    let win = (rate / 200).max(1); // 5ms
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + win <= samples.len() {
        let rms = (samples[i..i + win].iter().map(|s| s * s).sum::<f32>() / win as f32).sqrt();
        if rms >= BEEP_RMS_MIN {
            // 回溯到起振样本（提高 onset 精度；窗口内的首样本）
            let lo = i.saturating_sub(win * 2);
            let mut onset = i;
            for k in lo..i {
                if samples[k].abs() > BEEP_SAMPLE_MIN {
                    onset = k;
                    break;
                }
            }
            out.push(onset as f64 / rate as f64);
            // 跳过本次短音（80ms）+ 余量，避免同一次短音重复计数
            i += rate / 10;
        } else {
            i += win;
        }
    }
    out
}

// ─────────────────────────── 动画窗口（resize 用例） ───────────────────────────

const ANIM_CLASS_NAME: &str = "SR_M10B_RESIZE_WND";
static ANIM_CLASS: Once = Once::new();
static ANIM_TICKS: AtomicU32 = AtomicU32::new(0);

fn register_anim_class() {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{RegisterClassW, WNDCLASSW};
    ANIM_CLASS.call_once(|| unsafe {
        let hinst = GetModuleHandleW(None).ok().expect("GetModuleHandleW");
        let class_name = windows::core::HSTRING::from(ANIM_CLASS_NAME);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(anim_wndproc),
            hInstance: hinst.into(),
            lpszClassName: windows::core::PCWSTR::from_raw(class_name.as_ptr()),
            ..Default::default()
        };
        let atom = RegisterClassW(&wc);
        assert_ne!(atom, 0, "RegisterClassW({ANIM_CLASS_NAME}) 失败");
    });
}

unsafe extern "system" fn anim_wndproc(
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
                let h = (rc.bottom - rc.top).max(1);
                let n = ANIM_TICKS.fetch_add(1, Ordering::Relaxed) as i32;

                let bg = CreateSolidBrush(COLORREF(45 | (70 << 8) | (110 << 16)));
                FillRect(hdc, &rc, bg);
                let _ = DeleteObject(HGDIOBJ(bg.0));

                let bar_w = (w / 7).max(24);
                let bar_x = (n * 29).rem_euclid(w);
                let bar = CreateSolidBrush(COLORREF(240 | (200 << 8) | (60 << 16)));
                let bar_rc = RECT {
                    left: bar_x,
                    top: 0,
                    right: (bar_x + bar_w).min(w),
                    bottom: h,
                };
                FillRect(hdc, &bar_rc, bar);
                let _ = DeleteObject(HGDIOBJ(bar.0));

                let blk = CreateSolidBrush(COLORREF(80 | (220 << 8) | (140 << 16)));
                let blk_rc = RECT {
                    left: w / 4,
                    top: h / 3,
                    right: (3 * w / 4).max(w / 4 + 32),
                    bottom: (2 * h / 3).max(h / 3 + 32),
                };
                FillRect(hdc, &blk_rc, blk);
                let _ = DeleteObject(HGDIOBJ(blk.0));

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

/// 自建动画窗口（专用泵线程 + 33ms 重绘；支持录制中改尺寸/最小化/恢复）。
struct AnimWindow {
    hwnd_u64: u64,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl AnimWindow {
    fn new(title: &str, x: i32, y: i32, w: i32, h: i32) -> AnimWindow {
        let (tx, rx) = std::sync::mpsc::channel();
        let title = title.to_string();
        let thread = std::thread::Builder::new()
            .name(format!("sr-m10b-anim-{title}"))
            .spawn(move || {
                use windows::Win32::System::LibraryLoader::GetModuleHandleW;
                use windows::Win32::UI::WindowsAndMessaging::{
                    CreateWindowExW, DispatchMessageW, GetMessageW, SetTimer, SetWindowPos,
                    TranslateMessage, HWND_TOPMOST, MSG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
                    WS_EX_TOPMOST, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
                };
                register_anim_class();
                unsafe {
                    let hinst = GetModuleHandleW(None).ok().expect("GetModuleHandleW");
                    let class_name = windows::core::HSTRING::from(ANIM_CLASS_NAME);
                    let title_w = windows::core::HSTRING::from(title.as_str());
                    let hwnd = CreateWindowExW(
                        WS_EX_TOPMOST,
                        windows::core::PCWSTR::from_raw(class_name.as_ptr()),
                        windows::core::PCWSTR::from_raw(title_w.as_ptr()),
                        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                        x,
                        y,
                        w,
                        h,
                        None,
                        None,
                        Some(hinst.into()),
                        None,
                    )
                    .expect("CreateWindowExW 失败：无法构造动画窗口");
                    let _ = SetWindowPos(
                        hwnd,
                        Some(HWND_TOPMOST),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                    SetTimer(Some(hwnd), 1, 33, None);
                    tx.send(hwnd.0 as usize as u64).expect("回传 hwnd");
                    let mut msg = MSG::default();
                    loop {
                        let r = GetMessageW(&mut msg, None, 0, 0);
                        if r.0 <= 0 {
                            break;
                        }
                        let _ = TranslateMessage(&msg);
                        let _ = DispatchMessageW(&msg);
                    }
                }
            })
            .expect("spawn 动画窗口线程");
        let hwnd_u64 = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("动画窗口创建超时（泵线程未回传 hwnd）");
        AnimWindow {
            hwnd_u64,
            thread: Some(thread),
        }
    }

    fn hwnd(&self) -> windows::Win32::Foundation::HWND {
        windows::Win32::Foundation::HWND(self.hwnd_u64 as usize as *mut core::ffi::c_void)
    }

    /// 改尺寸（保持左上角；置顶语义不变）。
    fn resize(&self, w: i32, h: i32) {
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE,
        };
        unsafe {
            let _ = SetWindowPos(
                self.hwnd(),
                Some(HWND_TOPMOST),
                0,
                0,
                w,
                h,
                SWP_NOMOVE | SWP_NOACTIVATE,
            );
        }
    }

    fn minimize(&self) {
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_MINIMIZE};
        unsafe {
            let _ = ShowWindow(self.hwnd(), SW_MINIMIZE);
        }
    }

    fn restore(&self) {
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_RESTORE};
        unsafe {
            let _ = ShowWindow(self.hwnd(), SW_RESTORE);
        }
    }
}

impl Drop for AnimWindow {
    fn drop(&mut self) {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
        unsafe {
            let _ = PostMessageW(Some(self.hwnd()), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

// ─────────────────────────── 测试内参考源：WASAPI 渲染 + 内容时钟窗口 ───────────────────────────

/// 参考源的单一内容时钟（微秒；渲染线程按**已写出样本**推进）。
///
/// 视频条与音频短音都由该时钟驱动 → 源信号本身严格同刻（不依赖外部播放器的呈现时序）。
static CONTENT_US: OnceLock<Arc<AtomicU64>> = OnceLock::new();

fn content_clock() -> Arc<AtomicU64> {
    CONTENT_US
        .get_or_init(|| Arc::new(AtomicU64::new(0)))
        .clone()
}

/// 渲染采样率（与管线音频 48kHz 一致）。
const TONE_RATE: u64 = 48_000;
const CLOCK_WIN_CLASS: &str = "SR_M10B_CLOCK_WND";
const CLOCK_WIN_W: i32 = 640;
const CLOCK_WIN_H: i32 = 360;

/// 测试进程内的 WASAPI 渲染源：每 1s 一次 80ms 880Hz 短音（内容时间 = 样本序号/48k）。
struct ToneSource {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ToneSource {
    fn start(clock: Arc<AtomicU64>) -> ToneSource {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let thread = std::thread::Builder::new()
            .name("sr-m10b-tone".into())
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
                                0.35 * (2.0 * std::f64::consts::PI * 880.0 * phase).sin()
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
            Ok(Ok(())) => {}
            Ok(Err(e)) => blocked(&format!(
                "测试内 WASAPI 渲染源初始化失败（§3.5 环境缺失）：{e}"
            )),
            Err(_) => blocked("测试内 WASAPI 渲染源 10s 未就绪（默认渲染端点不可用？）"),
        }
        ToneSource {
            stop,
            thread: Some(thread),
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

/// 内容时钟窗口：按 [`CONTENT_US`] 在 640x360 客户区绘制 2s 周期横向秒表条。
struct ClockWindow {
    hwnd_u64: u64,
    thread: Option<std::thread::JoinHandle<()>>,
}

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
                let h = (rc.bottom - rc.top).max(1);

                let bg = CreateSolidBrush(COLORREF(16 | (16 << 8) | (16 << 16)));
                FillRect(hdc, &rc, bg);
                let _ = DeleteObject(HGDIOBJ(bg.0));

                let us = CONTENT_US
                    .get()
                    .map(|c| c.load(Ordering::Acquire))
                    .unwrap_or(0);
                let secs = us as f64 / 1_000_000.0;
                let pos = (secs % BAR_PERIOD_SECS) / BAR_PERIOD_SECS; // 0..1
                let x0 = (pos * BAR_TRAVEL_PX) as i32;
                let bar = CreateSolidBrush(COLORREF(255 | (255 << 8) | (255 << 16)));
                let bar_rc = RECT {
                    left: x0,
                    top: 0,
                    right: (x0 + BAR_W_PX as i32).min(w),
                    bottom: h,
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

impl ClockWindow {
    fn new(title: &str, x: i32, y: i32) -> ClockWindow {
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DispatchMessageW, GetMessageW, RegisterClassW, SetTimer,
            TranslateMessage, MSG, WNDCLASSW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
        };
        static REG: Once = Once::new();
        REG.call_once(|| unsafe {
            let hinst = GetModuleHandleW(None).ok().expect("GetModuleHandleW");
            let cn = windows::core::HSTRING::from(CLOCK_WIN_CLASS);
            let wc = WNDCLASSW {
                lpfnWndProc: Some(clock_wndproc),
                hInstance: hinst.into(),
                lpszClassName: windows::core::PCWSTR::from_raw(cn.as_ptr()),
                ..Default::default()
            };
            assert_ne!(
                RegisterClassW(&wc),
                0,
                "RegisterClassW({CLOCK_WIN_CLASS}) 失败"
            );
        });
        let (tx, rx) = std::sync::mpsc::channel();
        let title = title.to_string();
        let thread = std::thread::Builder::new()
            .name(format!("sr-m10b-clock-win-{title}"))
            .spawn(move || unsafe {
                let hinst = GetModuleHandleW(None).ok().expect("GetModuleHandleW");
                let cn = windows::core::HSTRING::from(CLOCK_WIN_CLASS);
                let title_w = windows::core::HSTRING::from(title.as_str());
                let hwnd = CreateWindowExW(
                    WS_EX_TOPMOST,
                    windows::core::PCWSTR::from_raw(cn.as_ptr()),
                    windows::core::PCWSTR::from_raw(title_w.as_ptr()),
                    WS_POPUP | WS_VISIBLE,
                    x,
                    y,
                    CLOCK_WIN_W,
                    CLOCK_WIN_H,
                    None,
                    None,
                    Some(hinst.into()),
                    None,
                )
                .expect("CreateWindowExW 失败：无法构造内容时钟窗口");
                SetTimer(Some(hwnd), 1, 16, None);
                tx.send(hwnd.0 as usize as u64).expect("回传 hwnd");
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                    let _ = TranslateMessage(&msg);
                    let _ = DispatchMessageW(&msg);
                }
            })
            .expect("spawn 内容时钟窗口线程");
        let hwnd_u64 = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("内容时钟窗口创建超时");
        ClockWindow {
            hwnd_u64,
            thread: Some(thread),
        }
    }
}

impl Drop for ClockWindow {
    fn drop(&mut self) {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
        unsafe {
            let hwnd =
                windows::Win32::Foundation::HWND(self.hwnd_u64 as usize as *mut core::ffi::c_void);
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// 录制时长（REQ-25「10 分钟」；610s 留 10s 余量）。
///
/// 快速复现可用环境变量 `SR_SYNC_RECORD_SECS` 覆盖（默认 610，spec 口径不变）；
/// 覆盖率阈值按同一比例缩放（610s 时与冻结阈值逐项一致）。
const RECORD_SECS: u64 = 610;
/// 参考片时长（> 录制时长 + 播放启动余量）。
fn clip_secs() -> u64 {
    record_secs() + 50
}
fn record_secs() -> u64 {
    std::env::var("SR_SYNC_RECORD_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|s| *s >= 15)
        .unwrap_or(RECORD_SECS)
}
/// 覆盖率阈值按录制时长缩放（610s → 冻结值原样）。
fn scaled_threshold(base: usize) -> usize {
    ((base as f64) * (record_secs() as f64) / (RECORD_SECS as f64)).round() as usize
}
/// 画布（固定画布 D6；内容级同步与分辨率无关，取产品保底 1080p30）。
const SYNC_CANVAS: (u32, u32) = (1920, 1080);
const SYNC_FPS: u32 = 30;
/// 同步硬阈值（REQ-25：≤150ms）。
const MAX_OFFSET_MS: f64 = 150.0;
/// 不累积判据：末 20% − 首 20%（均值差）与线性斜率上限。
const MAX_HEAD_TAIL_DRIFT_MS: f64 = 100.0;
const MAX_SLOPE_MS_PER_S: f64 = 0.20;
/// 短音覆盖率下限（610s 应约 610 个；5% 容差）与内容时钟样本下限（捕获帧率无关：
/// 只要求全程有足够的新鲜帧；本机实测捕获约 4 帧/s → ~2400 样本）。
const MIN_BEEPS: usize = 580;
const MIN_SAMPLES: usize = 1000;

/// 生成参考影音：2s 周期横向秒表条（画面内时间编码）+ 每 1s 一次 80ms 880Hz 短音。
///
/// 秒表条用 `overlay` 的逐帧 `x` 表达式实现（`drawbox` 的 `x` 在锁定构建中只求值一次，
/// 实测条固定在画面中央；overlay 的 `t` 逐帧有效）。
fn generate_sync_clip(dir: &Path) -> PathBuf {
    let path = dir.join("sync_ref.mkv");
    let secs = clip_secs();
    let video = format!("color=c=0x101010:s=640x360:r=30:d={secs}");
    let bar = format!("color=c=white:s=20x360:r=30:d={secs}");
    let audio = format!("aevalsrc='if(lt(mod(t,1),0.08),0.35*sin(2*PI*880*t),0)':s=48000:d={secs}");
    let filter = "[0:v][1:v]overlay=x='(mod(t,2)/2)*620':y=0[v]";
    let t0 = Instant::now();
    let out = Command::new(ffbin("ffmpeg.exe"))
        .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg(video)
        .args(["-f", "lavfi", "-i"])
        .arg(bar)
        .args(["-f", "lavfi", "-i"])
        .arg(audio)
        .args(["-filter_complex", filter, "-map", "[v]", "-map", "2:a"])
        .args([
            "-c:v",
            "mpeg4",
            "-q:v",
            "6",
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-shortest",
        ])
        .arg(&path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn 锁定 ffmpeg 生成同步参考片：{e}")));
    assert!(
        out.status.success(),
        "生成同步参考片失败（exit={}；stderr={}）",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    println!(
        "[sync] 参考片生成完成：{}（{:.1}s 生成耗时，{secs}s 内容）",
        path.display(),
        t0.elapsed().as_secs_f64()
    );
    path
}

/// 参考片的源内自检：用同一内容时钟法解码参考片本身，偏差应≈0（验证相位约定/解码器）。
fn assert_clip_selfaligned(clip: &Path) {
    let samples = clock_offsets(clip, clip_secs() as f64);
    assert!(
        samples.len() >= 500,
        "参考片内容时钟样本应 ≥500（得到 {}）",
        samples.len()
    );
    let max_abs = samples
        .iter()
        .map(|p| p.offset_ms.abs())
        .fold(0.0f64, f64::max);
    let mean = samples.iter().map(|p| p.offset_ms).sum::<f64>() / samples.len() as f64;
    println!(
        "[sync] 参考片源内自检：样本 {}；mean={mean:.1}ms max|Δ|={max_abs:.1}ms（源即真值）",
        samples.len()
    );
    assert!(
        max_abs <= MAX_OFFSET_MS,
        "参考片自身音画内容时钟应对应同刻（max|Δ|={max_abs:.1}ms > {MAX_OFFSET_MS}ms；参考片生成/解码器不正确）"
    );
}

/// 同步事件（t=视频 onset 秒；offset=音−视频，毫秒）。
struct SyncPoint {
    t: f64,
    offset_ms: f64,
}

/// (mean, std, max_abs, head_mean, tail_mean, slope_ms_per_s, first_t)
fn sync_stats(points: &[SyncPoint]) -> (f64, f64, f64, f64, f64, f64, f64) {
    let n = points.len().max(1);
    let offsets: Vec<f64> = points.iter().map(|p| p.offset_ms).collect();
    let mean = offsets.iter().sum::<f64>() / n as f64;
    let var = offsets.iter().map(|o| (o - mean).powi(2)).sum::<f64>() / n as f64;
    let max_abs = offsets.iter().cloned().fold(0.0f64, |a, b| a.max(b.abs()));
    let head_n = (points.len() / 5).max(1);
    let head = points[..head_n].iter().map(|p| p.offset_ms).sum::<f64>() / head_n as f64;
    let tail_n = (points.len() / 5).max(1);
    let tail = points[points.len() - tail_n..]
        .iter()
        .map(|p| p.offset_ms)
        .sum::<f64>()
        / tail_n as f64;
    // 最小二乘斜率（ms/s）
    let (mut sx, mut sy, mut sxx, mut sxy) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for p in points {
        sx += p.t;
        sy += p.offset_ms;
        sxx += p.t * p.t;
        sxy += p.t * p.offset_ms;
    }
    let denom = n as f64 * sxx - sx * sx;
    let slope = if denom.abs() > 1e-9 {
        (n as f64 * sxy - sx * sy) / denom
    } else {
        0.0
    };
    (mean, var.sqrt(), max_abs, head, tail, slope, points[0].t)
}

/// 内容时钟偏差样本：对成片逐帧（仅新鲜捕获帧）求 `φ_v`，与音轨短音的 `φ_a` 比较。
///
/// - `φ_v = (bar_pos × BAR_PERIOD) mod 1s`（2s 秒表条 → 源时间 mod 1s）；
/// - `φ_a = (t − 前一短音 onset) mod 1s`（短音每 1s 一次）；
/// - `offset = wrap1s(φ_a − φ_v)`（±0.5s 主值；150ms 门槛下无歧义）。
fn clock_offsets(path: &Path, span: f64) -> Vec<SyncPoint> {
    let frames = decode_frames_gray(path, CLOCK_W, CLOCK_H);
    let beeps = detect_beep_onsets(&decode_audio_mono_at(path, 8_000), 8_000);
    assert!(
        !frames.is_empty(),
        "内容时钟：解码帧不得为空（{}）",
        path.display()
    );
    assert!(
        !beeps.is_empty(),
        "内容时钟：短音 onset 不得为空（{}）",
        path.display()
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
            continue; // CFR 重复帧：内容时间已过期，重复计入会放大偏差
        }
        let t = i as f64 / fps;
        if t < 2.0 || t > span - 2.0 {
            continue; // 起停边界不参与（起播/收尾瞬态）
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
            continue; // 短音缺失（音轨断流超过 1s）→ 该帧不参与
        }
        let phi_a = (t - b) % 1.0;
        let mut d = phi_a - phi_v;
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

/// 就近取点（用于打印开始/中段/结束样本）。
fn point_nearest<'a>(points: &'a [SyncPoint], t: f64) -> Option<&'a SyncPoint> {
    points
        .iter()
        .min_by(|a, b| (a.t - t).abs().partial_cmp(&(b.t - t).abs()).unwrap())
}

fn assert_sync_output_shape(path: &Path, span: f64) {
    assert_eq!(probe_stream_count(path, "v"), 1, "成片应恰 1 条视频轨");
    assert_eq!(
        probe_stream_count(path, "a"),
        1,
        "隔离模式带应用音源应恰 1 条音轨"
    );
    let (codec, w, h, fps) = probe_video_stream(path);
    assert_eq!(codec, "h264", "T6：v1 仅 H.264");
    assert_eq!(
        (w, h),
        SYNC_CANVAS,
        "D6：画布应恒为 {}x{}",
        SYNC_CANVAS.0,
        SYNC_CANVAS.1
    );
    assert!(
        (fps - SYNC_FPS as f64).abs() <= 0.5,
        "帧率应为 {SYNC_FPS}（得到 {fps}）"
    );
    let (acodec, ar, ch) = probe_audio_stream(path);
    assert_eq!(acodec, "aac", "§3.3 基准：音频编码 aac");
    assert_eq!(ar, 48_000, "REQ-18：音轨 48kHz");
    assert_eq!(ch, 2, "REQ-18：音轨立体声");
    let dur = probe_duration(path);
    assert!(
        (dur - span).abs() <= 3.0,
        "成片时长应≈录制段（{dur:.3}s vs {span:.3}s，容差 3s）"
    );
    assert!(decode_clean(path), "成片应整文件可解码（无错误输出）");
}

#[test]
#[ignore = "L2/L3：需交互式会话/WGC/可用编码器/默认渲染端点/锁定 ffmpeg；录制 610s（specs §4 it_sync_content_10min）。显式运行：cargo test -p sr-itests --test sync it_sync_content_10min -- --ignored --nocapture"]
fn it_sync_content_10min() {
    let _g = gate();
    require_interactive_session();
    let _os = record_os_build();

    let tmp = fresh_dir("sync");
    let clip = generate_sync_clip(&tmp);
    assert_clip_selfaligned(&clip);

    let pid = std::process::id();
    let title = format!("SR-M10B-SYNC-{pid}");
    let clock = content_clock();
    let tone = ToneSource::start(clock.clone());
    let win = ClockWindow::new(&title, 220, 220);
    std::thread::sleep(Duration::from_millis(1200)); // 渲染稳定（内容时钟已推进）
    println!(
        "[sync] 测试内参考源：pid={pid} hwnd={:#x}（WASAPI 渲染 + 内容时钟窗口）",
        win.hwnd_u64
    );
    let created = process_created_unix_ms(pid)
        .unwrap_or_else(|| blocked("无法读取测试进程创建时间（身份地面真值）"));

    let out_dir = fresh_dir("sync-out");
    let handle = start_audio_route_ok(AudioRouteConfig {
        target: RouteTarget::Window { hwnd: win.hwnd_u64 },
        video: VideoCfg {
            canvas_w: SYNC_CANVAS.0,
            canvas_h: SYNC_CANVAS.1,
            fps: SYNC_FPS,
            cursor: false,
        },
        audio: vec![AudioSourceSpec::App {
            pid,
            created_at_unix_ms: created,
        }],
        output_dir: out_dir.clone(),
    });
    let t0 = Instant::now();
    let record_secs = record_secs();
    let mut next_report = 30.0f64;
    println!(
        "[sync] 录制已启动：{record_secs}s（spec ≥10 分钟）；参考片 {}s；输出 {}",
        clip_secs(),
        out_dir.display()
    );
    loop {
        let elapsed = t0.elapsed().as_secs_f64();
        if elapsed >= next_report {
            println!("[sync] elapsed={elapsed:.1}s / 目标 {record_secs}s");
            next_report += 30.0;
        }
        if elapsed >= record_secs as f64 {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let span = t0.elapsed().as_secs_f64();
    let tf = Instant::now();
    handle.request_stop();
    let fin: Finalized = handle
        .wait_finalized(Duration::from_secs(180))
        .unwrap_or_else(|e| panic!("同步用例应可收尾（录制 {span:.3}s；得到 {e:?}）"));
    let finalize_s = tf.elapsed().as_secs_f64();
    println!(
        "[sync] 停止：span={span:.3}s；收尾 {finalize_s:.2}s；成品 {}",
        fin.final_path.display()
    );
    drop(tone);

    assert_sync_output_shape(&fin.final_path, span);

    // ── 内容级分析（内容时钟法） ──
    let analysis_t0 = Instant::now();
    let points = clock_offsets(&fin.final_path, span);
    let beep_count = detect_beep_onsets(&decode_audio_mono_at(&fin.final_path, 8_000), 8_000).len();
    println!(
        "[sync] 分析完成（{:.1}s）：内容时钟样本 {}；短音 {}（源 1Hz）",
        analysis_t0.elapsed().as_secs_f64(),
        points.len(),
        beep_count
    );
    let min_beeps = scaled_threshold(MIN_BEEPS);
    let min_samples = scaled_threshold(MIN_SAMPLES);
    assert!(
        beep_count >= min_beeps,
        "音频短音数应 ≥{min_beeps}（得到 {beep_count}；音轨断流/静音？）"
    );
    assert!(
        points.len() >= min_samples,
        "内容时钟样本应 ≥{min_samples}（得到 {}；采集停滞/内容不更新？）",
        points.len()
    );

    let (mean, std, max_abs, head, tail, slope, first_t) = sync_stats(&points);
    println!(
        "[sync] 偏差统计：n={} mean={mean:.1}ms std={std:.1}ms max|Δ|={max_abs:.1}ms；\
         首20%={head:.1}ms 末20%={tail:.1}ms 差值={:.1}ms；斜率={slope:.4}ms/s；首个事件 t={first_t:.2}s",
        points.len(),
        tail - head
    );

    // 逐窗口（60s）摘要：定位开始/中段/结束趋势
    let mut w_start = 0.0f64;
    while w_start < span {
        let w_end = (w_start + 60.0).min(span);
        let win: Vec<&SyncPoint> = points
            .iter()
            .filter(|p| p.t >= w_start && p.t < w_end)
            .collect();
        if !win.is_empty() {
            let m = win.iter().map(|p| p.offset_ms).sum::<f64>() / win.len() as f64;
            let mx = win.iter().map(|p| p.offset_ms.abs()).fold(0.0f64, f64::max);
            println!(
                "[sync] 窗口 {:>5.0}-{:>5.0}s：n={:<3} mean={m:+.1}ms max|Δ|={mx:.1}ms",
                w_start,
                w_end,
                win.len()
            );
        }
        w_start = w_end;
    }

    for (label, t) in [("开始", 2.0), ("中段", span / 2.0), ("结束", span - 2.0)] {
        if let Some(p) = point_nearest(&points, t) {
            println!(
                "[sync] {label}样本：t={:.2}s offset={:+.1}ms",
                p.t, p.offset_ms
            );
        }
    }

    // concat 边界（5s 段边界 ±0.5s）
    let boundary: Vec<&SyncPoint> = points
        .iter()
        .filter(|p| {
            let m = p.t % 5.0;
            m < 0.5 || m > 4.5
        })
        .collect();
    let boundary_max = boundary
        .iter()
        .map(|p| p.offset_ms.abs())
        .fold(0.0f64, f64::max);
    println!(
        "[sync] concat 边界样本：n={}（5s 边界 ±0.5s）max|Δ|={boundary_max:.1}ms",
        boundary.len()
    );
    let min_boundary = scaled_threshold(100);
    assert!(
        boundary.len() >= min_boundary,
        "concat 边界样本应覆盖 ≥{min_boundary} 个（得到 {}；边界事件缺失？）",
        boundary.len()
    );
    assert!(
        boundary_max <= MAX_OFFSET_MS,
        "REQ-25：concat 边界处内容偏差应 ≤{MAX_OFFSET_MS}ms（得到 {boundary_max:.1}ms）"
    );

    // 硬阈值断言（不削弱）
    assert!(
        max_abs <= MAX_OFFSET_MS,
        "REQ-25：任意应同步轨对最大内容偏差应 ≤{MAX_OFFSET_MS}ms（得到 {max_abs:.1}ms）"
    );
    assert!(
        (tail - head).abs() <= MAX_HEAD_TAIL_DRIFT_MS,
        "REQ-25：偏差不得累积——末 20% 与首 20% 均值差应 ≤{MAX_HEAD_TAIL_DRIFT_MS}ms（得到 {:.1}ms）",
        tail - head
    );
    assert!(
        slope.abs() <= MAX_SLOPE_MS_PER_S,
        "REQ-25：偏差斜率应 ≤{MAX_SLOPE_MS_PER_S}ms/s（得到 {slope:.4}ms/s = {:.1}ms/分钟）",
        slope * 60.0
    );

    println!(
        "[sync] 结论：内容级同步通过（max|Δ|={max_abs:.1}ms ≤ {MAX_OFFSET_MS}ms；\
         首末差 {:.1}ms；斜率 {:.4}ms/s）",
        tail - head,
        slope
    );
    println!("[sync] 输出保留供复核：{}", out_dir.display());
}

// ─────────────────────────── 用例 2：固定画布 resize（REQ-13） ───────────────────────────

const RESIZE_CANVAS: (u32, u32) = (1280, 720);
const RESIZE_FPS: u32 = 30;
/// 采样帧非黑阈值（luma 标准差；与 M3/M5 同口径）。
const FRAME_STD_MIN: f64 = 2.0;
const SAMPLE_W: u32 = 64;
const SAMPLE_H: u32 = 36;

/// 解码为低分辨率 rgb24 帧序列（内容非黑判定）。
fn decode_frames_rgb(path: &Path) -> Vec<Vec<u8>> {
    let out = Command::new(ffbin("ffmpeg.exe"))
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args([
            "-vf",
            &format!("fps=3,scale={SAMPLE_W}:{SAMPLE_H}"),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn 锁定 ffmpeg 采样解码：{e}")));
    assert!(out.status.success(), "采样解码失败（exit={}）", out.status);
    let fl = (SAMPLE_W * SAMPLE_H * 3) as usize;
    assert_eq!(
        out.stdout.len() % fl,
        0,
        "采样字节流应为整帧倍数（{} 字节）",
        out.stdout.len()
    );
    out.stdout.chunks(fl).map(<[u8]>::to_vec).collect()
}

fn luma_std(frame: &[u8]) -> f64 {
    let n = frame.len() / 3;
    let lum: Vec<f64> = (0..n)
        .map(|i| {
            0.299 * frame[i * 3] as f64
                + 0.587 * frame[i * 3 + 1] as f64
                + 0.114 * frame[i * 3 + 2] as f64
        })
        .collect();
    let mean = lum.iter().sum::<f64>() / n as f64;
    (lum.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n as f64).sqrt()
}

/// 会话临时目录内的段（`out*.mkv`，按名排序；0 字节壳由调用方过滤）。
fn session_segments(out_dir: &Path) -> Vec<PathBuf> {
    let tmp = out_dir.join(".screenrec_tmp");
    let mut segs = Vec::new();
    let Ok(sessions) = std::fs::read_dir(&tmp) else {
        return segs;
    };
    for s in sessions.flatten() {
        let Ok(files) = std::fs::read_dir(s.path()) else {
            continue;
        };
        for f in files.flatten() {
            let p = f.path();
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if name.starts_with("out") && name.ends_with(".mkv") {
                segs.push(p);
            }
        }
    }
    segs.sort();
    segs
}

#[test]
#[ignore = "L2/L3：需交互式会话/WGC/可用编码器/锁定 ffmpeg（specs §4 it_window_resize_canvas_stable）。显式运行：cargo test -p sr-itests --test sync it_window_resize_canvas_stable -- --ignored --nocapture"]
fn it_window_resize_canvas_stable() {
    let _g = gate();
    require_interactive_session();
    let _os = record_os_build();

    let pid = std::process::id();
    let title = format!("SR-M10B-RESIZE-{pid}");
    let win = AnimWindow::new(&title, 140, 140, 640, 480);
    std::thread::sleep(Duration::from_millis(600)); // 图案起转

    let out_dir = fresh_dir("resize-out");
    // M14：本用例需逐段校验参数/可解码，显式启用 keep_segments。
    let handle = start_route_with_keep_ok(RouteConfig {
        target: RouteTarget::Window { hwnd: win.hwnd_u64 },
        video: VideoCfg {
            canvas_w: RESIZE_CANVAS.0,
            canvas_h: RESIZE_CANVAS.1,
            fps: RESIZE_FPS,
            cursor: true,
        },
        output_dir: out_dir.clone(),
    });
    let t0 = Instant::now();
    println!(
        "[resize] 录制已启动：640x480 源 → 画布 {}x{}@{}",
        RESIZE_CANVAS.0, RESIZE_CANVAS.1, RESIZE_FPS
    );

    std::thread::sleep(Duration::from_secs(2));
    win.resize(800, 600);
    println!(
        "[resize] t={:.2}s：resize → 800x600（4:3，尺寸变化）",
        t0.elapsed().as_secs_f64()
    );
    std::thread::sleep(Duration::from_secs(2));
    win.resize(960, 360);
    println!(
        "[resize] t={:.2}s：resize → 960x360（8:3，比例变化）",
        t0.elapsed().as_secs_f64()
    );
    std::thread::sleep(Duration::from_secs(2));
    win.minimize();
    let t_min = t0.elapsed().as_secs_f64();
    println!("[resize] t={t_min:.2}s：最小化（REQ-13：补帧不结束录制）");
    std::thread::sleep(Duration::from_secs(2));
    win.restore();
    let t_restore = t0.elapsed().as_secs_f64();
    println!("[resize] t={t_restore:.2}s：恢复显示");
    std::thread::sleep(Duration::from_secs(2));

    let span = t0.elapsed().as_secs_f64();
    let tf = Instant::now();
    handle.request_stop();
    let fin = handle
        .wait_finalized(Duration::from_secs(120))
        .unwrap_or_else(|e| {
            panic!("REQ-13：resize/最小化后该路应仍可收尾（录制 {span:.3}s；得到 {e:?}）")
        });
    let finalize_s = tf.elapsed().as_secs_f64();
    println!(
        "[resize] span={span:.3}s；收尾 {finalize_s:.2}s；成品 {}",
        fin.final_path.display()
    );

    // ── 成片：固定画布 + 帧率恒定 + 时长覆盖整个（含最小化）录制段 ──
    assert_eq!(
        probe_stream_count(&fin.final_path, "v"),
        1,
        "REQ-13：成片应恰 1 条视频轨"
    );
    assert_eq!(
        probe_stream_count(&fin.final_path, "a"),
        0,
        "REQ-13：纯视频路不应有音轨"
    );
    let (codec, w, h, fps) = probe_video_stream(&fin.final_path);
    assert_eq!(codec, "h264", "T6：v1 仅 H.264");
    assert_eq!(
        (w, h),
        RESIZE_CANVAS,
        "REQ-13/D6：输出画布应恒为配置值 {}x{}（得到 {w}x{h}）",
        RESIZE_CANVAS.0,
        RESIZE_CANVAS.1
    );
    assert!(
        (fps - RESIZE_FPS as f64).abs() <= 0.5,
        "帧率应为 {RESIZE_FPS}（得到 {fps}）"
    );
    let dur = probe_duration(&fin.final_path);
    println!("[resize] 成片时长 {dur:.3}s（录制段 {span:.3}s）");
    assert!(
        (dur - span).abs() <= 2.0,
        "REQ-13：成片时长应≈录制段（含最小化 2s；{dur:.3}s vs {span:.3}s，容差 2s）"
    );
    assert!(decode_clean(&fin.final_path), "REQ-13：成片应整文件可解码");

    // ── 段间参数兼容 + 每段可独立解码（固定画布，段参数不得变化） ──
    let segs: Vec<PathBuf> = session_segments(&out_dir)
        .into_iter()
        .filter(|p| std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false))
        .collect();
    println!(
        "[resize] 会话段 {} 个：{:?}",
        segs.len(),
        segs.iter()
            .map(|p| p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string())
            .collect::<Vec<_>>()
    );
    assert!(
        segs.len() >= 2,
        "REQ-13：10s 录制应至少产出 2 个 5s 段（得到 {}）",
        segs.len()
    );
    for seg in &segs {
        let (scodec, sw, sh, sfps) = probe_video_stream(seg);
        assert_eq!(
            (scodec.as_str(), sw, sh),
            ("h264", RESIZE_CANVAS.0, RESIZE_CANVAS.1),
            "REQ-13：段参数应与固定画布一致（{} 得到 {scodec}/{sw}x{sh}）",
            seg.display()
        );
        assert!(
            (sfps - RESIZE_FPS as f64).abs() <= 0.5,
            "REQ-13：段帧率应与配置一致（{} 得到 {sfps}）",
            seg.display()
        );
        assert!(
            decode_clean(seg),
            "REQ-13：段应可独立解码（闭合 GOP；{}）",
            seg.display()
        );
    }

    // ── 内容：全程非黑（最小化期间按规则补帧，不得出现黑场/停录） ──
    let frames = decode_frames_rgb(&fin.final_path);
    let min_frames = (span * 3.0 * 0.9) as usize;
    assert!(
        frames.len() >= min_frames,
        "REQ-13：3fps 采样帧数应≈{min_frames}（得到 {}；录制中断？）",
        frames.len()
    );
    let mut blank = Vec::new();
    for (i, f) in frames.iter().enumerate() {
        let s = luma_std(f);
        if s < FRAME_STD_MIN {
            blank.push((i, s));
        }
    }
    assert!(
        blank.is_empty(),
        "REQ-13：全程采样帧应非黑（含最小化区间；黑帧 {blank:?} ⇒ 补帧规则失效/录制中断）"
    );
    let min_frame = (t_min * 3.0) as usize;
    let restore_frame = (t_restore * 3.0) as usize;
    println!(
        "[resize] 内容检查：{} 帧全部非黑（最小化区间帧 {min_frame}..{restore_frame}）",
        frames.len()
    );

    println!(
        "[resize] 结论：resize/比例变化/最小化恢复后画布恒定 {}x{}，{} 段参数一致且独立可播，overall OK",
        RESIZE_CANVAS.0, RESIZE_CANVAS.1, segs.len()
    );
    drop(win);
    println!("[resize] 输出保留供复核：{}", out_dir.display());
}
