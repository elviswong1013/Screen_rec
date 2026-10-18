//! L2 集成测试：音频隔离（REQ-14 核心 + REQ-17 隔离模式音轨 + REQ-26 背压阶跃校正
//! + REQ-18 管线格式端到端；specs/001-screen-recorder.md §2.4/§2.6、§3.1/§3.2/§3.3、
//! §4、§5 M4 行；docs/spikes/s3/s3-audio-isolation.md §2–§4 方法与阈值）。
//!
//! 运行方式（L2 门：`#[ignore]`，默认 `cargo test` 不执行本文件用例）：
//! ```text
//! cargo test -p sr-itests --test audio_isolation -- --ignored --nocapture
//! cargo test -p sr-itests --test audio_isolation it_audio_application_isolated_fft -- --ignored --nocapture   # 单用例
//! ```
//! 前置条件：交互式用户会话 + 可用默认渲染端点 + 仓库内锁定 ffmpeg n8.1 release 构建
//! （spec §8；含 ffplay/ffprobe）；缺失 → BLOCKED（fail），不静默 skip（§4）。
//! 本机环境（红测试编写时）：Windows 11 25H2，build 26200.9457，SESSIONNAME=Console。
//!
//! 手法（S3 移植，§2/§3）：ffmpeg 合成 440/880Hz 正弦 WAV（音量 0.25 防混音削波
//! 二次谐波落位 880）→ 两个独立 ffplay 进程同时播放 → 进程环回录制 → FFT
//! （rustfft 32768 点 Hann、hop 半帧、帧间平均谱）断言：目标音存在（高于本底
//! ≥30dB）、非目标 ≤ −40dB（相对目标；S3 预锁定阈值，实测隔离余量 −158dB）。
//! 双目标并发录制互为「整机混音确含两声」的对照（S3 场景 b/d 合并）。
//!
//! 接口契约（测试即契约，详见 docs/milestones/m4-red-report.md §3）：
//! - `sr_audio::wasapi::capture_process_loopback(pid: u32, created_at_unix_ms: u64,
//!   duration: Duration) -> Result<CapturedPcm, CaptureError>`
//!   语义：激活前身份前置校验（PID 存在 + 创建时间吻合，D9/S3 phantom 教训；失败
//!   须快速失败，不得录满时长）；系统宿主根（explorer/svchost/ApplicationFrameHost/
//!   RuntimeBroker/sihost）硬性禁止（REQ-14）；INCLUDE_TARGET_PROCESS_TREE（D9，
//!   S3 场景 e：父根含子进程）；专用 MTA 线程（S3 §4）；48k/f32/立体声 autoconvert
//!   （REQ-18）；时长按请求值**精确**补齐（S3 §3.3 补静音）。
//! - `CapturedPcm { samples: Vec<f32>（交错立体声）, sample_rate: u32(48000),
//!   channels: u16(2), stats: CaptureStats{padded_frames,dropped_frames,discontinuities} }`
//! - `CaptureError`（derive(Debug)；测试钉 5 变体，花括号模式容忍字段）：
//!   `TargetLost`（E_TARGET_LOST：PID 不存在）、`IdentityAmbiguous`
//!   （E_IDENTITY_AMBIGUOUS：创建时间不符/PID 复用）、`HostForbidden`
//!   （REQ-14 系统宿主根）、`LoopbackUnavailable`（E_LOOPBACK_UNAVAILABLE → 测试
//!   BLOCKED）、`Failed`。
//! - `sr_pipeline::route::AudioRouteConfig { target: RouteTarget, video: VideoCfg,
//!   audio: Vec<sr_audio::wasapi::AudioSourceSpec>, output_dir: PathBuf }`
//!   `AudioSourceSpec::App { pid, created_at_unix_ms } | System | Mic { device }`
//!   （§3.4；M4 测试仅构造 App——System/Mic 属 REQ-15/16 的 M6 用例）
//! - `sr_pipeline::route::start_audio_route(cfg) -> Result<RouteHandle, RouteError>`
//!   （M4 新增；既有 video-only `start_route`/`RouteConfig` 不动。Ok = 输出开始
//!   （§3.1）；隔离模式：每 App 源一条独立音轨、互不混音（REQ-17）；音源 PID
//!   消失 → 该轨静音继续、路由不失败（REQ-14/S3 §3.3）；RouteError 增补
//!   `LoopbackUnavailable { detail }`（§3.5）；句柄/收尾语义同 M3 RouteHandle）。
//!
//! 背压注入（REQ-26 用例）：Toolhelp32 找本测试进程的 ffmpeg sidecar 子进程 →
//! SuspendThread/ResumeThread 挂起全部线程 2s（< S1b 冻结的写超时下限 3s，不触发
//! E_PIPE_STALL 的「活但不读」路径）→ 管道写满阻塞（S1b 背压模型）→ 恢复后由
//! 阶跃校正把内容偏差一次性拉回（REQ-26：阈值起点 40ms）。

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rustfft::{num_complex::Complex, FftPlanner};

use common::*;
use sr_audio::wasapi::{capture_process_loopback, AudioSourceSpec, CaptureError, CapturedPcm};
use sr_pipeline::route::{
    start_audio_route, start_routes_with_options_and_rate, AudioRouteConfig, MultiRouteConfig,
    RouteError, RouteHandle, RouteTarget, VideoCfg,
};

