//! L3 半自动 soak：2 路 1080p30 连续录制 ≥10 分钟（REQ-27 / §9 T1 保底并发，specs
//! 001-screen-recorder.md §4 `manual_soak_two_routes_10min`、§5 M10 行）。
//!
//! 运行方式（L3 门：`#[ignore]`，默认 `cargo test` 不执行）：
//! ```text
//! cargo test -p sr-itests --test soak -- --ignored --nocapture
//! ```
//! 前置条件：交互式用户会话 + WGC + 可用编码器（D4 链）+ 锁定 ffmpeg n8.1（§8）；
//! 缺失 → BLOCKED（fail），不静默 skip（§4）。
//!
//! 判定面（本文件自带，不修改既有测试）：
//! - 2 路窗口目标、固定画布 1920x1080@30、纯视频（REQ-27 不要求音轨）；
//! - 连续录制 ≥600s（本用例录 620s，留 20s 余量）；
//! - 停止 → 两路各自收尾成功（成品存在、MKV、恰 1 条 h264 轨、画布/帧率恒定）；
//! - 成品时长 ≈ 录制段（±3s）且整文件可解码；
//! - 内容级「贯穿录制段」：在起始/中段/收尾各取一帧，均非黑且至少一对存在明显差异
//!   （证明输出不是冻结的同一帧）；
//! - 收尾后本进程无 ffmpeg sidecar 残留（无崩溃/无孤儿进程）。
//!
//! 资源观测（RSS/CPU）由**外部 PowerShell 采样器**在测试运行期间每 30s 记录一次
//! （见 docs/milestones/m10-soak-samples.csv 与 m10-rehearsal-report.md §3）。

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::Once;
use std::time::{Duration, Instant};

use common::*;
use sr_pipeline::route::{
    start_routes, Finalized, MultiRouteConfig, RouteError, RouteHandle, RouteTarget, VideoCfg,
};

/// 录制时长（REQ-27：≥10 分钟；620s = 600s 保底 + 20s 余量）。
const SOAK_SECS: u64 = 620;
/// 成品时长与录制段的容差（10 分钟量级；段切换/收尾抖动远小于此）。
const DUR_TOL_SECS: f64 = 3.0;
/// 画布（D6 固定画布）与帧率（§9 T1 保底 2 路 1080p30）。
const CANVAS: (u32, u32) = (1920, 1080);
const FPS: u32 = 30;
/// 内容级非黑阈值（与 M3/M5 同一 luma 标准差口径）。
const FRAME_STD_MIN: f64 = 2.0;
/// 起始/中段/收尾帧对的最小差异（证明内容随时间变化，而非冻结首帧）。
const FRAME_DIFF_MIN: f64 = 2.0;
/// 采样缩略帧尺寸。
const SAMPLE_W: u32 = 64;
const SAMPLE_H: u32 = 36;

// ─────────────────────────── 锁定构建 / 输出目录 ───────────────────────────

/// 锁定 ffmpeg/ffprobe：spec §8 M0 冻结的 BtbN n8.1 release 构建。
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

fn fresh_out_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr_m10_soak_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("创建输出目录失败");
    d
}

/// start_routes 的 BLOCKED 包装（环境缺失 → BLOCKED；其它 Err → panic）。
fn start_routes_ok(cfgs: Vec<MultiRouteConfig>) -> Vec<RouteHandle> {
    let n = cfgs.len();
    match start_routes(cfgs) {
        Ok(hs) => {
            assert_eq!(
                hs.len(),
                n,
                "start_routes Ok 应返回与 cfgs 同序同长的句柄集"
            );
            hs
        }
        Err(e) if matches!(e, RouteError::EncoderUnavailable { .. }) => blocked(&format!(
            "无可用编码器（§9 T1：明确报错并列出可用编码器）：{e:?}"
        )),
        Err(e) if matches!(e, RouteError::ConsentDenied { .. }) => {
            blocked(&format!("WGC 授权被拒（E_CONSENT_DENIED）：{e:?}"))
        }
        Err(e) => panic!("start_routes 应成功（得到 {e:?}）"),
    }
}

// ─────────────────────────── ffprobe / 解码 / 内容采样 ───────────────────────────

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

