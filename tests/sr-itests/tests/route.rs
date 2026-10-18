//! L2 集成测试：单路视频管线（REQ-9/10/11/20/21/24 的 M3 范围，specs/001-screen-recorder.md
//! §2.3/§2.5、§3.3 管道与 ffmpeg 基准、§5 M3、§8 M0 冻结项；docs/spikes/s1b §7 建议）。
//!
//! 运行方式（L2 门：`#[ignore]`，默认 `cargo test` 不执行本文件用例）：
//! ```text
//! cargo test -p sr-itests --test route -- --ignored --nocapture
//! cargo test -p sr-itests --test route it_single_route_frames_nonblank -- --ignored --nocapture   # 单用例
//! ```
//! 前置条件：交互式用户会话 + WGC 可用 + 可用编码器（D4 链）+ 仓库内锁定 ffmpeg n8.1
//! release 构建（spec §8；含 ffprobe）；缺失 → BLOCKED（fail），不静默 skip（§4）。
//! 本机环境（红测试编写时）：Windows 11 25H2，build 26200.9457，SESSIONNAME=Console。
//!
//! 接口契约（测试即契约，详见 docs/milestones/m3-red-report.md）：
//! - `sr_pipeline::route::start_route(RouteConfig) -> Result<RouteHandle, RouteError>`
//!   Ok 语义：启动协议走到「输出开始」（管道建立+编码器就绪+公共 T0，§3.1）。
//! - `RouteConfig { target: RouteTarget, video: VideoCfg, output_dir: PathBuf }`
//!   `RouteTarget::Window { hwnd: u64 } | Monitor { index: u32 }`（Region 按 §3.4 由实现补全）
//!   `VideoCfg { canvas_w: u32, canvas_h: u32, fps: u32, cursor: bool }`（固定画布 D6）
//! - `RouteHandle::request_stop(&self)`（「已请求停止」，立即返回，§3.3/T5）
//! - `RouteHandle::wait_finalized(&self, timeout) -> Result<Finalized, RouteError>`
//!   （等后台 concat 收尾；Err 携带失败原因；handle 在 Err 后仍可查询 segments()）
//! - `RouteHandle::segments(&self) -> Vec<PathBuf>`（当前已写出的段；**0 字节壳不列入**
//!   ——S5/S1b 冻结：0 字节段排除）
//! - `Finalized { final_path: PathBuf, segment_list: PathBuf, segments: Vec<PathBuf> }`
//!   （成品=REQ-24 命名：`{yyyyMMdd_HHmmss}_{sanitize(目标名)}.mkv`；segment_list=ffconcat）
//! - `RouteError`：公开错误枚举，derive(Debug)；测试钉 4 个变体（M3 最小集，花括号+`..`
//!   容忍字段）：`EncoderUnavailable`（§9 T1 明确报错）、`ConsentDenied`（WGC 授权）、
//!   `SidecarExit`（E_SIDECAR_EXIT：sidecar 非零退出/被终止，保留分段，§3.5）、
//!   `WaitTimeout`（收尾超时）。其余变体实现自定。
//!   （§3.4 的 E_* 完整错误枚举落 sr_core::error——已存在；RouteError 与其映射属实现内部。）
//!
//! 手法：被录目标是**自建运动图案窗口**（专用线程跑消息泵 + 33ms 定时器 + 移动亮条/秒级
//! 翻转块），内容级判定用锁定 ffmpeg 解码采样帧（亮度方差=非黑帧；帧间差=运动）；
//! 容器/时长/关键帧用 ffprobe；强杀用例以 Toolhelp32 找本测试进程的 ffmpeg sidecar 子进程
//! 并 `taskkill /T /F`（基础版：主进程自杀不可行，杀 sidecar 树即 §3.5「sidecar 被终止」
//! 路径；主进程崩溃矩阵属 REQ-22 后续用例）。

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::Once;
use std::time::{Duration, Instant};

use common::*;
use sr_pipeline::naming::sanitize_target_name;
use sr_pipeline::route::{
    start_route_with_options, RouteConfig, RouteError, RouteHandle, RouteTarget, VideoCfg,
};