/// L2 音频用例串行门：同一测试二进制内 6 个用例各自起 ffplay/路由，串行执行
/// （避免并发音源互相抬升引擎负载；隔离语义本身不受影响——进程环回按目标过滤）。
static GATE: Mutex<()> = Mutex::new(());

fn gate() -> std::sync::MutexGuard<'static, ()> {
    GATE.lock().unwrap_or_else(|e| e.into_inner())
}

// ─────────────────────────── 锁定构建 / 音调源（S3 §2 手法） ───────────────────────────

/// 锁定 ffmpeg/ffplay/ffprobe 位置：spec §8 M0 冻结的 BtbN **n8.1 release** 构建。
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

/// 进程树退出即清理的守卫（taskkill /T /F：连带 cmd 父进程的子 ffplay）。
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

/// 用锁定 ffmpeg 生成测试音 WAV（sine 48k 立体声 f32；音量 0.25——两声相加 0.5
/// 不削波，避免 440 的二次谐波（削波失真）落位 880 造成假泄漏）。
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

/// `\\?\` Verbatim 前缀（canonicalize 的产物）→ 普通 Win32 路径。
/// CreateProcess 直连可接受 Verbatim 路径（直接 spawn_ffplay 无碍），但
/// cmd.exe 自行解析时对 `\\?\` 前缀报「系统找不到指定的路径」——供 cmd
/// 命令行内嵌使用（fixprobe 实测：外层引号 + Verbatim 前缀 ⇒ 子进程不出现；
/// 外层引号 + 剥前缀 ⇒ 子进程出现）。
fn plain_win32(p: &Path) -> String {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        s.strip_prefix(r"\\?\")
            .map(str::to_string)
            .unwrap_or_else(|| s.to_string())
    }
}

/// 父 cmd.exe + 子 ffplay（进程树包含语义的载体，S3 场景 e）。
/// cmd /c 阻塞等待子进程 ⇒ 树根存活。两处 cmd 陷阱（m4-green §5-2 + fixprobe 实测）：
/// ①引号规则：cmd /C 对「以引号开头且多于两个引号」的串做首尾引号剥离——
///   `"EXE" args "WAV"` 被剥成 `EXE" args "WAV` 立即失败；整条命令外层再包
///   一对引号，剥离后恰还原内层命令行。
/// ②路径域：ffbin() 的 canonicalize 产生 `\\?\` Verbatim 前缀，cmd 无法执行，
///   须经 plain_win32 剥前缀（wav 路径无前缀，原样）。
fn spawn_cmd_ffplay(wav: &Path) -> KillTreeOnDrop {
    use std::os::windows::process::CommandExt;
    let inner = format!(
        "\"{}\" -nodisp -autoexit -loglevel quiet \"{}\"",
        plain_win32(&ffbin("ffplay.exe")),
        wav.display()
    );
    let child = Command::new("cmd.exe")
        .raw_arg("/c")
        .raw_arg(format!("\"{inner}\""))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn cmd/ffplay 进程树：{e}")));
    KillTreeOnDrop(child)
}

// ─────────────────────────── 进程发现 / 线程挂起（背压注入） ───────────────────────────

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

struct ProcEntry {
    pid: u32,
    ppid: u32,
    name_lower: String,
}

/// Toolhelp32 进程快照（单次遍历，含父子关系）。
fn snapshot_processes() -> Vec<ProcEntry> {
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
                out.push(ProcEntry {
                    pid: pe.th32ProcessID,
                    ppid: pe.th32ParentProcessID,
                    name_lower: wide_to_string(&pe.szExeFile).to_lowercase(),
                });
                if Process32NextW(snap, &mut pe).is_err() {
                    break;
                }
            }
        }
        let _ = windows::Win32::Foundation::CloseHandle(snap);
        out
    }
}

fn children_of(pid: u32) -> Vec<u32> {
    snapshot_processes()
        .into_iter()
        .filter(|e| e.pid != 0 && e.ppid == pid)
        .map(|e| e.pid)
        .collect()
}

fn proc_pid_by_name(name_lower: &str) -> Option<u32> {
    snapshot_processes()
        .into_iter()
        .find(|e| e.name_lower == name_lower)
        .map(|e| e.pid)
}

/// 本测试进程的 ffmpeg sidecar 子进程（Toolhelp32 父子关系 + 映像名，M3 同款）。
fn ffmpeg_child_pids() -> Vec<u32> {
    let me = std::process::id();
    snapshot_processes()
        .into_iter()
        .filter(|e| e.pid != 0 && e.ppid == me && e.name_lower.contains("ffmpeg"))
        .map(|e| e.pid)
        .collect()
}