fn probe_packet_count(path: &Path) -> u64 {
    let s = ffprobe_csv(
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
    );
    s.parse::<u64>().unwrap_or(0)
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

/// 在 `at_secs` 处取一帧缩略 RGB（快速定位 seek）。
fn sample_frame_rgb(path: &Path, at_secs: f64) -> Vec<u8> {
    let out = Command::new(ffbin("ffmpeg.exe"))
        .args([
            "-v",
            "error",
            "-nostdin",
            "-ss",
            &format!("{at_secs:.3}"),
            "-i",
        ])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            &format!("scale={SAMPLE_W}:{SAMPLE_H}"),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn 锁定 ffmpeg 取帧：{e}")));
    assert!(out.status.success(), "取帧失败（exit={}）", out.status);
    assert_eq!(
        out.stdout.len(),
        (SAMPLE_W * SAMPLE_H * 3) as usize,
        "单帧字节数应为 {SAMPLE_W}x{SAMPLE_H} RGB24"
    );
    out.stdout
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

fn mean_abs_diff(a: &[u8], b: &[u8]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (*x as i32 - *y as i32).abs() as f64)
        .sum::<f64>()
        / a.len() as f64
}

// ─────────────────────────── sidecar 残留检查 ───────────────────────────

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

/// 本测试进程的 ffmpeg 子进程（Toolhelp32 父子关系 + 映像名）。
fn ffmpeg_child_pids() -> Vec<u32> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    unsafe {
        let snap =
            CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).expect("CreateToolhelp32Snapshot");
        let mut out = Vec::new();
        let mut pe = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut pe).is_ok() {
            loop {
                let name = wide_to_string(&pe.szExeFile).to_lowercase();
                if pe.th32ParentProcessID == std::process::id() && name.contains("ffmpeg") {
                    out.push(pe.th32ProcessID);
                }
                if Process32NextW(snap, &mut pe).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
        out
    }
}

// ─────────────────────────── 持续动画的采集窗口（自带泵线程） ───────────────────────────

const SOAK_CLASS_NAME: &str = "SR_M10_SOAK_WND";
static SOAK_CLASS: Once = Once::new();
static PAINT_TICKS: AtomicU32 = AtomicU32::new(0);

fn register_soak_class() {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{RegisterClassW, WNDCLASSW};
    SOAK_CLASS.call_once(|| unsafe {
        let hinst = GetModuleHandleW(None).ok().expect("GetModuleHandleW");
        let class_name = windows::core::HSTRING::from(SOAK_CLASS_NAME);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(soak_wndproc),
            hInstance: hinst.into(),
            lpszClassName: windows::core::PCWSTR::from_raw(class_name.as_ptr()),
            ..Default::default()
        };
        let atom = RegisterClassW(&wc);
        assert_ne!(atom, 0, "RegisterClassW({SOAK_CLASS_NAME}) 失败");
    });
}

