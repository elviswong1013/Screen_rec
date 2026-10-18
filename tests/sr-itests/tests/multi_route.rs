//! L2 集成测试：多路同时录制（REQ-12 的 M5 范围，specs/001-screen-recorder.md
//! §2.3（REQ-12）、§3.1/§3.2/§3.3（start_routes 批量契约/管道协议）、§3.4、§4、
//! §5 M5 行（含三路停中）、§9 T1（保底 2 路 1080p30）、§8 M0 冻结项（S1：双/三路
//! 30fps 实测通过）。
//!
//! 运行方式（L2 门：`#[ignore]`，默认 `cargo test` 不执行本文件用例）：
//! ```text
//! cargo test -p sr-itests --test multi_route -- --ignored --nocapture
//! cargo test -p sr-itests --test multi_route it_three_routes_stop_middle -- --ignored --nocapture   # 单用例
//! ```
//! 前置条件：交互式用户会话 + WGC 可用 + 可用编码器（D4 链）+ 仓库内锁定 ffmpeg
//! n8.1 release 构建（spec §8；含 ffprobe；音频子断言另需默认渲染端点）；缺失 →
//! BLOCKED（fail），不静默 skip（§4）。
//! 本机环境（红测试编写时）：Windows 11 25H2，build 26200.9711，SESSIONNAME=Console。
//!
//! 接口契约（测试即契约，详见 docs/milestones/m5-red-report.md §3）：
//! - `sr_pipeline::route::MultiRouteConfig { target: RouteTarget, video: VideoCfg,
//!   audio: Vec<sr_audio::wasapi::AudioSourceSpec>, output_dir: PathBuf }`
//!   多路批量启动的单路配置面（§3.4 RouteConfig 的多路投影）：video 必填（D6 固定
//!   画布）；audio 空 = 纯视频路，非空 = 每 App 源一条独立音轨（M4 隔离模式语义，
//!   「视频+可选音频」）。既有 `RouteConfig`/`AudioRouteConfig`（M3/M4 测试字面量）
//!   不动。
//! - `sr_pipeline::route::start_routes(Vec<MultiRouteConfig>) -> Result<Vec<RouteHandle>, RouteError>`
//!   （§3.3 `start_routes` 的管线层对应物）：**全成功或回滚**——Ok(handles) 与 cfgs
//!   同序同长、每路均已到达「输出开始」（§3.1 启动协议逐路完成）；任一路启动失败
//!   （如无效目标 → TargetInvalid）→ 已启动各路全部停止清理后返回 Err（测试断言
//!   回滚后无 sidecar 子进程残留、有效路无成品）。RouteHandle/RouteError/Finalized
//!   语义复用 M3/M4（request_stop / wait_finalized / segments；SidecarExit 强匹配）。
//!
//! 手法：每路被录目标是**自带唯一颜色标记**的自建运动窗口（专用泵线程 + 33ms 定时
//! 器 + 静态 50%×50% 标记块 + 全高移动亮条；路 0 绿 (60,220,90) / 路 1 品红
//! (220,60,200) / 路 2 青 (60,200,220)）——内容级判定「各自独立、互不串扰」：
//! 采样解码帧中**本路标记存在**（≥3% 像素命中）且**其它路标记不存在**（≤0.5%），
//! 配合 M3 先例的非黑/运动判定；标记色与图案元素（灰底/白条）在箱式判定域两两
//! 不相交（阈值经 scratch 实测标定，见红报告 §6）。容器/时长/轨道形状用 ffprobe；
//! 单路故障注入 = Toolhelp32 发现 ffmpeg sidecar 子进程后 `taskkill /T /F` 其中
//! 一个（对称断言：恰 1 路 SidecarExit、其余 Ok——不依赖「哪个 sidecar 属于哪路」
//! 的映射知识）。

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};

use common::*;
use sr_audio::wasapi::AudioSourceSpec;
use sr_pipeline::naming::sanitize_target_name;
use sr_pipeline::route::{
    start_routes, Finalized, MultiRouteConfig, RouteError, RouteHandle, RouteTarget, VideoCfg,
};

/// L2 多路用例串行门：同一测试二进制内 4 个用例各自起 2–3 路路由/多个测试窗口，
/// 串行执行（避免用例间并发叠加超出已验证并发面；M4 R5 先例。隔离语义本身不受
/// 串行影响——每路管道/sidecar/输出独立）。
static GATE: Mutex<()> = Mutex::new(());

fn gate() -> std::sync::MutexGuard<'static, ()> {
    GATE.lock().unwrap_or_else(|e| e.into_inner())
}

// ─────────────────────────── 锁定构建 / 输出目录 ───────────────────────────

/// 锁定 ffmpeg/ffprobe/ffplay 位置：spec §8 M0 冻结的 BtbN **n8.1 release** 构建。
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
    let d = std::env::temp_dir().join(format!("sr_m5_red_multi_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("创建输出目录失败");
    d
}

/// start_routes 的 BLOCKED 包装：环境缺失（无编码器/WGC 被拒/环回不可用）→ BLOCKED；
/// 其它 Err → panic。
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
        Err(e) if matches!(e, RouteError::LoopbackUnavailable { .. }) => blocked(&format!(
            "进程环回不可用（E_LOOPBACK_UNAVAILABLE，§3.5 环境缺失）：{e:?}"
        )),
        Err(e) => panic!("start_routes 应成功（得到 {e:?}）"),
    }
}

// ─────────────────────────── ffprobe / 解码交叉验证（M3/M4 同款） ───────────────────────────

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