/// 挂起进程全部线程（背压注入：sidecar「活但不读」），返回已挂起的 tid 供恢复。
fn suspend_process_threads(pid: u32) -> Vec<u32> {
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows::Win32::System::Threading::{OpenThread, SuspendThread, THREAD_SUSPEND_RESUME};
    unsafe {
        let snap =
            CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0).expect("CreateToolhelp32Snapshot(线程)");
        let mut tids = Vec::new();
        let mut te = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        if Thread32First(snap, &mut te).is_ok() {
            loop {
                if te.th32OwnerProcessID == pid {
                    if let Ok(h) = OpenThread(THREAD_SUSPEND_RESUME, false, te.th32ThreadID) {
                        let _ = SuspendThread(h);
                        let _ = windows::Win32::Foundation::CloseHandle(h);
                        tids.push(te.th32ThreadID);
                    }
                }
                if Thread32Next(snap, &mut te).is_err() {
                    break;
                }
            }
        }
        let _ = windows::Win32::Foundation::CloseHandle(snap);
        tids
    }
}

fn resume_process_threads(tids: &[u32]) {
    use windows::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};
    for &tid in tids {
        unsafe {
            if let Ok(h) = OpenThread(THREAD_SUSPEND_RESUME, false, tid) {
                let _ = ResumeThread(h);
                let _ = windows::Win32::Foundation::CloseHandle(h);
            }
        }
    }
}

// ─────────────────────────── FFT 分析（S3 §2 方法移植，rustfft） ───────────────────────────

/// 32768 点 @48k：0.6827s 窗、bin=1.4648Hz（S3 同参数）；帧间平均功率谱。
const NFFT: usize = 32768;
/// 跳过首 1.0s（ffplay 起播/激活过渡；S3 跳 0.25s，此处按 L2 起播时序更保守）。
const SKIP_HEAD_S: f64 = 1.0;
const SKIP_TAIL_S: f64 = 0.5;

struct Spectrum {
    bin_hz: f64,
    power: Vec<f64>,
}

fn avg_spectrum(x: &[f32], sample_rate: u32) -> Spectrum {
    use std::f64::consts::PI;
    assert!(x.len() > NFFT * 2, "可分析样本过短（{} < 2×NFFT）", x.len());
    let hop = NFFT / 2;
    let head = (SKIP_HEAD_S * sample_rate as f64) as usize;
    let tail = x
        .len()
        .saturating_sub((SKIP_TAIL_S * sample_rate as f64) as usize);
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
        for k in 0..NFFT {
            acc[k] += buf[k].norm_sqr();
        }
        frames += 1;
        start += hop;
    }
    assert!(frames >= 2, "可分析帧数不足（{frames}）");
    for a in &mut acc {
        *a /= frames as f64;
    }
    Spectrum {
        bin_hz: sample_rate as f64 / NFFT as f64,
        power: acc,
    }
}

impl Spectrum {
    /// 频带 [hz−bw, hz+bw] 内最大 bin 的相对 dB（跨段比较用差值；S3：目标频带 ±5Hz 取最大 bin）。
    fn band_peak_rel_db(&self, hz: f64, half_bw: f64) -> f64 {
        let k0 = (((hz - half_bw) / self.bin_hz).floor() as usize).max(1);
        let k1 = (((hz + half_bw) / self.bin_hz).ceil() as usize).min(self.power.len() / 2);
        let p = (k0..=k1).fold(0.0f64, |m, k| m.max(self.power[k]));
        10.0 * p.log10()
    }

    /// 噪声底：[lo,hi] 去除排除带后的中位数功率（S3 §2 同手法）。
    fn band_median_rel_db(&self, lo: f64, hi: f64, exclude: &[(f64, f64)]) -> f64 {
        let mut ps: Vec<f64> = (1..self.power.len() / 2)
            .map(|k| (k, k as f64 * self.bin_hz))
            .filter(|(_, f)| {
                *f >= lo && *f <= hi && !exclude.iter().any(|&(a, b)| *f >= a && *f <= b)
            })
            .map(|(k, _)| self.power[k])
            .collect();
        assert!(ps.len() >= 8, "噪声底样本过少（{}）", ps.len());
        ps.sort_by(|a, b| a.partial_cmp(b).expect("NaN 功率"));
        ps[ps.len() / 2]
    }
}

/// S3 预锁定判定（§2）：目标音存在（高于本底 ≥30dB）且非目标泄漏 ≤ −40dB（相对目标）。
fn assert_tone_isolated(x: &[f32], sample_rate: u32, target_hz: f64, other_hz: f64, label: &str) {
    let s = avg_spectrum(x, sample_rate);
    let t = s.band_peak_rel_db(target_hz, 5.0);
    let o = s.band_peak_rel_db(other_hz, 5.0);
    let floor = s.band_median_rel_db(
        100.0,
        6000.0,
        &[
            (target_hz - 10.0, target_hz + 10.0),
            (other_hz - 10.0, other_hz + 10.0),
        ],
    );
    println!("[fft:{label}] target({target_hz}Hz)={t:.1}dB other({other_hz}Hz)={o:.1}dB floor={floor:.1}dB ⇒ 泄漏 {:.1}dB", o - t);
    assert!(
        t - floor >= 30.0,
        "REQ-14：{label} 目标 {target_hz}Hz 应存在（高于本底 ≥30dB，差 {:.1}dB）",
        t - floor
    );
    assert!(
        o - t <= -40.0,
        "REQ-14：{label} 非目标 {other_hz}Hz 泄漏应 ≤ −40dB（相对目标 {:.1}dB；S3 预锁定阈值）",
        o - t
    );
}