unsafe extern "system" fn soak_wndproc(
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
                let n = PAINT_TICKS.fetch_add(1, Ordering::Relaxed) as i32;

                // 深灰底
                let bg = CreateSolidBrush(COLORREF(40 | (44 << 8) | (52 << 16)));
                FillRect(hdc, &rc, bg);
                let _ = DeleteObject(HGDIOBJ(bg.0));

                // 全高移动亮条（横向 + 纵向两块，保证画面持续变化）
                let bar_w = (w / 8).max(32);
                let bar_x = (n * 23).rem_euclid(w);
                let bar = CreateSolidBrush(COLORREF(240 | (210 << 8) | (70 << 16)));
                let bar_rc = RECT {
                    left: bar_x,
                    top: 0,
                    right: (bar_x + bar_w).min(w),
                    bottom: h,
                };
                FillRect(hdc, &bar_rc, bar);
                let _ = DeleteObject(HGDIOBJ(bar.0));

                let blk_h = (h / 6).max(16);
                let blk_y = (n * 11).rem_euclid(h);
                let blk = CreateSolidBrush(COLORREF(70 | (140 << 8) | (240 << 16)));
                let blk_rc = RECT {
                    left: w / 3,
                    top: blk_y,
                    right: (2 * w / 3).min(w),
                    bottom: (blk_y + blk_h).min(h),
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

/// 自建动画窗口（专用泵线程 + 33ms 定时重绘），供 2 路 WGC 窗口捕获。
struct SoakWindow {
    hwnd_u64: u64,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SoakWindow {
    fn new(title: &str, x: i32, y: i32, w: i32, h: i32) -> SoakWindow {
        let (tx, rx) = mpsc::channel();
        let title = title.to_string();
        let thread = std::thread::Builder::new()
            .name(format!("sr-m10-soak-wnd-{title}"))
            .spawn(move || {
                use windows::Win32::Graphics::Gdi::UpdateWindow;
                use windows::Win32::System::LibraryLoader::GetModuleHandleW;
                use windows::Win32::UI::WindowsAndMessaging::{
                    CreateWindowExW, DispatchMessageW, GetMessageW, SetTimer, SetWindowPos,
                    TranslateMessage, HWND_TOPMOST, MSG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
                    WS_EX_TOPMOST, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
                };
                register_soak_class();
                unsafe {
                    let hinst = GetModuleHandleW(None).ok().expect("GetModuleHandleW");
                    let class_name = windows::core::HSTRING::from(SOAK_CLASS_NAME);
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
                    .expect("CreateWindowExW 失败：无法构造 soak 窗口");
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
                    let _ = UpdateWindow(hwnd);
                    tx.send(hwnd.0 as usize as u64).expect("回传 hwnd");
                    let mut msg = MSG::default();
                    loop {
                        let r = GetMessageW(&mut msg, None, 0, 0);
                        if r.0 <= 0 {
                            break; // 0 = WM_QUIT；-1 = 错误
                        }
                        let _ = TranslateMessage(&msg);
                        let _ = DispatchMessageW(&msg);
                    }
                }
            })
            .expect("spawn soak 窗口线程");
        let hwnd_u64 = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("soak 窗口创建超时（泵线程未回传 hwnd）");
        SoakWindow {
            hwnd_u64,
            thread: Some(thread),
        }
    }
}

impl Drop for SoakWindow {
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

// ─────────────────────────── 用例 ───────────────────────────

/// REQ-27 / §9 T1：2 路 1080p30 连续录制 ≥10 分钟，无崩溃、两路可收尾可播。
#[test]
#[ignore = "L3：需交互式会话/WGC/可用编码器/锁定 ffmpeg；录制 620s（specs §4 manual_soak_two_routes_10min）。显式运行：cargo test -p sr-itests --test soak -- --ignored --nocapture"]
fn manual_soak_two_routes_10min() {
    require_interactive_session();
    let _os = record_os_build();

    let pid = std::process::id();
    let title_a = format!("SR-M10-SOAK-A-{pid}");
    let title_b = format!("SR-M10-SOAK-B-{pid}");
    let win_a = SoakWindow::new(&title_a, 60, 60, 900, 520);
    let win_b = SoakWindow::new(&title_b, 1000, 60, 900, 520);
    std::thread::sleep(Duration::from_millis(600)); // 图案起转

    let out_a = fresh_out_dir("a");
    let out_b = fresh_out_dir("b");
    println!(
        "[soak] 目标 A={title_a} hwnd={} out={}",
        win_a.hwnd_u64,
        out_a.display()
    );
    println!(
        "[soak] 目标 B={title_b} hwnd={} out={}",
        win_b.hwnd_u64,
        out_b.display()
    );

    let handles = start_routes_ok(vec![
        MultiRouteConfig {
            target: RouteTarget::Window {
                hwnd: win_a.hwnd_u64,
            },
            video: VideoCfg {
                canvas_w: CANVAS.0,
                canvas_h: CANVAS.1,
                fps: FPS,
                cursor: true,
            },
            audio: vec![],
            output_dir: out_a.clone(),
        },
        MultiRouteConfig {
            target: RouteTarget::Window {
                hwnd: win_b.hwnd_u64,
            },
            video: VideoCfg {
                canvas_w: CANVAS.0,
                canvas_h: CANVAS.1,
                fps: FPS,
                cursor: true,
            },
            audio: vec![],
            output_dir: out_b.clone(),
        },
    ]);
    let t0 = Instant::now();
    let mut next_report = 30.0f64;
    println!("[soak] 两路已启动（1920x1080@30，纯视频）；目标录制 {SOAK_SECS}s（REQ-27 ≥600s）");

    // ── 录制期：每 30s 打点（外部采样器以 elapsed 对齐） ──
    loop {
        let elapsed = t0.elapsed().as_secs_f64();
        if elapsed >= next_report {
            println!("[soak] elapsed={elapsed:.1}s / 目标 {SOAK_SECS}s");
            next_report += 30.0;
        }
        if elapsed >= SOAK_SECS as f64 {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    // ── 停止并收尾（逐路记录 span / 收尾耗时） ──
    let mut spans = Vec::new();
    let mut finalize_secs = Vec::new();
    let mut finals: Vec<Finalized> = Vec::new();
    for (i, h) in handles.iter().enumerate() {
        let span = t0.elapsed().as_secs_f64();
        h.request_stop();
        let tf = Instant::now();
        let fin = h
            .wait_finalized(Duration::from_secs(180))
            .unwrap_or_else(|e| {
                panic!("soak 第 {i} 路（录制 {span:.3}s）应可收尾（得到 {e:?}）");
            });
        finalize_secs.push(tf.elapsed().as_secs_f64());
        spans.push(span);
        finals.push(fin);
    }

    // ── 断言：两路成品可收尾、可播、时长≈录制段、内容贯穿 ──
    let mut total_bytes = 0u64;
    for (i, (fin, span)) in finals.iter().zip(spans.iter()).enumerate() {
        let label = ["A", "B"][i];
        let span = *span;
        assert!(
            fin.final_path.exists(),
            "路{label}：成品应存在（{}）",
            fin.final_path.display()
        );
        assert_eq!(
            probe_stream_count(&fin.final_path, "v"),
            1,
            "路{label}：应恰 1 条视频轨"
        );
        assert_eq!(
            probe_stream_count(&fin.final_path, "a"),
            0,
            "路{label}：纯视频路不应有音轨"
        );
        let (codec, w, h, fps) = probe_video_stream(&fin.final_path);
        assert_eq!(codec, "h264", "路{label}：T6 v1 仅 H.264");
        assert_eq!(
            (w, h),
            CANVAS,
            "路{label}：D6 固定画布应恒为 {}x{}（得到 {w}x{h}）",
            CANVAS.0,
            CANVAS.1
        );
        assert!(
            (fps - FPS as f64).abs() <= 0.5,
            "路{label}：帧率应为 {FPS}（得到 {fps}）"
        );

        let dur = probe_duration(&fin.final_path);
        assert!(
            span >= 600.0,
            "REQ-27：录制段应 ≥600s（路{label} 得到 {span:.3}s）"
        );
        assert!(
            (dur - span).abs() <= DUR_TOL_SECS,
            "路{label}：成品时长应≈录制段（{dur:.3}s vs {span:.3}s，容差 {DUR_TOL_SECS}s）"
        );
        let packets = probe_packet_count(&fin.final_path);
        let expected = (dur * FPS as f64) as u64;
        assert!(
            packets + 5 >= expected,
            "路{label}：视频包数应≈时长×帧率（{packets} < {expected}；疑似大段丢帧）"
        );
        assert!(
            decode_clean(&fin.final_path),
            "路{label}：成品应整文件可解码（无错误输出）"
        );

        // 内容贯穿：起始/中段/收尾各取一帧（非黑 + 至少一对存在明显差异）
        let f_start = sample_frame_rgb(&fin.final_path, 1.0);
        let f_mid = sample_frame_rgb(&fin.final_path, dur / 2.0);
        let f_end = sample_frame_rgb(&fin.final_path, (dur - 1.5).max(0.0));
        let stds = [luma_std(&f_start), luma_std(&f_mid), luma_std(&f_end)];
        for (k, s) in stds.iter().enumerate() {
            assert!(
                *s >= FRAME_STD_MIN,
                "路{label}：采样帧 {k} 应非黑（luma std {s:.3} < {FRAME_STD_MIN}）"
            );
        }
        let diffs = [
            mean_abs_diff(&f_start, &f_mid),
            mean_abs_diff(&f_mid, &f_end),
            mean_abs_diff(&f_start, &f_end),
        ];
        assert!(
            diffs.iter().cloned().fold(0.0f64, f64::max) >= FRAME_DIFF_MIN,
            "路{label}：起/中/尾帧应体现内容变化（最大差异 {:.3} < {FRAME_DIFF_MIN}；疑似冻结首帧/采集停滞）",
            diffs.iter().cloned().fold(0.0f64, f64::max)
        );
        let size = std::fs::metadata(&fin.final_path)
            .map(|m| m.len())
            .unwrap_or(0);
        total_bytes += size;
        println!(
            "[soak] 路{label}: 录制段 {span:.3}s；成品 {dur:.3}s；包数 {packets}（期望≈{expected}）；\
             收尾 {:.2}s；{} 字节；起/中/尾 luma_std={:.2}/{:.2}/{:.2}；帧差 max={:.2}；路径 {}",
            finalize_secs[i],
            size,
            stds[0],
            stds[1],
            stds[2],
            diffs.iter().cloned().fold(0.0f64, f64::max),
            fin.final_path.display()
        );
    }
    assert_ne!(
        finals[0].final_path, finals[1].final_path,
        "两路成品路径应互异"
    );

    // ── 无崩溃/无孤儿：收尾后本进程不应残留 ffmpeg sidecar ──
    let leftovers = wait_until(Duration::from_secs(10), Duration::from_millis(200), || {
        ffmpeg_child_pids().is_empty().then_some(())
    });
    assert!(
        leftovers.is_some(),
        "两路收尾后不应残留 ffmpeg sidecar（得到 {:?}）",
        ffmpeg_child_pids()
    );

    println!(
        "[soak] 结论：2 路 × {SOAK_SECS}s（≥600s）完成；成品合计 {:.2} MiB；无崩溃、无 sidecar 残留",
        total_bytes as f64 / (1024.0 * 1024.0)
    );
    println!(
        "[soak] 输出保留供复核：{}；{}",
        out_a.display(),
        out_b.display()
    );
    drop(win_a);
    drop(win_b);
}