fn probe_duration_opt(path: &Path) -> Option<f64> {
    let s = ffprobe_csv(
        path,
        &["-show_entries", "format=duration", "-of", "csv=p=0"],
    );
    s.parse::<f64>().ok().filter(|d| *d > 0.0)
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

/// 整文件可解码（ffmpeg -v error -f null -）。
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

/// 低分辨率低帧率采样解码（rgb24 帧；M3 同款）。
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

/// 解码首音轨为 48k 单声道 f32（M4 同款）。
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

// ─────────────────────────── 内容级判定阈值（构造性保证 + scratch 实测标定） ───────────────────────────
const SAMPLE_FPS: u32 = 3;
const SAMPLE_W: u32 = 64;
const SAMPLE_H: u32 = 36;
const FRAME_STD_MIN: f64 = 2.0;
const MOTION_DIFF_MIN: f64 = 2.0;
const MOTION_PAIR_RATIO_MIN: f64 = 0.30;
const MOTION_MAX_DIFF_MIN: f64 = 10.0;
/// 本路标记色最小像素占比（50%×50% 标记块 + 亮条穿越的瞬时遮蔽后仍有 ≈6–11%
/// 量级；1080p 低码率编码会软化色度边缘，阈值留 3× 裕度——scratch 实测标定）。
const OWN_MARKER_MIN: f64 = 0.02;
/// 其它路标记色最大像素占比（串扰=0；仅容忍边缘混色零星像素）。
const OTHER_MARKER_MAX: f64 = 0.005;
/// 音轨非静音 RMS 下限（440Hz 音量 0.25 理论 RMS ≈ 0.18；实测经端点增益后
/// 0.022 量级、随机器音量浮动，数字静音 ≈ 1e-4 量级——阈值取两者之间）。
const AUDIO_RMS_MIN: f64 = 0.005;

/// 采样帧中命中某路标记色箱式判定的像素占比。
fn marker_fraction(frame: &[u8], idx: usize) -> f64 {
    let n = frame.len() / 3;
    let hits = (0..n)
        .filter(|&i| {
            let r = frame[i * 3] as i32;
            let g = frame[i * 3 + 1] as i32;
            let b = frame[i * 3 + 2] as i32;
            match idx {
                // 绿 (60,220,90)：绿主导、红蓝均低
                0 => r < 130 && g > 170 && b < 150 && g - r > 60 && g - b > 60,
                // 品红 (220,60,200)：红蓝主导、绿低
                1 => r > 170 && g < 130 && b > 160 && r - g > 60 && b - g > 60,
                // 青 (60,200,220)：绿蓝主导、红低
                _ => r < 130 && g > 160 && b > 170 && g - r > 60 && b - r > 60,
            }
        })
        .count();
    hits as f64 / n as f64
}

/// 内容级判定（REQ-9/12）：非黑 + 运动（M3 阈值）+ 本路标记存在 + 其它路标记不存在。
fn assert_frames_route_identified(path: &Path, own: usize, min_frames: usize, label: &str) {
    let frames = decode_frames_rgb(path);
    assert!(
        frames.len() >= min_frames,
        "{label}：采样帧数应 ≥{min_frames}（得到 {}；3fps 采样下时长不足？）",
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
        "{label}：所有采样帧应为非黑帧（亮度标准差 ≥{FRAME_STD_MIN}；黑帧：{blank:?}）"
    );
    let mut diffs = Vec::new();
    for w in frames.windows(2) {
        diffs.push(mean_abs_diff(&w[0], &w[1]));
    }
    let moving = diffs.iter().filter(|d| **d >= MOTION_DIFF_MIN).count();
    let max_d = diffs.iter().cloned().fold(0.0f64, f64::max);
    assert!(
        moving as f64 / diffs.len() as f64 >= MOTION_PAIR_RATIO_MIN,
        "{label}：相邻采样帧差异应体现运动（≥{:.0}% 的对差 ≥{MOTION_DIFF_MIN}；得到 {moving}/{}）",
        MOTION_PAIR_RATIO_MIN * 100.0,
        diffs.len()
    );
    assert!(
        max_d >= MOTION_MAX_DIFF_MIN,
        "{label}：最大帧间差应 ≥{MOTION_MAX_DIFF_MIN}（得到 {max_d:.2}：疑似静帧/单色捕获）"
    );
    let mut own_min = f64::MAX;
    let mut other_max = 0.0f64;
    for f in &frames {
        own_min = own_min.min(marker_fraction(f, own));
        for o in 0..ROUTE_MARKERS_LEN {
            if o != own {
                other_max = other_max.max(marker_fraction(f, o));
            }
        }
    }
    println!(
        "[marker:{label}] 本路#{own} 最小占比 {own_min:.4}（阈值 ≥{OWN_MARKER_MIN}）；其它路最大占比 {other_max:.4}（阈值 ≤{OTHER_MARKER_MAX}）；运动对 {moving}/{}，最大帧差 {max_d:.2}",
        diffs.len()
    );
    // 逐帧细粒度日志（仅日志增强，不改动断言/阈值）：失败时直接定位受害帧与受害路
    for (i, f) in frames.iter().enumerate() {
        let per_frame: Vec<String> = (0..ROUTE_MARKERS_LEN)
            .map(|o| format!("#{o}={:.4}", marker_fraction(f, o)))
            .collect();
        println!("[marker:{label}] 帧 {i:>2}：{}", per_frame.join("；"));
    }
    assert!(
        own_min >= OWN_MARKER_MIN,
        "REQ-12：{label} 采样帧应含本路（#{own}）标记色（最小占比 {own_min:.4} < {OWN_MARKER_MIN}：目标串线/错误画布？）"
    );
    assert!(
        other_max <= OTHER_MARKER_MAX,
        "REQ-12：{label} 采样帧不得含其它路标记色（最大占比 {other_max:.4} > {OTHER_MARKER_MAX}：多路串扰——录到了别的目标？）"
    );
}

/// 单路成品公共断言：存在/MKV/位于自己的输出目录/命名含 sanitize(目标名)/恰 1 条
/// h264 视频轨/固定画布与帧率/时长≈录制段（±2s）/整文件可解码。
fn assert_route_common(
    fin: &Finalized,
    span: f64,
    title: &str,
    out_dir: &Path,
    canvas: (u32, u32),
    label: &str,
) {
    assert!(
        fin.final_path.exists(),
        "{label}：成品应存在（{}）",
        fin.final_path.display()
    );
    assert!(
        fin.final_path
            .extension()
            .map_or(false, |e| e.to_string_lossy() == "mkv"),
        "{label}：成品应为 MKV（{}）",
        fin.final_path.display()
    );
    let p = fin
        .final_path
        .canonicalize()
        .unwrap_or_else(|e| panic!("{label}：canonicalize 成品失败：{e}"));
    let d = out_dir
        .canonicalize()
        .unwrap_or_else(|e| panic!("{label}：canonicalize 输出目录失败：{e}"));
    assert!(
        p.starts_with(&d),
        "{label}：成品应位于自己的输出目录内（{} 不在 {} 下）",
        p.display(),
        d.display()
    );
    let stem = fin
        .final_path
        .file_stem()
        .expect("stem")
        .to_string_lossy()
        .to_string();
    let token = sanitize_target_name(title);
    assert!(
        stem.contains(&token),
        "{label}：文件名应含 sanitize(目标名)={token:?}（得到 {stem:?}）"
    );
    assert_eq!(
        probe_stream_count(&fin.final_path, "v"),
        1,
        "{label}：应恰 1 条视频轨"
    );
    let (codec, w, h, fps) = probe_video_stream(&fin.final_path);
    assert_eq!(codec, "h264", "{label}：T6：v1 仅 H.264 输出");
    assert_eq!(
        (w, h),
        canvas,
        "{label}：D6：输出画布应恒为配置值 {}x{}（得到 {w}x{h}）",
        canvas.0,
        canvas.1
    );
    assert!(
        (fps - 30.0).abs() <= 0.5,
        "{label}：帧率应为 30（得到 {fps}）"
    );
    let dur = probe_duration(&fin.final_path);
    println!("[common:{label}] 时长 {dur:.3}s（录制段 {span:.3}s）");
    assert!(
        (dur - span).abs() <= 2.0,
        "{label}：成品时长应≈录制段（{dur:.3}s vs {span:.3}s，容差 2s）"
    );
    assert!(decode_clean(&fin.final_path), "{label}：成品应整文件可解码");
}

// ─────────────────────────── 唯一标记色运动窗口（专用泵线程） ───────────────────────────

const MARKER_CLASS_NAME: &str = "SR_M5_RED_MARKER_WND";
static MARKER_CLASS: Once = Once::new();
static PAINT_COUNT: AtomicU32 = AtomicU32::new(0);
/// 每路唯一标记色（静态 40%×40% 左上块；箱式判定域两两不相交，见 marker_fraction）。
const ROUTE_MARKERS: [(u8, u8, u8); 3] = [(60, 220, 90), (220, 60, 200), (60, 200, 220)];
const ROUTE_MARKERS_LEN: usize = 3;

/// WM_PAINT 双缓冲（每窗口泵线程独占一份，见 `paint_buf_for`）：整帧先画进内存
/// DC，最后一次 BitBlt 提交——GDI 分步刷新期间捕获端会取到「只有底色、标记/亮条
/// 未画」的中间态（2026-10-17 抖动加固；实测受害帧整帧纯底色）。
struct PaintBuf {
    mem: windows::Win32::Graphics::Gdi::HDC,
    bmp: windows::Win32::Graphics::Gdi::HBITMAP,
    old: windows::Win32::Graphics::Gdi::HGDIOBJ,
    size: (i32, i32),
}

impl Drop for PaintBuf {
    fn drop(&mut self) {
        use windows::Win32::Graphics::Gdi::{DeleteDC, DeleteObject, SelectObject, HGDIOBJ};
        unsafe {
            let _ = SelectObject(self.mem, self.old);
            let _ = DeleteObject(HGDIOBJ(self.bmp.0));
            let _ = DeleteDC(self.mem);
        }
    }
}

thread_local! {
    static PAINT_BUF: std::cell::RefCell<Option<PaintBuf>> = const { std::cell::RefCell::new(None) };
}

/// 取（必要时按当前客户区尺寸重建）本线程的双缓冲内存 DC；尺寸变化重建，
/// 旧缓冲随 `PaintBuf::drop` 释放。
fn paint_buf_for(
    hdc: windows::Win32::Graphics::Gdi::HDC,
    w: i32,
    h: i32,
) -> windows::Win32::Graphics::Gdi::HDC {
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, SelectObject, HGDIOBJ,
    };
    PAINT_BUF.with(|cell| {
        let mut slot = cell.borrow_mut();
        if let Some(b) = &*slot {
            if b.size == (w, h) {
                return b.mem;
            }
        }
        drop(slot.take()); // 旧缓冲 Drop 即释放 GDI 对象
        unsafe {
            let mem = CreateCompatibleDC(Some(hdc));
            let bmp = CreateCompatibleBitmap(hdc, w, h);
            let old = SelectObject(mem, HGDIOBJ(bmp.0));
            *slot = Some(PaintBuf {
                mem,
                bmp,
                old,
                size: (w, h),
            });
            mem
        }
    })
}