/// 音调存在性（无对照源场景）：目标高于本底 ≥20dB。
fn assert_tone_present(x: &[f32], sample_rate: u32, target_hz: f64, label: &str) {
    let s = avg_spectrum(x, sample_rate);
    let t = s.band_peak_rel_db(target_hz, 5.0);
    let floor = s.band_median_rel_db(100.0, 6000.0, &[(target_hz - 10.0, target_hz + 10.0)]);
    println!(
        "[fft:{label}] target({target_hz}Hz)={t:.1}dB floor={floor:.1}dB ⇒ 差 {:.1}dB",
        t - floor
    );
    assert!(
        t - floor >= 20.0,
        "REQ-26：{label} {target_hz}Hz 应存在（高于本底 ≥20dB，差 {:.1}dB）",
        t - floor
    );
}

// ─────────────────────────── 被测 API 的 BLOCKED 包装 ───────────────────────────

fn capture_ok(pid: u32, created_at_unix_ms: u64, duration: Duration, label: &str) -> CapturedPcm {
    match capture_process_loopback(pid, created_at_unix_ms, duration) {
        Ok(c) => c,
        Err(e) if matches!(e, CaptureError::LoopbackUnavailable { .. }) => blocked(&format!(
            "{label}：进程环回不可用（E_LOOPBACK_UNAVAILABLE，§3.5 环境缺失）：{e:?}"
        )),
        Err(e) => panic!("{label}：进程环回捕获应成功（得到 {e:?}）"),
    }
}

fn start_audio_route_ok(cfg: AudioRouteConfig) -> RouteHandle {
    match start_audio_route(cfg) {
        Ok(h) => h,
        Err(e) if matches!(e, RouteError::EncoderUnavailable { .. }) => blocked(&format!(
            "无可用编码器（§9 T1：明确报错并列出可用编码器）：{e:?}"
        )),
        Err(e) if matches!(e, RouteError::ConsentDenied { .. }) => {
            blocked(&format!("WGC 授权被拒（E_CONSENT_DENIED）：{e:?}"))
        }
        Err(e) if matches!(e, RouteError::LoopbackUnavailable { .. }) => blocked(&format!(
            "进程环回不可用（E_LOOPBACK_UNAVAILABLE，§3.5 环境缺失）：{e:?}"
        )),
        Err(e) => panic!("start_audio_route 应成功（得到 {e:?}）"),
    }
}

/// M15b/REQ-5：指定输出率的音频路由（批量启动 API 的单路形态；错误映射同上）。
fn start_audio_route_at_rate_ok(cfg: MultiRouteConfig, out_rate: u32) -> RouteHandle {
    match start_routes_with_options_and_rate(vec![cfg], None, false, out_rate) {
        Ok(mut handles) => handles.remove(0),
        Err(e) if matches!(e, RouteError::EncoderUnavailable { .. }) => blocked(&format!(
            "无可用编码器（§9 T1：明确报错并列出可用编码器）：{e:?}"
        )),
        Err(e) if matches!(e, RouteError::ConsentDenied { .. }) => {
            blocked(&format!("WGC 授权被拒（E_CONSENT_DENIED）：{e:?}"))
        }
        Err(e) if matches!(e, RouteError::LoopbackUnavailable { .. }) => blocked(&format!(
            "进程环回不可用（E_LOOPBACK_UNAVAILABLE，§3.5 环境缺失）：{e:?}"
        )),
        Err(e) => panic!("start_routes_with_options_and_rate 应成功（得到 {e:?}）"),
    }
}

// ─────────────────────────── ffprobe / 音轨解码 ───────────────────────────

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

/// 首音轨时长（秒；无流时长时回退容器时长）。
fn probe_audio_duration(path: &Path) -> f64 {
    let s = ffprobe_csv(
        path,
        &[
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=duration",
            "-of",
            "csv=p=0",
        ],
    );
    s.parse::<f64>()
        .ok()
        .filter(|d| *d > 0.0)
        .unwrap_or_else(|| probe_duration(path))
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
    // 形如 "aac,48000,2"
    let parts: Vec<&str> = s.split(',').collect();
    assert_eq!(parts.len(), 3, "音轨 csv 形状（得到 {s:?}）");
    (
        parts[0].trim().to_string(),
        parts[1].trim().parse().expect("sample_rate"),
        parts[2].trim().parse().expect("channels"),
    )
}

/// 首视频流 (codec, width, height)。
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
    let parts: Vec<&str> = s.split(',').collect();
    assert_eq!(parts.len(), 3, "视频流 csv 形状（得到 {s:?}）");
    (
        parts[0].trim().to_string(),
        parts[1].trim().parse().expect("width"),
        parts[2].trim().parse().expect("height"),
    )
}

/// 解码首音轨为 48k 单声道 f32（FFT 分析输入）。
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

fn fresh_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr_m4_red_audio_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("创建临时目录失败");
    d
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ─────────────────────────── 用例 ───────────────────────────