// ─────────────────────────── 内容级判定阈值（构造性保证 + scratch 实测） ───────────────────────────
// 运动图案：深灰底 + 全高亮条每帧移 37px + 秒级翻转色块 ⇒ 采样帧亮度 std ≫ 2，
// 3fps 采样下相邻帧差异 ≫ 2。黑帧/静帧捕获的 std≈0、帧间差≈0，判别距离一个数量级。
const SAMPLE_FPS: u32 = 3;
const SAMPLE_W: u32 = 64;
const SAMPLE_H: u32 = 36;
const FRAME_STD_MIN: f64 = 2.0;
const MOTION_DIFF_MIN: f64 = 2.0;
const MOTION_PAIR_RATIO_MIN: f64 = 0.30;
const MOTION_MAX_DIFF_MIN: f64 = 10.0;

/// 锁定 ffmpeg/ffprobe 位置：spec §8 M0 冻结的 BtbN **n8.1 release** 构建（绝对路径）。
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

fn fresh_out_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr_m3_red_route_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("创建输出目录失败");
    d
}

/// start_route 的 BLOCKED 包装：环境缺失（无编码器/WGC 被拒）→ BLOCKED；其它 Err → panic。
fn start_route_ok(cfg: RouteConfig) -> RouteHandle {
    start_route_with_keep_ok(cfg, false)
}

/// M14：显式保留策略的启动包装（`keep_segments=true` 用于需检查原段的用例）。
fn start_route_with_keep_ok(cfg: RouteConfig, keep_segments: bool) -> RouteHandle {
    match start_route_with_options(cfg, keep_segments) {
        Ok(h) => h,
        Err(e) if matches!(e, RouteError::EncoderUnavailable { .. }) => blocked(&format!(
            "无可用编码器（§9 T1：明确报错并列出可用编码器）：{e:?}"
        )),
        Err(e) if matches!(e, RouteError::ConsentDenied { .. }) => {
            blocked(&format!("WGC 授权被拒（E_CONSENT_DENIED）：{e:?}"))
        }
        Err(e) => panic!("start_route_with_options 应成功（得到 {e:?}）"),
    }
}

/// 等待已写出段数达到 min（轮询 RouteHandle::segments）。
fn wait_segment_count(h: &RouteHandle, min: usize, timeout: Duration) -> usize {
    wait_until(timeout, Duration::from_millis(250), || {
        let n = h.segments().len();
        if n >= min {
            Some(n)
        } else {
            None
        }
    })
    .unwrap_or_else(|| {
        panic!(
            "{min} 个段未在 {timeout:?} 内写出（当前 {} 个）",
            h.segments().len()
        )
    })
}

// ─────────────────────────── ffprobe / 解码交叉验证 ───────────────────────────

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

/// 容器时长（format duration，秒）。损坏/未封口段返回 None。
fn probe_duration_opt(path: &Path) -> Option<f64> {
    let s = ffprobe_csv(
        path,
        &["-show_entries", "format=duration", "-of", "csv=p=0"],
    );
    s.parse::<f64>().ok().filter(|d| *d > 0.0)
}