unsafe extern "system" fn marker_wndproc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wp: windows::Win32::Foundation::WPARAM,
    lp: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::{LRESULT, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, GetWindowLongPtrW, KillTimer, PostQuitMessage, SetWindowLongPtrW,
        SetWindowPos, CREATESTRUCTW, GWLP_USERDATA, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOSIZE, WM_DESTROY, WM_NCCREATE, WM_PAINT, WM_TIMER,
    };
    unsafe {
        match msg {
            // 窗口创建期把 CreateWindowExW 的 lpCreateParams（路序号）挂到 GWLP_USERDATA
            WM_NCCREATE => {
                let cs = &*(lp.0 as *const CREATESTRUCTW);
                let idx = cs.lpCreateParams as usize;
                let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, idx as isize);
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_TIMER => {
                // 33ms 定时失效 → 重绘（运动由 WM_TIMER 驱动）
                use windows::Win32::Graphics::Gdi::InvalidateRect;
                let _ = InvalidateRect(Some(hwnd), None, false);
                // 录制期间维持置顶（33ms 心跳重申，HWND_TOPMOST + 不动几何）：外部置顶
                // 窗口/浮层出现后标记窗口自动回到最前（2026-10-17 抖动加固）
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
                LRESULT(0)
            }
            WM_PAINT => {
                use windows::Win32::Graphics::Gdi::{
                    BeginPaint, BitBlt, CreateSolidBrush, DeleteObject, EndPaint, FillRect,
                    PAINTSTRUCT, SRCCOPY,
                };
                use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
                let idx = GetWindowLongPtrW(hwnd, GWLP_USERDATA).clamp(0, 2) as usize;
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                let w = (rc.right - rc.left).max(1);
                let h = (rc.bottom - rc.top).max(1);
                let n = PAINT_COUNT.fetch_add(1, Ordering::Relaxed) as i32;
                // 双缓冲：全部绘制落在内存 DC，最后一次性 BitBlt 提交（见 PaintBuf 注释）
                let mem = paint_buf_for(hdc, w, h);

                let brush = |r: u32, g: u32, b: u32| {
                    windows::Win32::Foundation::COLORREF(r | (g << 8) | (b << 16))
                };
                // 1) 深灰底
                let bg = CreateSolidBrush(brush(64, 64, 64));
                FillRect(mem, &rc, bg);
                let _ = DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(bg.0));
                // 2) 静态标记块（本路唯一色；左上 50%×50%，占比 ≈25%）
                let m = ROUTE_MARKERS[idx];
                let marker = CreateSolidBrush(brush(m.0 as u32, m.1 as u32, m.2 as u32));
                let marker_rc = RECT {
                    left: rc.left,
                    top: rc.top,
                    right: rc.left + w / 2,
                    bottom: rc.top + h / 2,
                };
                FillRect(mem, &marker_rc, marker);
                let _ = DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(marker.0));
                // 3) 全高移动亮条（每帧 +37px，环绕；提供运动）
                let bar_w = (w / 10).max(24);
                let bar_x = (n * 37) % w;
                let bar = CreateSolidBrush(brush(235, 240, 255));
                let bar_rc = RECT {
                    left: rc.left + bar_x,
                    top: rc.top,
                    right: rc.left + bar_x + bar_w,
                    bottom: rc.bottom,
                };
                FillRect(mem, &bar_rc, bar);
                let _ = DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(bar.0));

                // 4) 一次性提交到窗口 DC（原子呈现，杜绝中间态入画）
                let _ = BitBlt(hdc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = KillTimer(Some(hwnd), 1);
                // 释放本线程双缓冲的 GDI 对象（内存 DC + 目标位图）
                PAINT_BUF.with(|cell| {
                    drop(cell.borrow_mut().take());
                });
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

fn register_marker_class() {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::RegisterClassW;
    MARKER_CLASS.call_once(|| unsafe {
        let hinst = GetModuleHandleW(None).ok().expect("GetModuleHandleW");
        let class_name = windows::core::HSTRING::from(MARKER_CLASS_NAME);
        let wc = windows::Win32::UI::WindowsAndMessaging::WNDCLASSW {
            lpfnWndProc: Some(marker_wndproc),
            hInstance: hinst.into(),
            lpszClassName: windows::core::PCWSTR::from_raw(class_name.as_ptr()),
            ..Default::default()
        };
        let atom = RegisterClassW(&wc);
        assert_ne!(atom, 0, "RegisterClassW({MARKER_CLASS_NAME}) 失败");
    });
}

/// 自建唯一标记色运动窗口：内容在**专用线程**的消息泵驱动下持续变化（33ms 定时
/// 器），与被测 start_routes（主测试线程）解耦；Drop 时 PostMessage(WM_CLOSE) + join。
/// marker_idx ∈ 0..3 决定标记色（ROUTE_MARKERS）与**工作区槽位**（3 等分横排、
/// 互不遮挡——CW_USEDEFAULT 级联会叠放，触发 WGC 对被遮挡 GDI 窗口的表面节流
/// （scratch 实测：约 15% 轮次内容缩成 ~60% 画布+黑边；显式铺排后消除）；
/// 几何确定化亦利于内容级断言的稳定性）。窗口置顶（HWND_TOPMOST，创建时设置 +
/// 33ms 心跳重申）：外部窗口覆盖左上标记区会吞掉标记色；整帧绘制经内存 DC
/// 双缓冲后一次性 BitBlt 提交，杜绝分步刷新中间态被捕获（2026-10-17 抖动加固）。
pub struct MarkerWindow {
    pub hwnd_u64: u64,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl MarkerWindow {
    pub fn new(title: &str, marker_idx: usize, w: i32, h: i32) -> MarkerWindow {
        assert!(
            marker_idx < ROUTE_MARKERS_LEN,
            "marker_idx 越界（{marker_idx}）"
        );
        let (tx, rx) = mpsc::channel();
        let title = title.to_string();
        let thread = std::thread::Builder::new()
            .name(format!("sr-m5-marker-{marker_idx}"))
            .spawn(move || {
                use windows::Win32::Graphics::Gdi::UpdateWindow;
                use windows::Win32::UI::WindowsAndMessaging::{
                    CreateWindowExW, DispatchMessageW, GetMessageW, SetTimer, SetWindowPos,
                    TranslateMessage, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
                    WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
                };
                unsafe {
                    register_marker_class();
                    let hinst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
                        .ok()
                        .expect("GetModuleHandleW");
                    let class_name = windows::core::HSTRING::from(MARKER_CLASS_NAME);
                    let title_w = windows::core::HSTRING::from(title.as_str());
                    // 显式槽位（互不遮挡）：x = 工作区 3 等分，y 固定 40；宽度防御性
                    // 收进槽内（窄屏下避免再次叠放）
                    let slot_w = {
                        use windows::Win32::Foundation::RECT;
                        use windows::Win32::UI::WindowsAndMessaging::{
                            SystemParametersInfoW, SPI_GETWORKAREA,
                            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
                        };
                        let mut rc = RECT::default();
                        let ok = SystemParametersInfoW(
                            SPI_GETWORKAREA,
                            0,
                            Some(&mut rc as *mut RECT as *mut core::ffi::c_void),
                            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
                        )
                        .is_ok();
                        if ok && rc.right - rc.left > 0 {
                            (rc.right - rc.left) / 3
                        } else {
                            640
                        }
                    };
                    let x = 10 + marker_idx as i32 * slot_w.max(220);
                    let w = w.min(slot_w - 20);
                    let hwnd = CreateWindowExW(
                        WINDOW_EX_STYLE(0),
                        windows::core::PCWSTR::from_raw(class_name.as_ptr()),
                        windows::core::PCWSTR::from_raw(title_w.as_ptr()),
                        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                        x,
                        40,
                        w,
                        h,
                        None,
                        None,
                        Some(hinst.into()),
                        Some(marker_idx as usize as *const core::ffi::c_void),
                    )
                    .expect("CreateWindowExW 失败：无法构造标记窗口");
                    // 置顶且不可被遮挡（2026-10-17 抖动加固，详见
                    // docs/milestones/m6-green-report.md「抖动加固」小节）：外部窗口覆盖左上
                    // 标记区会吞掉标记色而运动仍在（受害路随机的 flake 根因）；SWP_NOMOVE|
                    // SWP_NOSIZE 保持现 3 槽位几何逐字不变。
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
            .expect("spawn 标记窗口线程");
        let hwnd_u64 = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("标记窗口创建超时（泵线程未回传 hwnd）");
        MarkerWindow {
            hwnd_u64,
            thread: Some(thread),
        }
    }
}

impl Drop for MarkerWindow {
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

// ─────────────────────────── sidecar 子进程发现 / 强杀 / 音源（M3/M4 同款） ───────────────────────────

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

/// 进程树退出即清理的守卫（ffplay 音源）。
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

/// 用锁定 ffmpeg 生成测试音 WAV（sine 48k 立体声 f32；音量 0.25，M4 R2 同款）。
fn gen_tone_wav(dir: &Path, name: &str, hz: u32) -> PathBuf {
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
            &format!("sine=frequency={hz}:sample_rate=48000:duration=40,volume=0.25"),
            "-ac",
            "2",
            "-c:a",
            "pcm_f32le",
        ])
        .arg(&out)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap_or_else(|e| blocked(&format!("无法运行锁定 ffmpeg 生成测试音：{e}")));
    if !st.success() {
        blocked(&format!("ffmpeg 生成 {hz}Hz 测试音失败（exit={st}）"));
    }
    out
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

fn audio_rms(x: &[f32]) -> f64 {
    let s: f64 = x.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / x.len() as f64;
    s.sqrt()
}

// ─────────────────────────── 用例 ───────────────────────────

/// REQ-12（§4 点名）：≥2 路同时录制——本用例 2 路 1080p30（§9 T1 保底并发），
/// 路 A 视频+可选音频（应用音源 ffplay 440Hz）、路 B 纯视频；**停一路**经
/// 「关闭路 B 的目标窗口」实现（REQ-12：目标窗口关闭 → 该路优雅停止并保留内容）；
/// 路 A 不受影响继续录制并正常收尾；两路独立输出文件、内容互不串扰（标记色）。
#[test]
#[ignore = "L2：需交互式会话/WGC/可用编码器/默认渲染端点（specs §4）。显式运行：cargo test -p sr-itests --test multi_route -- --ignored --nocapture"]
fn it_two_routes_isolated_stop_one() {
    let _g = gate();
    require_interactive_session();
    let _os = record_os_build();

    let pid = std::process::id();
    let tmp = fresh_out_dir("two");
    let wav = gen_tone_wav(&tmp, "a440.wav", 440);
    let player = spawn_ffplay(&wav);
    std::thread::sleep(Duration::from_millis(1500)); // 起播
    let src_pid = player.0.id();
    let src_created = process_created_unix_ms(src_pid)
        .unwrap_or_else(|| blocked("无法读取 ffplay 创建时间（身份地面真值）"));

    let title_a = format!("SR-M5-RED-two-A-{pid}");
    let title_b = format!("SR-M5-RED-two-B-{pid}");
    let win_a = MarkerWindow::new(&title_a, 0, 620, 420);
    let win_b = MarkerWindow::new(&title_b, 1, 620, 420);
    std::thread::sleep(Duration::from_millis(500)); // 图案起转

    let out_a = fresh_out_dir("two-a");
    let out_b = fresh_out_dir("two-b");
    let handles = start_routes_ok(vec![
        MultiRouteConfig {
            target: RouteTarget::Window {
                hwnd: win_a.hwnd_u64,
            },
            video: VideoCfg {
                canvas_w: 1920,
                canvas_h: 1080,
                fps: 30,
                cursor: true,
            },
            audio: vec![AudioSourceSpec::App {
                pid: src_pid,
                created_at_unix_ms: src_created,
            }],
            output_dir: out_a.clone(),
        },
        MultiRouteConfig {
            target: RouteTarget::Window {
                hwnd: win_b.hwnd_u64,
            },
            video: VideoCfg {
                canvas_w: 1920,
                canvas_h: 1080,
                fps: 30,
                cursor: true,
            },
            audio: vec![],
            output_dir: out_b.clone(),
        },
    ]);
    std::thread::sleep(Duration::from_millis(300));
    let t0 = Instant::now();

    // ~5.5s 后停一路：关闭路 B 的目标窗口（优雅停止 + 保留内容）
    std::thread::sleep(Duration::from_millis(5500));
    let span_b = t0.elapsed().as_secs_f64();
    drop(win_b);
    let fin_b = handles[1]
        .wait_finalized(Duration::from_secs(120))
        .expect("REQ-12：目标窗口关闭 → 该路应优雅停止并保留内容（Ok 收尾 + concat）");

    // 路 A 不受影响：继续录到 ~9s
    while t0.elapsed() < Duration::from_secs(9) {
        std::thread::sleep(Duration::from_millis(100));
    }
    handles[0].request_stop();
    let span_a = t0.elapsed().as_secs_f64();
    let fin_a = handles[0]
        .wait_finalized(Duration::from_secs(120))
        .expect("REQ-12：另一路停止/收尾期间，剩余路应不受影响并正常收尾");

    // ── 独立输出文件 ──
    assert_ne!(
        fin_a.final_path, fin_b.final_path,
        "REQ-12：两路成品路径应互异（独立输出文件）"
    );
    assert_route_common(
        &fin_a,
        span_a,
        &title_a,
        &out_a,
        (1920, 1080),
        "两路·A（视频+音频）",
    );
    assert_route_common(
        &fin_b,
        span_b,
        &title_b,
        &out_b,
        (1920, 1080),
        "两路·B（纯视频）",
    );

    // ── 可选音频：带音源的路恰 1 条应用音轨（aac/48k/立体声）且非静音；纯视频路 0 音轨 ──
    assert_eq!(
        probe_stream_count(&fin_a.final_path, "a"),
        1,
        "REQ-12：带音源的路应恰 1 条应用音轨（隔离模式，M4 语义）"
    );
    let (acodec, ar, ch) = probe_audio_stream(&fin_a.final_path);
    assert_eq!(acodec, "aac", "§3.3 基准：音频编码 aac");
    assert_eq!(ar, 48000, "REQ-18：音轨 48kHz");
    assert_eq!(ch, 2, "REQ-18：音轨立体声");
    assert_eq!(
        probe_stream_count(&fin_b.final_path, "a"),
        0,
        "REQ-12：纯视频路不应有音频轨"
    );
    let mono = decode_audio_mono(&fin_a.final_path);
    assert!(
        mono.len() > 48_000 * 6,
        "路 A 音轨样本应 ≥6s（得到 {} 帧）",
        mono.len()
    );
    let mid = &mono[48_000..mono.len() - 48_000];
    let rms = audio_rms(mid);
    println!("[two] 路 A 音轨 RMS（去首尾 1s）= {rms:.4}");
    assert!(
        rms >= AUDIO_RMS_MIN,
        "REQ-12：路 A 音轨应含 440Hz 测试音（RMS {rms:.4} < {AUDIO_RMS_MIN}：静音轨？）"
    );

    // ── 「停一路」后另一路仍持续：时长明显更长 ──
    let dur_a = probe_duration(&fin_a.final_path);
    let dur_b = probe_duration(&fin_b.final_path);
    assert!(
        dur_a > dur_b + 1.5,
        "REQ-12：路 A 应在路 B 停止后仍持续录制（{dur_a:.3}s vs {dur_b:.3}s）"
    );

    // ── 内容互不串扰 ──
    assert_frames_route_identified(&fin_a.final_path, 0, 21, "两路·A");
    assert_frames_route_identified(&fin_b.final_path, 1, 11, "两路·B");
    assert!(
        decode_clean(&fin_a.final_path) && decode_clean(&fin_b.final_path),
        "两路成品均应整文件可解码"
    );

    drop(win_a);
    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(&out_a);
    let _ = std::fs::remove_dir_all(&out_b);
}

/// REQ-12（§4/§5 M5 点名）：三路停中——3 路 1080p30 同时录制（S1 实测三路可行；
/// §9 T1：2 路保底、第 3 路按硬件能力），运行中停止**中间一路**：该路收尾成功、
/// 两侧路不受影响（继续录满更长录制段）、三路成品两两互异、每路内容指向各自
/// 目标（标记色互不串扰）、全部可收尾可播。
#[test]
#[ignore = "L2：需交互式会话/WGC/可用编码器（specs §4）。显式运行：cargo test -p sr-itests --test multi_route -- --ignored --nocapture"]
fn it_three_routes_stop_middle() {
    let _g = gate();
    require_interactive_session();
    let _os = record_os_build();

    let pid = std::process::id();
    let mut titles = Vec::new();
    let mut wins = Vec::new();
    let mut outs = Vec::new();
    let mut cfgs = Vec::new();
    for i in 0..3 {
        let title = format!("SR-M5-RED-three-{i}-{pid}");
        let win = MarkerWindow::new(&title, i, 620, 420);
        let out = fresh_out_dir(&format!("three-{i}"));
        cfgs.push(MultiRouteConfig {
            target: RouteTarget::Window { hwnd: win.hwnd_u64 },
            video: VideoCfg {
                canvas_w: 1920,
                canvas_h: 1080,
                fps: 30,
                cursor: true,
            },
            audio: vec![],
            output_dir: out.clone(),
        });
        titles.push(title);
        outs.push(out);
        wins.push(win);
    }
    std::thread::sleep(Duration::from_millis(500)); // 图案起转

    let handles = start_routes_ok(cfgs);
    std::thread::sleep(Duration::from_millis(300));
    let t0 = Instant::now();

    // ~6s：三路均在录 → 停中间一路
    std::thread::sleep(Duration::from_secs(6));
    let span_mid = t0.elapsed().as_secs_f64();
    handles[1].request_stop();
    let fin_mid = handles[1]
        .wait_finalized(Duration::from_secs(120))
        .expect("REQ-12：中间路停止后应收尾成功（EOF 优雅停止 + 后台 concat）");

    // 其余两路不受影响：继续录到 ~10s
    while t0.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(100));
    }
    handles[0].request_stop();
    handles[2].request_stop();
    let span_side = t0.elapsed().as_secs_f64();
    let fin_0 = handles[0]
        .wait_finalized(Duration::from_secs(120))
        .expect("REQ-12：路 0 收尾应成功（不受中间路停止影响）");
    let fin_2 = handles[2]
        .wait_finalized(Duration::from_secs(120))
        .expect("REQ-12：路 2 收尾应成功（不受中间路停止影响）");

    // ── 独立输出文件：两两互异 ──
    let paths = [&fin_0.final_path, &fin_mid.final_path, &fin_2.final_path];
    for i in 0..3 {
        for j in i + 1..3 {
            assert_ne!(
                paths[i], paths[j],
                "REQ-12：每路独立输出文件（路 {i} 与路 {j} 成品路径相同）"
            );
        }
    }

    // ── 每路公共有效性 + 纯视频轨道形状 + 内容指向各自目标 ──
    let routes: Vec<(&Finalized, f64, &str, &PathBuf, usize, usize)> = vec![
        (&fin_0, span_side, titles[0].as_str(), &outs[0], 0, 24),
        (&fin_mid, span_mid, titles[1].as_str(), &outs[1], 1, 12),
        (&fin_2, span_side, titles[2].as_str(), &outs[2], 2, 24),
    ];
    for (i, (fin, span, title, out, marker, min_frames)) in routes.iter().enumerate() {
        assert_route_common(
            fin,
            *span,
            title,
            out,
            (1920, 1080),
            &format!("三路·路 {i}"),
        );
        assert_eq!(
            probe_stream_count(&fin.final_path, "a"),
            0,
            "三路均为纯视频路：不应有音频轨"
        );
        assert_frames_route_identified(
            &fin.final_path,
            *marker,
            *min_frames,
            &format!("三路·路 {i}"),
        );
    }

    // ── 中间路先停：两侧路时长明显更长（其余不受影响） ──
    let d_mid = probe_duration(&fin_mid.final_path);
    let d_0 = probe_duration(&fin_0.final_path);
    let d_2 = probe_duration(&fin_2.final_path);
    println!("[three] 时长：路0 {d_0:.3}s / 中间路 {d_mid:.3}s / 路2 {d_2:.3}s（span 侧 {span_side:.3}s / 中 {span_mid:.3}s）");
    assert!(
        d_0 > d_mid + 1.5 && d_2 > d_mid + 1.5,
        "REQ-12：停止中间路不得影响两侧路（两侧 {d_0:.3}/{d_2:.3}s vs 中间 {d_mid:.3}s，应各自≈录制段）"
    );

    drop(wins);
    for o in &outs {
        let _ = std::fs::remove_dir_all(o);
    }
}

/// REQ-12：单路故障不影响其它路——2 路录制中强杀**其中一个** ffmpeg sidecar
/// （taskkill /T /F，§3.5「sidecar 被终止」路径；Toolhelp32 只给父子关系、不给
/// 「哪个 sidecar 属于哪路」的映射，故断言对称化）：恰 1 路 Err(SidecarExit)
/// （保留分段、闭合段可播），另 1 路不受影响照常收尾成功。
#[test]
#[ignore = "L2：需交互式会话/WGC/可用编码器（specs §4）。显式运行：cargo test -p sr-itests --test multi_route -- --ignored --nocapture"]
fn it_multi_route_single_fault_isolated() {
    let _g = gate();
    require_interactive_session();
    let _os = record_os_build();

    let pid = std::process::id();
    let title_a = format!("SR-M5-RED-fault-A-{pid}");
    let title_b = format!("SR-M5-RED-fault-B-{pid}");
    let win_a = MarkerWindow::new(&title_a, 0, 620, 420);
    let win_b = MarkerWindow::new(&title_b, 1, 620, 420);
    std::thread::sleep(Duration::from_millis(500));

    let out_a = fresh_out_dir("fault-a");
    let out_b = fresh_out_dir("fault-b");
    let handles = start_routes_ok(vec![
        MultiRouteConfig {
            target: RouteTarget::Window {
                hwnd: win_a.hwnd_u64,
            },
            video: VideoCfg {
                canvas_w: 640,
                canvas_h: 360,
                fps: 30,
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
                canvas_w: 640,
                canvas_h: 360,
                fps: 30,
                cursor: true,
            },
            audio: vec![],
            output_dir: out_b.clone(),
        },
    ]);
    std::thread::sleep(Duration::from_millis(300));
    let t0 = Instant::now();
    std::thread::sleep(Duration::from_secs(6)); // > 5s 段长：每路至少 1 个已闭合段

    // 单路故障注入：强杀其中一个 sidecar（任选；断言对称化）
    let victims = ffmpeg_child_pids();
    assert!(
        victims.len() >= 2,
        "2 路运行中应能发现 ≥2 个 ffmpeg sidecar 子进程（得到 {}；每路一个 sidecar 契约违背）",
        victims.len()
    );
    println!("[fault] sidecar 候选 {victims:?}，强杀 {}", victims[0]);
    kill_tree(victims[0]);
    std::thread::sleep(Duration::from_secs(1)); // 死亡传播（M3 scratch：句柄回收 ≤5s）

    handles[0].request_stop();
    handles[1].request_stop();
    let span = t0.elapsed().as_secs_f64();
    let results: Vec<_> = handles
        .iter()
        .map(|h| h.wait_finalized(Duration::from_secs(90)))
        .collect();
    let errs: Vec<usize> = results
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.is_err().then_some(i))
        .collect();
    let oks: Vec<usize> = results
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.is_ok().then_some(i))
        .collect();
    assert_eq!(
        (errs.len(), oks.len()),
        (1, 1),
        "REQ-12：强杀一个 sidecar 应恰影响 1 路（得到 errs={errs:?} oks={oks:?}；结果 {results:?}）"
    );

    // ── 故障路：SidecarExit（强匹配）+ 保留分段可播 ──
    let fe = &results[errs[0]];
    match fe {
        Err(e) => assert!(
            matches!(e, RouteError::SidecarExit { .. }),
            "故障路应报 RouteError::SidecarExit（E_SIDECAR_EXIT；得到 {e:?}）"
        ),
        Ok(_) => unreachable!("errs 索引处必为 Err"),
    }
    let failed = &handles[errs[0]];
    let segs = failed.segments();
    assert!(
        !segs.is_empty(),
        "故障路应保留已写出段（6s > 5s 段长；完成段永不丢弃）"
    );
    for s in &segs {
        assert!(s.exists(), "段文件应仍在磁盘（{}）", s.display());
        assert!(
            s.metadata().map_or(false, |m| m.len() > 0),
            "0 字节壳不得列入段清单（{}）",
            s.display()
        );
    }
    let d = probe_duration_opt(&segs[0])
        .unwrap_or_else(|| panic!("故障路已闭合段应可独立读出时长（{}）", segs[0].display()));
    assert!(d > 0.0, "已闭合段时长应为正（{d}）");
    assert!(
        decode_clean(&segs[0]),
        "故障路已闭合段应独立可播（{}）",
        segs[0].display()
    );
    println!(
        "[fault] 故障路 = 路 {}（SidecarExit，{} 个段保留、首段 {d:.3}s 可播）",
        errs[0],
        segs.len()
    );

    // ── 幸存路：不受影响，正常收尾 ──
    let fin = match &results[oks[0]] {
        Ok(fin) => fin,
        Err(_) => unreachable!("oks 索引处必为 Ok"),
    };
    let (title, out, marker) = if oks[0] == 0 {
        (&title_a, &out_a, 0usize)
    } else {
        (&title_b, &out_b, 1usize)
    };
    assert_route_common(fin, span, title, out, (640, 360), "故障隔离·幸存路");
    assert_eq!(
        probe_stream_count(&fin.final_path, "a"),
        0,
        "纯视频路：不应有音频轨"
    );
    assert_frames_route_identified(&fin.final_path, marker, 15, "故障隔离·幸存路");

    drop(win_a);
    drop(win_b);
    let _ = std::fs::remove_dir_all(&out_a);
    let _ = std::fs::remove_dir_all(&out_b);
}

/// REQ-12/§3.3：批次启动「全成功或回滚」——[有效目标, 无效目标] 批量启动必须整体
/// 失败（Err(TargetInvalid)）；回滚完成后不得残留 sidecar 子进程、有效路不得产出
/// 成品（不得留下半启动状态）。
#[test]
#[ignore = "L2：需交互式会话/WGC/可用编码器（specs §4）。显式运行：cargo test -p sr-itests --test multi_route -- --ignored --nocapture"]
fn it_routes_batch_start_all_or_rollback() {
    let _g = gate();
    require_interactive_session();
    let _os = record_os_build();

    let pid = std::process::id();
    let title = format!("SR-M5-RED-rollback-{pid}");
    let win = MarkerWindow::new(&title, 0, 620, 420);
    std::thread::sleep(Duration::from_millis(500));

    let out_ok = fresh_out_dir("rb-ok");
    let out_bad = fresh_out_dir("rb-bad");
    // 显然非窗口句柄（CreateWindowExW 产物不会是该值）：该路启动必须失败
    let phantom_hwnd: u64 = 0xDEAD_BEEF;
    let r = start_routes(vec![
        MultiRouteConfig {
            target: RouteTarget::Window { hwnd: win.hwnd_u64 },
            video: VideoCfg {
                canvas_w: 640,
                canvas_h: 360,
                fps: 30,
                cursor: true,
            },
            audio: vec![],
            output_dir: out_ok.clone(),
        },
        MultiRouteConfig {
            target: RouteTarget::Window { hwnd: phantom_hwnd },
            video: VideoCfg {
                canvas_w: 640,
                canvas_h: 360,
                fps: 30,
                cursor: true,
            },
            audio: vec![],
            output_dir: out_bad.clone(),
        },
    ]);
    match r {
        Ok(hs) => panic!(
            "REQ-12/§3.3：批次含无效目标必须整体失败（全成功或回滚；得到 {} 个句柄）",
            hs.len()
        ),
        Err(e) if matches!(e, RouteError::EncoderUnavailable { .. }) => {
            blocked(&format!("无可用编码器（§9 T1 环境缺失）：{e:?}"))
        }
        Err(e) if matches!(e, RouteError::ConsentDenied { .. }) => {
            blocked(&format!("WGC 授权被拒（E_CONSENT_DENIED）：{e:?}"))
        }
        Err(e) if matches!(e, RouteError::LoopbackUnavailable { .. }) => blocked(&format!(
            "进程环回不可用（E_LOOPBACK_UNAVAILABLE，§3.5 环境缺失）：{e:?}"
        )),
        Err(e) => assert!(
            matches!(e, RouteError::TargetInvalid { .. }),
            "含无效窗口目标的批次启动应报 TargetInvalid（E_TARGET_LOST 语义；得到 {e:?}）"
        ),
    }

    // ── 回滚完成：无 sidecar 子进程残留（10s 轮询窗口） ──
    let clean = wait_until(Duration::from_secs(10), Duration::from_millis(500), || {
        ffmpeg_child_pids().is_empty().then_some(())
    });
    assert!(
        clean.is_some(),
        "REQ-12/§3.3：批次启动失败后应回滚——不得残留 ffmpeg sidecar 子进程（仍有 {:?}）",
        ffmpeg_child_pids()
    );

    // ── 回滚后有效路无成品（段文件按保留策略可留，成品=「已完成录制」不得存在） ──
    let token = sanitize_target_name(&title);
    let mut leftovers = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&out_ok) {
        for e in rd.filter_map(|e| e.ok()) {
            let is_final = e
                .path()
                .extension()
                .map_or(false, |x| x.to_string_lossy() == "mkv")
                && e.file_name().to_string_lossy().contains(&token);
            if is_final {
                leftovers.push(e.path());
            }
        }
    }
    assert!(
        leftovers.is_empty(),
        "REQ-12/§3.3：回滚后有效路的输出目录不得残留成品（{leftovers:?}）"
    );

    drop(win);
    let _ = std::fs::remove_dir_all(&out_ok);
    let _ = std::fs::remove_dir_all(&out_bad);
}