/// REQ-14（核心）：按应用音频隔离——A(440Hz)+B(880Hz) 同时经默认输出播放，
/// 进程环回各自只含自己的音调（非目标 ≤ −40dB，S3 预锁定；实测余量 −158dB）。
/// 双目标**并发**录制（两条采集线程/两个 loopback 客户端，S3 场景 d），
/// 录 B 含 880 即为「整机混音确含两声」的对照（S3 场景 b 自对照合并）。
/// REQ-18（端到端）：48k/f32/立体声；时长按请求值精确补齐（静音补齐钉时间轴）。
#[test]
#[ignore = "L2：需交互式会话/默认渲染端点/锁定 n8.1 ffplay（specs §4）。显式运行：cargo test -p sr-itests --test audio_isolation -- --ignored --nocapture"]
fn it_audio_application_isolated_fft() {
    let _g = gate();
    require_interactive_session();
    let _os = record_os_build();

    let tmp = fresh_dir("iso");
    let wav_a = gen_tone_wav(&tmp, "a440.wav", 440);
    let wav_b = gen_tone_wav(&tmp, "b880.wav", 880);
    let player_a = spawn_ffplay(&wav_a);
    let player_b = spawn_ffplay(&wav_b);
    std::thread::sleep(Duration::from_millis(1500)); // 起播

    let pid_a = player_a.0.id();
    let pid_b = player_b.0.id();
    let created_a = process_created_unix_ms(pid_a)
        .unwrap_or_else(|| blocked("无法读取 ffplay A 创建时间（身份地面真值）"));
    let created_b = process_created_unix_ms(pid_b)
        .unwrap_or_else(|| blocked("无法读取 ffplay B 创建时间（身份地面真值）"));

    const DUR: Duration = Duration::from_secs(6);
    let (cap_a, cap_b) = std::thread::scope(|s| {
        let ha = s.spawn(move || capture_ok(pid_a, created_a, DUR, "录 A"));
        let hb = s.spawn(move || capture_ok(pid_b, created_b, DUR, "录 B"));
        (
            ha.join().expect("录 A 线程 panic"),
            hb.join().expect("录 B 线程 panic"),
        )
    });

    for (cap, target_hz, other_hz, label) in [
        (&cap_a, 440.0f64, 880.0f64, "录 A"),
        (&cap_b, 880.0, 440.0, "录 B"),
    ] {
        assert_eq!(cap.sample_rate, 48000, "REQ-18：{label} 管线格式应为 48kHz");
        assert_eq!(cap.channels, 2, "REQ-18：{label} 管线格式应为立体声");
        let frames = cap.samples.len() / 2;
        let expect = 6 * 48_000usize;
        assert!(
            (frames as i64 - expect as i64).abs() <= 48,
            "REQ-18：{label} 静音补齐应把时长钉到请求值（帧数 {frames} vs 期望 {expect}，容差 48 帧/1ms）"
        );
        let left: Vec<f32> = cap.samples.chunks(2).map(|c| c[0]).collect();
        assert_tone_isolated(&left, cap.sample_rate, target_hz, other_hz, label);
        println!(
            "[iso:{label}] stats：padded={} dropped={} discontinuities={}",
            cap.stats.padded_frames, cap.stats.dropped_frames, cap.stats.discontinuities
        );
    }

    let _ = std::fs::remove_dir_all(&tmp);
    // players 由 KillTreeOnDrop 清理
}