fn probe_duration(path: &Path) -> f64 {
    probe_duration_opt(path)
        .unwrap_or_else(|| panic!("应能读出时长（{} 未封口/损坏？）", path.display()))
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
    // 形如 "h264,640,360,30/1"
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

/// 关键帧时间戳（秒；-skip_frame nokey）。
fn probe_keyframe_pts(path: &Path) -> Vec<f64> {
    let s = ffprobe_csv(
        path,
        &[
            "-select_streams",
            "v:0",
            "-skip_frame",
            "nokey",
            "-show_entries",
            "frame=pts_time",
            "-of",
            "csv=p=0",
        ],
    );
    s.lines()
        .filter_map(|l| l.trim().parse::<f64>().ok())
        .collect()
}

/// 整文件可解码（ffmpeg -v error -f null -）：exit 0 即通过。
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

/// 低分辨率低帧率采样解码（rgb24 帧）。
fn decode_frames_rgb(path: &Path) -> Vec<Vec<u8>> {
    let out = Command::new(ffbin("ffmpeg.exe"))
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args([
            "-vf",
            &format!("fps={SAMPLE_FPS},scale={SAMPLE_W}:{SAMPLE_H}"),
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
    let fl = (SAMPLE_W as usize) * (SAMPLE_H as usize) * 3;
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

fn mean_abs_diff(a: &[u8], b: &[u8]) -> f64 {
    let n = a.len();
    a.iter()
        .zip(b)
        .map(|(x, y)| (*x as i32 - *y as i32).abs() as f64)
        .sum::<f64>()
        / n as f64
}

/// 非黑帧 + 运动判定（REQ-9 内容级）。
fn assert_frames_nonblank_and_moving(final_path: &Path, min_frames: usize) {
    let frames = decode_frames_rgb(final_path);
    assert!(
        frames.len() >= min_frames,
        "采样帧数应 ≥{min_frames}（得到 {}；3fps 采样下时长不足？）",
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
        "REQ-9：所有采样帧应为非黑帧（亮度标准差 ≥{FRAME_STD_MIN}；黑帧：{blank:?}）"
    );
    let mut diffs = Vec::new();
    for w in frames.windows(2) {
        diffs.push(mean_abs_diff(&w[0], &w[1]));
    }
    let moving = diffs.iter().filter(|d| **d >= MOTION_DIFF_MIN).count();
    let max_d = diffs.iter().cloned().fold(0.0f64, f64::max);
    println!(
        "[frames] {} 帧；运动对 {}/{}（≥{:.0}%），最大帧差 {max_d:.2}",
        frames.len(),
        moving,
        diffs.len(),
        MOTION_PAIR_RATIO_MIN * 100.0
    );
    assert!(
        moving as f64 / diffs.len() as f64 >= MOTION_PAIR_RATIO_MIN,
        "REQ-9：相邻采样帧差异应体现运动（≥{:.0}% 的对差 ≥{MOTION_DIFF_MIN}；得到 {moving}/{})",
        MOTION_PAIR_RATIO_MIN * 100.0,
        diffs.len()
    );
    assert!(
        max_d >= MOTION_MAX_DIFF_MIN,
        "REQ-9：最大帧间差应 ≥{MOTION_MAX_DIFF_MIN}（得到 {max_d:.2}：疑似静帧/单色捕获）"
    );
}

// ─────────────────────────── 运动图案窗口（专用泵线程） ───────────────────────────

const MOTION_CLASS_NAME: &str = "SR_M3_RED_MOTION_WND";
static MOTION_CLASS: Once = Once::new();
static PAINT_COUNT: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn motion_wndproc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wp: windows::Win32::Foundation::WPARAM,
    lp: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::{LRESULT, RECT};
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, InvalidateRect, PAINTSTRUCT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, GetClientRect, KillTimer, PostQuitMessage, WM_DESTROY, WM_PAINT, WM_TIMER,
    };
    unsafe {
        match msg {
            WM_TIMER => {
                // 33ms 定时失效 → 重绘（运动由 WM_TIMER 驱动）
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
                let n = PAINT_COUNT.fetch_add(1, Ordering::Relaxed) as i32;

                let brush = |r: u32, g: u32, b: u32| {
                    windows::Win32::Foundation::COLORREF(r | (g << 8) | (b << 16))
                };
                // 1) 深灰底
                let bg = CreateSolidBrush(brush(64, 64, 64));
                FillRect(hdc, &rc, bg);
                let _ = DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(bg.0));
                // 2) 全高移动亮条（每帧 +37px，环绕）
                let bar_w = (w / 10).max(24);
                let bar_x = (n * 37) % w;
                let bar = CreateSolidBrush(brush(235, 240, 255));
                let bar_rc = RECT {
                    left: rc.left + bar_x,
                    top: rc.top,
                    right: rc.left + bar_x + bar_w,
                    bottom: rc.bottom,
                };
                FillRect(hdc, &bar_rc, bar);
                let _ = DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(bar.0));
                // 3) 秒级翻转象限色块（红↔蓝，30 帧/秒节拍）
                let block = CreateSolidBrush(if (n / 30) % 2 == 0 {
                    brush(210, 50, 60)
                } else {
                    brush(50, 90, 220)
                });
                let bq = if (n / 30) % 2 == 0 {
                    RECT {
                        left: rc.left,
                        top: rc.top,
                        right: rc.left + w / 2,
                        bottom: rc.top + h / 2,
                    }
                } else {
                    RECT {
                        left: rc.right - w / 2,
                        top: rc.bottom - h / 2,
                        right: rc.right,
                        bottom: rc.bottom,
                    }
                };
                FillRect(hdc, &bq, block);
                let _ = DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(block.0));

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

fn register_motion_class() {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::RegisterClassW;
    MOTION_CLASS.call_once(|| unsafe {
        let hinst = GetModuleHandleW(None).ok().expect("GetModuleHandleW");
        let class_name = windows::core::HSTRING::from(MOTION_CLASS_NAME);
        let wc = windows::Win32::UI::WindowsAndMessaging::WNDCLASSW {
            lpfnWndProc: Some(motion_wndproc),
            hInstance: hinst.into(),
            lpszClassName: windows::core::PCWSTR::from_raw(class_name.as_ptr()),
            ..Default::default()
        };
        let atom = RegisterClassW(&wc);
        assert_ne!(atom, 0, "RegisterClassW({MOTION_CLASS_NAME}) 失败");
    });
}

/// 自建运动图案窗口：内容在**专用线程**的消息泵驱动下持续变化（33ms 定时器），
/// 与被测 start_route（主测试线程）解耦；Drop 时 PostMessage(WM_CLOSE) + join。
pub struct MotionWindow {
    pub hwnd_u64: u64,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl MotionWindow {
    pub fn new(title: &str, w: i32, h: i32) -> MotionWindow {
        let (tx, rx) = mpsc::channel();
        let title = title.to_string();
        let thread = std::thread::Builder::new()
            .name("sr-m3-motion".to_string())
            .spawn(move || {
                use windows::Win32::Graphics::Gdi::UpdateWindow;
                use windows::Win32::UI::WindowsAndMessaging::{
                    CreateWindowExW, DispatchMessageW, GetMessageW, SetTimer, TranslateMessage,
                    CW_USEDEFAULT, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
                };
                unsafe {
                    register_motion_class();
                    let hinst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
                        .ok()
                        .expect("GetModuleHandleW");
                    let class_name = windows::core::HSTRING::from(MOTION_CLASS_NAME);
                    let title_w = windows::core::HSTRING::from(title.as_str());
                    let hwnd = CreateWindowExW(
                        WINDOW_EX_STYLE(0),
                        windows::core::PCWSTR::from_raw(class_name.as_ptr()),
                        windows::core::PCWSTR::from_raw(title_w.as_ptr()),
                        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                        CW_USEDEFAULT,
                        CW_USEDEFAULT,
                        w,
                        h,
                        None,
                        None,
                        Some(hinst.into()),
                        None,
                    )
                    .expect("CreateWindowExW 失败：无法构造运动窗口");
                    SetTimer(Some(hwnd), 1, 33, None);
                    let _ = UpdateWindow(hwnd);
                    tx.send(hwnd.0 as usize as u64).expect("回传 hwnd");
                    let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
                    loop {
                        let r = GetMessageW(&mut msg, None, 0, 0);
                        if r.0 <= 0 {
                            break; // 0 = WM_QUIT（窗口销毁后退出泵）；-1 = 错误
                        }
                        let _ = TranslateMessage(&msg);
                        let _ = DispatchMessageW(&msg);
                    }
                }
            })
            .expect("spawn 运动窗口线程");
        let hwnd_u64 = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("运动窗口创建超时（泵线程未回传 hwnd）");
        MotionWindow {
            hwnd_u64,
            thread: Some(thread),
        }
    }
}

impl Drop for MotionWindow {
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

// ─────────────────────────── sidecar 子进程发现与强杀 ───────────────────────────

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

/// 本测试进程的 ffmpeg sidecar 子进程（Toolhelp32 父子关系 + 映像名）。
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

/// 强杀进程树（taskkill /T /F；S1b 实验 2 同手段）。
fn kill_tree(pid: u32) {
    let st = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("spawn taskkill");
    assert!(st.success(), "taskkill /PID {pid} /T /F 失败：{st}");
}

// ─────────────────────────── 用例 ───────────────────────────

/// REQ-9：单路录制——真实 WGC 会话产出**非黑且运动**的画面，成品 MKV 时长≈录制段。
#[test]
#[ignore = "L2：需交互式会话/WGC/可用编码器（specs §4）。显式运行：cargo test -p sr-itests --test route -- --ignored --nocapture"]
fn it_single_route_frames_nonblank() {
    require_interactive_session();
    let _os = record_os_build();

    let title = format!("SR-M3-RED-route-{}", std::process::id());
    let win = MotionWindow::new(&title, 800, 500);
    std::thread::sleep(Duration::from_millis(500)); // 图案起转

    let out_dir = fresh_out_dir("single");
    let h = start_route_ok(RouteConfig {
        target: RouteTarget::Window { hwnd: win.hwnd_u64 },
        video: VideoCfg {
            canvas_w: 640,
            canvas_h: 360,
            fps: 30,
            cursor: true,
        },
        output_dir: out_dir.clone(),
    });
    std::thread::sleep(Duration::from_millis(300));
    let t_rec = Instant::now();
    std::thread::sleep(Duration::from_secs(7));
    h.request_stop();
    let span = t_rec.elapsed().as_secs_f64();
    let fin = h
        .wait_finalized(Duration::from_secs(120))
        .expect("REQ-9：单路停止后收尾应成功（EOF 优雅停止 + 后台 concat）");

    assert!(
        fin.final_path.exists(),
        "成品应存在（{}）",
        fin.final_path.display()
    );
    assert!(
        fin.final_path.extension().map_or(false, |e| e == "mkv"),
        "成品应为 MKV（{}）",
        fin.final_path.display()
    );
    assert!(!fin.segments.is_empty(), "应至少有 1 个段（7s > 5s 段长）");
    // M14 默认保留策略：成品验证成功后清理原段（会话目录递归删除）。
    let session_dir = fin
        .segment_list
        .parent()
        .expect("segment_list 应位于会话目录内");
    assert!(
        !session_dir.exists(),
        "M14：默认成功后会话临时目录应被清理（{}）",
        session_dir.display()
    );
    for s in &fin.segments {
        assert!(
            !s.exists(),
            "M14：默认成功后原段应随会话目录清理（{}）",
            s.display()
        );
    }

    let (codec, _w, _h, _fps) = probe_video_stream(&fin.final_path);
    assert_eq!(codec, "h264", "REQ-9/T6：v1 仅 H.264 输出");

    let dur = probe_duration(&fin.final_path);
    println!(
        "[single] 时长 {dur:.3}s（录制段 {span:.3}s），{} 个段",
        fin.segments.len()
    );
    assert!(
        (dur - span).abs() <= 2.0,
        "REQ-9：成品时长应≈录制段（{dur:.3}s vs {span:.3}s，容差 2s）"
    );

    assert_frames_nonblank_and_moving(&fin.final_path, 15);

    drop(win);
    let _ = std::fs::remove_dir_all(&out_dir);
}

/// REQ-20/24：管线产物有效性——固定画布尺寸/帧率、仅 1 条视频轨、关键帧间隔 ≤2s
/// （-g 60 + force_key_frames）、成品命名符合 `{yyyyMMdd_HHmmss}_{sanitize(目标名)}.mkv`。
#[test]
#[ignore = "L2：需交互式会话/WGC/可用编码器（specs §4）。显式运行：cargo test -p sr-itests --test route -- --ignored --nocapture"]
fn it_route_pipeline_mkv_valid() {
    require_interactive_session();
    let _os = record_os_build();

    let title = format!("SR-M3-RED-route-{}", std::process::id());
    let win = MotionWindow::new(&title, 800, 500);
    std::thread::sleep(Duration::from_millis(500));

    let out_dir = fresh_out_dir("pipeline");
    let h = start_route_ok(RouteConfig {
        target: RouteTarget::Window { hwnd: win.hwnd_u64 },
        video: VideoCfg {
            canvas_w: 640,
            canvas_h: 360,
            fps: 30,
            cursor: true,
        },
        output_dir: out_dir.clone(),
    });
    std::thread::sleep(Duration::from_secs(7));
    h.request_stop();
    let fin = h
        .wait_finalized(Duration::from_secs(120))
        .expect("收尾应成功");

    // ── 命名与位置（REQ-24） ──
    let name = fin
        .final_path
        .file_name()
        .expect("成品文件名")
        .to_string_lossy()
        .to_string();
    let stem = fin
        .final_path
        .file_stem()
        .expect("stem")
        .to_string_lossy()
        .to_string();
    let token = sanitize_target_name(&title);
    assert!(
        fin.final_path
            .extension()
            .map_or(false, |e| e.to_string_lossy() == "mkv"),
        "扩展名应为 .mkv（{name}）"
    );
    let p = fin.final_path.canonicalize().expect("canonicalize 成品");
    let d = out_dir.canonicalize().expect("canonicalize 输出目录");
    assert!(
        p.starts_with(&d),
        "成品应位于输出目录内（{} 不在 {} 下）",
        p.display(),
        d.display()
    );
    let stem_bytes = stem.as_bytes();
    assert!(
        stem_bytes.len() >= 16
            && stem_bytes[..8].iter().all(u8::is_ascii_digit)
            && stem_bytes[8] == b'_'
            && stem_bytes[9..15].iter().all(u8::is_ascii_digit)
            && stem_bytes[15] == b'_',
        "REQ-24：文件名应为 {{yyyyMMdd_HHmmss}}_{{目标名}} 形状（得到 {stem:?}）"
    );
    assert!(
        stem.contains(&token),
        "REQ-24：文件名应含 sanitize(目标名)={token:?}（得到 {stem:?}）"
    );

    // ── 流形状：固定画布（D6）、30fps、单视频轨无音频（M3 视频单路） ──
    let (codec, w, hgt, fps) = probe_video_stream(&fin.final_path);
    assert_eq!(codec, "h264", "编码应为 h264");
    assert_eq!(
        (w, hgt),
        (640, 360),
        "REQ-13/D6：输出画布应恒为配置值 640x360（得到 {w}x{hgt}）"
    );
    assert!((fps - 30.0).abs() <= 0.5, "帧率应为 30（得到 {fps}）");
    assert_eq!(
        probe_stream_count(&fin.final_path, "v"),
        1,
        "应恰有 1 条视频轨"
    );
    assert_eq!(
        probe_stream_count(&fin.final_path, "a"),
        0,
        "M3 视频单路：不应有音频轨"
    );

    // ── 关键帧间隔（闭合 GOP：-g 60@30fps ⇒ ≤2s；force_key_frames 段首 IDR） ──
    let pts = probe_keyframe_pts(&fin.final_path);
    println!("[pipeline] 关键帧 {} 个：{pts:?}", pts.len());
    assert!(
        pts.len() >= 2,
        "7s 成品应至少 2 个关键帧（得到 {}）",
        pts.len()
    );
    let max_gap = pts.windows(2).map(|w| w[1] - w[0]).fold(0.0f64, f64::max);
    assert!(
        max_gap <= 2.5,
        "REQ-20：关键帧最大间隔应 ≤2s（-g 60；实测 {max_gap:.3}s，容差 0.5s）"
    );

    drop(win);
    let _ = std::fs::remove_dir_all(&out_dir);
}

/// REQ-20/21：5s 分段——录制中已闭合段独立可播；停止收尾（concat）后总时长一致。
#[test]
#[ignore = "L2：需交互式会话/WGC/可用编码器（specs §4）。显式运行：cargo test -p sr-itests --test route -- --ignored --nocapture"]
fn it_segments_finalize_concat() {
    require_interactive_session();
    let _os = record_os_build();

    let title = format!("SR-M3-RED-route-{}", std::process::id());
    let win = MotionWindow::new(&title, 800, 500);
    std::thread::sleep(Duration::from_millis(500));

    let out_dir = fresh_out_dir("segments");
    // M14：本用例需检查原段/ffconcat 清单，显式启用 keep_segments（默认成功后原段被清理）。
    let h = start_route_with_keep_ok(
        RouteConfig {
            target: RouteTarget::Window { hwnd: win.hwnd_u64 },
            video: VideoCfg {
                canvas_w: 640,
                canvas_h: 360,
                fps: 30,
                cursor: true,
            },
            output_dir: out_dir.clone(),
        },
        true,
    );
    let t_rec = Instant::now();

    // ≥2 个段文件出现（≈11s：seg0 于 5s 边界闭合、seg1 开启）
    let n = wait_segment_count(&h, 2, Duration::from_secs(30));
    // 已闭合段（segments[0]）在录制进行中即独立可播（闭合 GOP）
    let seg0 = h.segments()[0].clone();
    let mid_dur = probe_duration_opt(&seg0).unwrap_or_else(|| {
        panic!(
            "REQ-21：录制中已闭合段应可独立读出时长（{}）",
            seg0.display()
        )
    });
    assert!(mid_dur > 0.0, "闭合段时长应为正（{mid_dur}）");
    assert!(
        decode_clean(&seg0),
        "REQ-21：录制中已闭合段应可独立解码播放（{}）",
        seg0.display()
    );
    println!("[segments] 录制中第 1 段已闭合可播：{mid_dur:.3}s（当前 {n} 段）");

    std::thread::sleep(Duration::from_secs(2));
    h.request_stop();
    let span = t_rec.elapsed().as_secs_f64();
    let fin = h
        .wait_finalized(Duration::from_secs(120))
        .expect("收尾应成功");

    assert!(
        fin.segments.len() >= 2,
        "13s 录制约应有 ≥2 段（得到 {}）",
        fin.segments.len()
    );
    assert!(
        fin.segment_list.exists(),
        "REQ-21：ffconcat 段清单应存在（{}）",
        fin.segment_list.display()
    );

    let mut seg_sum = 0.0;
    for s in &fin.segments {
        let d = probe_duration(s);
        seg_sum += d;
    }
    let dur = probe_duration(&fin.final_path);
    println!(
        "[segments] 成品 {dur:.3}s；Σ段 {seg_sum:.3}s（{} 段）；录制段 {span:.3}s",
        fin.segments.len()
    );
    assert!(
        (dur - seg_sum).abs() <= 0.75,
        "REQ-21：concat 收尾应保留完整时长（成品 {dur:.3}s vs Σ段 {seg_sum:.3}s，容差 0.75s）"
    );
    assert!(
        (dur - span).abs() <= 2.0,
        "REQ-21：成品时长应≈录制段（{dur:.3}s vs {span:.3}s）"
    );
    assert!(
        decode_clean(&fin.final_path),
        "成品应整文件可解码（concat 无损）"
    );

    drop(win);
    let _ = std::fs::remove_dir_all(&out_dir);
}

/// REQ-21（基础版）：录制中强杀 sidecar 进程树 → 该路 E_SIDECAR_EXIT；
/// 杀前已闭合的段仍独立可播、完成段永不丢弃、0 字节壳不列入段清单。
/// （主进程自杀不可行——完整杀进程矩阵属 REQ-22 的 `it_crash_kill_matrix_*`。）
#[test]
#[ignore = "L2：需交互式会话/WGC/可用编码器（specs §4）。显式运行：cargo test -p sr-itests --test route -- --ignored --nocapture"]
fn it_crash_kill_segments_playable() {
    require_interactive_session();
    let _os = record_os_build();

    let title = format!("SR-M3-RED-route-{}", std::process::id());
    let win = MotionWindow::new(&title, 800, 500);
    std::thread::sleep(Duration::from_millis(500));

    let out_dir = fresh_out_dir("crash");
    let h = start_route_ok(RouteConfig {
        target: RouteTarget::Window { hwnd: win.hwnd_u64 },
        video: VideoCfg {
            canvas_w: 640,
            canvas_h: 360,
            fps: 30,
            cursor: true,
        },
        output_dir: out_dir.clone(),
    });

    // ≥2 段出现（≈11s：seg0、seg1 已写出；seg0 已闭合），再录 6s 确保 seg1 也闭合
    let _ = wait_segment_count(&h, 2, Duration::from_secs(30));
    std::thread::sleep(Duration::from_secs(6));
    let pre = h.segments();
    assert!(pre.len() >= 2, "强杀前应已有 ≥2 个段（得到 {}）", pre.len());
    let closed_snapshot: Vec<PathBuf> = pre.iter().take(2).cloned().collect();
    println!("[crash] 强杀前段快照：{closed_snapshot:?}");

    let victims = ffmpeg_child_pids();
    assert!(
        !victims.is_empty(),
        "应能发现 ffmpeg sidecar 子进程（Toolhelp32 父子关系；sidecar 未 spawn？契约违背）"
    );
    for pid in &victims {
        kill_tree(*pid);
    }

    let r = h.wait_finalized(Duration::from_secs(90));
    match r {
        Ok(_) => panic!("REQ-21/§3.5：sidecar 被强杀后收尾不应成功（应报 E_SIDECAR_EXIT）"),
        Err(e) => assert!(
            matches!(e, RouteError::SidecarExit { .. }),
            "错误应为 RouteError::SidecarExit（E_SIDECAR_EXIT；得到 {e:?}）"
        ),
    }

    // 完成段永不丢弃（REQ-22）+ 0 字节壳不列入（S1b/S5 冻结）
    let after = h.segments();
    for s in &closed_snapshot {
        assert!(
            after.contains(s),
            "强杀后已闭合段不得从段清单消失（{}）",
            s.display()
        );
        assert!(s.exists(), "段文件应仍在磁盘（{}）", s.display());
        let d = probe_duration_opt(s).unwrap_or_else(|| {
            panic!(
                "REQ-21：强杀前已闭合的段应可独立读出时长（{}）",
                s.display()
            )
        });
        assert!(d > 0.0, "已闭合段时长应为正（{d}）");
        assert!(
            decode_clean(s),
            "REQ-21：强杀前已闭合的段应独立可播（{}）",
            s.display()
        );
    }
    for s in &after {
        assert!(
            s.metadata().map_or(false, |m| m.len() > 0),
            "0 字节壳不得列入段清单（{}）",
            s.display()
        );
    }
    println!(
        "[crash] E_SIDECAR_EXIT 已确认；{} 个段清单条目全部非 0 字节，闭合快照可播",
        after.len()
    );

    drop(win);
    let _ = std::fs::remove_dir_all(&out_dir);
}

/// REQ-11：光标捕获开关——cursor=true/false 两种配置的录制都成立且产出有效非黑画面。
/// （「能力不可用时降级不阻塞」需故障注入钩子，属 REQ-28 的
/// `it_probe_permission_matrix_injected`；本用例钉开关语义与两种配置的有效性。）
#[test]
#[ignore = "L2：需交互式会话/WGC/可用编码器（specs §4）。显式运行：cargo test -p sr-itests --test route -- --ignored --nocapture"]
fn it_cursor_toggle_degrades() {
    require_interactive_session();
    let _os = record_os_build();

    let title = format!("SR-M3-RED-route-{}", std::process::id());
    let win = MotionWindow::new(&title, 800, 500);
    std::thread::sleep(Duration::from_millis(500));

    let mut finals: Vec<PathBuf> = Vec::new();
    for (mode, cursor) in [("on", true), ("off", false)] {
        let out_dir = fresh_out_dir(&format!("cursor-{mode}"));
        let h = start_route_ok(RouteConfig {
            target: RouteTarget::Window { hwnd: win.hwnd_u64 },
            video: VideoCfg {
                canvas_w: 640,
                canvas_h: 360,
                fps: 30,
                cursor,
            },
            output_dir: out_dir.clone(),
        });
        std::thread::sleep(Duration::from_secs(5));
        h.request_stop();
        let fin = h
            .wait_finalized(Duration::from_secs(120))
            .unwrap_or_else(|e| panic!("REQ-11：cursor={cursor} 的录制收尾应成功（得到 {e:?}）"));

        assert!(
            fin.final_path.exists(),
            "cursor={cursor}：成品应存在（{}）",
            fin.final_path.display()
        );
        let (codec, _w, _h, _fps) = probe_video_stream(&fin.final_path);
        assert_eq!(codec, "h264", "cursor={cursor}：编码应为 h264");
        let dur = probe_duration(&fin.final_path);
        assert!(
            dur >= 3.0,
            "cursor={cursor}：5s 录制时长应 ≥3s（得到 {dur:.3}）"
        );
        assert_frames_nonblank_and_moving(&fin.final_path, 8);
        println!("[cursor-{mode}] cursor={cursor}：{dur:.3}s，非黑且运动 ✓");
        finals.push(fin.final_path.clone());
        let _ = std::fs::remove_dir_all(&out_dir);
    }

    assert_ne!(finals[0], finals[1], "两次录制的成品路径应不同");
    drop(win);
}