/// REQ-14（有界身份）：默认捕获根 = 目标 PID 及其后代树（INCLUDE，D9/S3 场景 e）；
/// 系统宿主根硬性禁止（防宿主泄漏=变相录系统混音）；PID 存在性与创建时间在
/// **激活前**校验（S3 extra：phantom PID 激活不报错、静默零包——不能靠激活校验）。
#[test]
#[ignore = "L2：需交互式会话/默认渲染端点/锁定 n8.1 ffplay（specs §4）。显式运行：cargo test -p sr-itests --test audio_isolation -- --ignored --nocapture"]
fn it_audio_identity_bounded_no_host_leak() {
    let _g = gate();
    require_interactive_session();
    let _os = record_os_build();

    let tmp = fresh_dir("ident");
    let wav_a = gen_tone_wav(&tmp, "a440.wav", 440);
    let wav_b = gen_tone_wav(&tmp, "b880.wav", 880);

    // 独立干扰源 B（不属于 cmd 树；_ 前缀绑定：Drop 守卫仍于作用域结束执行）
    let _player_b = spawn_ffplay(&wav_b);
    // 父 cmd + 子 ffplay A：树包含语义载体（S3 场景 e）
    let cmd_guard = spawn_cmd_ffplay(&wav_a);
    let cmd_pid = cmd_guard.0.id();
    let child_a = wait_until(Duration::from_secs(10), Duration::from_millis(300), || {
        children_of(cmd_pid).into_iter().next()
    })
    .unwrap_or_else(|| panic!("cmd 的子 ffplay 未在 10s 内出现（cmd 树构造失败）"));
    println!(
        "[ident] cmd={cmd_pid} → 子 ffplay={child_a}；独立 B={}",
        _player_b.0.id()
    );

    let created_cmd = process_created_unix_ms(cmd_pid)
        .unwrap_or_else(|| blocked("无法读取 cmd 创建时间（身份地面真值）"));
    let created_child = process_created_unix_ms(child_a)
        .unwrap_or_else(|| blocked("无法读取子 ffplay 创建时间（身份地面真值）"));
    std::thread::sleep(Duration::from_millis(1500)); // 起播

    // ── 1. 后代树包含（D9 默认根 = PID + INCLUDE tree）：
    //      以 cmd 为根捕获 → 须录到子 ffplay 的 440；树外 B 的 880 ≤ −40dB ──
    let cap = capture_ok(cmd_pid, created_cmd, Duration::from_secs(5), "cmd 树");
    assert_eq!(cap.sample_rate, 48000, "REQ-18：48kHz");
    assert_eq!(cap.channels, 2, "REQ-18：立体声");
    let left: Vec<f32> = cap.samples.chunks(2).map(|c| c[0]).collect();
    assert_tone_isolated(&left, cap.sample_rate, 440.0, 880.0, "cmd 树");

    // ── 2. 系统宿主根：硬性禁止（REQ-14；explorer.exe 常驻交互会话）──
    let explorer = proc_pid_by_name("explorer.exe")
        .unwrap_or_else(|| blocked("找不到 explorer.exe（宿主拒绝用例前置失败）"));
    let created_exp = process_created_unix_ms(explorer)
        .unwrap_or_else(|| blocked("无法读取 explorer.exe 创建时间"));
    let t = Instant::now();
    match capture_process_loopback(explorer, created_exp, Duration::from_secs(3)) {
        Err(e) if matches!(e, CaptureError::HostForbidden { .. }) => {
            println!("[ident] explorer.exe 根被拒绝：{e:?}")
        }
        Err(other) => panic!("REQ-14：系统宿主根应报 HostForbidden（得到 {other:?}）"),
        Ok(_) => panic!("REQ-14：系统宿主根不得开始捕获（宿主泄漏 = 变相录系统混音，禁止）"),
    }
    assert!(
        t.elapsed() <= Duration::from_secs(2),
        "REQ-14：宿主拒绝须在激活前的校验阶段快速失败（耗时 {:?}；不得录满 3s）",
        t.elapsed()
    );

    // ── 3. phantom PID：激活不校验 PID（S3 extra 实测）→ 须自行校验存在性 ──
    //      999_999 为奇数 ⇒ 不可能是合法 Windows PID（PID 为 4 的倍数）
    let t = Instant::now();
    match capture_process_loopback(999_999, now_unix_ms(), Duration::from_secs(3)) {
        Err(e) if matches!(e, CaptureError::TargetLost { .. }) => {
            println!("[ident] phantom 拒绝：{e:?}")
        }
        Err(other) => panic!("REQ-14/D9：phantom PID 应报 TargetLost（得到 {other:?}）"),
        Ok(_) => panic!("REQ-14/D9：phantom PID 不得成功捕获（S3：激活不校验 PID，静默零包）"),
    }
    assert!(
        t.elapsed() <= Duration::from_secs(2),
        "phantom 校验须快速失败（耗时 {:?}；S3 教训：不得静默录满零包）",
        t.elapsed()
    );

    // ── 4. PID 复用防护（D9：PID + 创建时间）：真实 PID + 偏差创建时间 → 拒绝 ──
    let wrong_created = created_child.saturating_add(300_000); // ±5 分钟偏差
    let t = Instant::now();
    match capture_process_loopback(child_a, wrong_created, Duration::from_secs(3)) {
        Err(e) if matches!(e, CaptureError::IdentityAmbiguous { .. }) => {
            println!("[ident] 创建时间不符拒绝：{e:?}")
        }
        Err(other) => panic!("D9：创建时间不符应报 IdentityAmbiguous（得到 {other:?}）"),
        Ok(_) => panic!("D9：PID 复用防护——创建时间不符的 PID 不得按原身份捕获"),
    }
    assert!(
        t.elapsed() <= Duration::from_secs(2),
        "身份不符校验须快速失败（耗时 {:?}）",
        t.elapsed()
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// REQ-17（隔离模式的音轨输出部分，M4；全音轨模式属 M6）：窗口路由 + 目标应用音轨
/// → 成品 MKV 恰 1 视频 + 1 音轨（隔离默认；无麦克风 ⇒ 无 mic 轨）、aac/输出率/立体声
/// （§3.3 基准；M15b/REQ-5 起 48k 基线 + 44.1k/96k 变体），音轨内容级隔离
/// （440 在、880 ≤ −40dB）。时长≈录制段。
fn multitrack_isolation_case(output_rate: u32) {
    let _g = gate();
    require_interactive_session();
    let _os = record_os_build();

    let tmp = fresh_dir(&format!("route_{output_rate}"));
    let out_dir = tmp.join("out");
    std::fs::create_dir_all(&out_dir).expect("创建输出目录失败");
    let wav_a = gen_tone_wav(&tmp, "a440.wav", 440);
    let wav_b = gen_tone_wav(&tmp, "b880.wav", 880);
    let player_a = spawn_ffplay(&wav_a);
    let _player_b = spawn_ffplay(&wav_b); // 干扰源守卫（880Hz，不属于目标应用）
    std::thread::sleep(Duration::from_millis(1500)); // 起播

    let pid_a = player_a.0.id();
    let created_a = process_created_unix_ms(pid_a)
        .unwrap_or_else(|| blocked("无法读取 ffplay A 创建时间（身份地面真值）"));

    let probe = ProbeWindow::new(&format!("SR-M4-RED-audio-{}", std::process::id()));
    std::thread::sleep(Duration::from_millis(300));

    let h = start_audio_route_at_rate_ok(
        MultiRouteConfig {
            target: RouteTarget::Window {
                hwnd: probe.hwnd_u64,
            },
            video: VideoCfg {
                canvas_w: 640,
                canvas_h: 360,
                fps: 30,
                cursor: true,
            },
            audio: vec![AudioSourceSpec::App {
                pid: pid_a,
                created_at_unix_ms: created_a,
            }],
            output_dir: out_dir.clone(),
        },
        output_rate,
    );
    std::thread::sleep(Duration::from_millis(300));
    let t_rec = Instant::now();
    std::thread::sleep(Duration::from_secs(8));
    h.request_stop();
    let span = t_rec.elapsed().as_secs_f64();
    let fin = h
        .wait_finalized(Duration::from_secs(120))
        .expect("REQ-17：隔离模式路由停止后收尾应成功（EOF 优雅停止 + 后台 concat）");

    assert!(
        fin.final_path.exists(),
        "成品应存在（{}）",
        fin.final_path.display()
    );
    assert!(
        fin.final_path
            .extension()
            .map_or(false, |e| e.to_string_lossy() == "mkv"),
        "成品应为 MKV（{}）",
        fin.final_path.display()
    );

    // ── 轨道形状（REQ-17 隔离模式）：1 视频 + 恰 1 目标应用音轨 ──
    let (vcodec, w, hgt) = probe_video_stream(&fin.final_path);
    assert_eq!(vcodec, "h264", "T6：v1 仅 H.264");
    assert_eq!((w, hgt), (640, 360), "D6：固定画布 640x360");
    assert_eq!(
        probe_stream_count(&fin.final_path, "v"),
        1,
        "应恰 1 条视频轨"
    );
    assert_eq!(
        probe_stream_count(&fin.final_path, "a"),
        1,
        "REQ-17：隔离模式应恰 1 条目标应用音轨（未启用麦克风 ⇒ 无 mic 轨；M4 无追加应用）"
    );
    let (acodec, ar, ch) = probe_audio_stream(&fin.final_path);
    assert_eq!(acodec, "aac", "§3.3 基准：音频编码 aac");
    assert_eq!(
        ar, output_rate,
        "REQ-5：音轨实际采样率应为所选输出率（得到 {ar}）"
    );
    assert_eq!(ch, 2, "REQ-18：音轨立体声");

    let dur = probe_duration(&fin.final_path);
    println!(
        "[route-rate-{output_rate}] 时长 {dur:.3}s（录制段 {span:.3}s）；音轨 aac,{output_rate},2"
    );
    assert!(
        (dur - span).abs() <= 2.0,
        "REQ-17：成品时长应≈录制段（{dur:.3}s vs {span:.3}s）"
    );

    // ── 音轨内容级隔离（S3 判定）──
    let mono = decode_audio_mono(&fin.final_path);
    assert!(
        mono.len() > 48_000 * 4,
        "音轨样本应 ≥4s（得到 {} 帧；音轨缺失/过短？）",
        mono.len()
    );
    assert_tone_isolated(&mono, 48_000, 440.0, 880.0, "app 轨");

    let _ = std::fs::remove_dir_all(&tmp);
}

/// 48k 基线（REQ-17/18 既有断言不变）。
#[test]
#[ignore = "L2：需交互式会话/WGC/可用编码器/默认渲染端点（specs §4）。显式运行：cargo test -p sr-itests --test audio_isolation -- --ignored --nocapture"]
fn it_audio_multitrack_ffprobe_fft() {
    multitrack_isolation_case(48_000);
}

/// M15b/REQ-5：44.1k 输出率的隔离子集（隔离判定与 48k 基线一致）。
#[test]
#[ignore = "L2/M15b：44.1k 输出率隔离子集（交互式会话/WGC/编码器/默认渲染端点）"]
fn it_audio_multitrack_ffprobe_fft_44100() {
    multitrack_isolation_case(44_100);
}

/// M15b/REQ-5：96k 输出率的隔离子集。
#[test]
#[ignore = "L2/M15b：96k 输出率隔离子集（交互式会话/WGC/编码器/默认渲染端点）"]
fn it_audio_multitrack_ffprobe_fft_96000() {
    multitrack_isolation_case(96_000);
}

/// REQ-26：背压/停滞恢复后的阶跃校正——录制中挂起 sidecar 全部线程 2s
/// （< S1b 写超时下限 3s：不触发 E_PIPE_STALL「活但不读」，走纯背压路径）→
/// 管道写满、写线程阻塞（S1b 背压模型）→ 恢复后积压排空，内容偏差（已写样本
/// vs QPC）≥40ms ⇒ 须一次性丢/补把音轨时间轴拉回墙钟（无校正 ⇒ 时长亏空 ≈2s）。
/// 内容完整性：停滞前后目标音均在（恢复无错位/爆音由整文件解码干净间接覆盖）。
#[test]
#[ignore = "L2：需交互式会话/WGC/可用编码器/默认渲染端点（specs §4）。显式运行：cargo test -p sr-itests --test audio_isolation -- --ignored --nocapture"]
fn it_backpressure_step_correction() {
    let _g = gate();
    require_interactive_session();
    let _os = record_os_build();

    let tmp = fresh_dir("bp");
    let out_dir = tmp.join("out");
    std::fs::create_dir_all(&out_dir).expect("创建输出目录失败");
    let wav_a = gen_tone_wav(&tmp, "a440.wav", 440);
    let player_a = spawn_ffplay(&wav_a);
    std::thread::sleep(Duration::from_millis(1500)); // 起播

    let pid_a = player_a.0.id();
    let created_a = process_created_unix_ms(pid_a)
        .unwrap_or_else(|| blocked("无法读取 ffplay A 创建时间（身份地面真值）"));

    let probe = ProbeWindow::new(&format!("SR-M4-RED-bp-{}", std::process::id()));
    std::thread::sleep(Duration::from_millis(300));

    let h = start_audio_route_ok(AudioRouteConfig {
        target: RouteTarget::Window {
            hwnd: probe.hwnd_u64,
        },
        video: VideoCfg {
            canvas_w: 640,
            canvas_h: 360,
            fps: 30,
            cursor: true,
        },
        audio: vec![AudioSourceSpec::App {
            pid: pid_a,
            created_at_unix_ms: created_a,
        }],
        output_dir: out_dir.clone(),
    });
    std::thread::sleep(Duration::from_millis(300));
    let t_rec = Instant::now();

    // 正常段 3s
    std::thread::sleep(Duration::from_secs(3));

    // 背压注入：挂起 sidecar 全部线程 2s（活但不读，且 < 写超时下限 3s）
    let victims = ffmpeg_child_pids();
    assert!(
        !victims.is_empty(),
        "应能发现 ffmpeg sidecar 子进程（Toolhelp32 父子关系；sidecar 未 spawn？契约违背）"
    );
    let mut suspended = Vec::new();
    for &v in &victims {
        suspended.extend(suspend_process_threads(v));
    }
    assert!(
        !suspended.is_empty(),
        "应挂起至少 1 条 sidecar 线程（背压注入失败）"
    );
    println!(
        "[bp] 挂起 sidecar {:?} 共 {} 线程，停滞 2s",
        victims,
        suspended.len()
    );
    std::thread::sleep(Duration::from_secs(2));
    resume_process_threads(&suspended);

    // 恢复后继续录满 ~12s（总跨度含停滞）
    while t_rec.elapsed() < Duration::from_secs(12) {
        std::thread::sleep(Duration::from_millis(100));
    }
    h.request_stop();
    let span = t_rec.elapsed().as_secs_f64();
    let fin = h
        .wait_finalized(Duration::from_secs(150))
        .expect("REQ-26：2s 背压停滞属恢复窗口内（§2.6 有界失败之前），收尾应成功");

    let (acodec, ar, ch) = probe_audio_stream(&fin.final_path);
    assert_eq!(acodec, "aac", "§3.3 基准：音频编码 aac");
    assert_eq!(ar, 48000, "REQ-18：音轨 48kHz");
    assert_eq!(ch, 2, "REQ-18：音轨立体声");

    // 阶跃校正判定：音轨时长 ≈ 墙钟段（无校正 ⇒ 亏空 ≈ 停滞 2s ≫ 容差）
    let adur = probe_audio_duration(&fin.final_path);
    println!("[bp] 音轨 {adur:.3}s vs 墙钟 {span:.3}s（停滞 2s）");
    assert!(
        (adur - span).abs() <= 0.75,
        "REQ-26：背压停滞 2s 后应经一次性丢/补把内容偏差拉回（音轨 {adur:.3}s vs 墙钟 {span:.3}s，容差 0.75s；无校正亏空 ≈2s）"
    );

    // 内容完整性：停滞前（前 3s）与恢复后（末 3s）目标音均在。
    // 切片须 ≥2.8s（m4-green §5-3）：avg_spectrum 固定跳首 1.0s + 尾 0.5s、
    // NFFT 窗 0.6827s、hop 0.3413s ⇒ 3s 切片余 1.5s 可分析窗 = 3 帧（≥2）；
    // 2s 切片余 0.5s < NFFT，恒 0 帧（数学不可满足）。停滞始于 t≈3s 且其首
    // ~0.7s 为管道缓冲排空段（内容仍连续），头切片分析窗 [1.0,2.5]s 全部
    // 位于停滞前的有效音区——断言意图（背压前后音轨连续）不变。
    let mono = decode_audio_mono(&fin.final_path);
    let edge = 3 * 48_000usize;
    assert!(
        mono.len() > edge * 2,
        "音轨应 ≥6s（得到 {} 帧）",
        mono.len()
    );
    assert_tone_present(&mono[..edge], 48_000, 440.0, "bp-head");
    assert_tone_present(&mono[mono.len() - edge..], 48_000, 440.0, "bp-tail");
    assert!(
        decode_clean(&fin.final_path),
        "整文件应可解码（恢复无爆音/损坏）"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
