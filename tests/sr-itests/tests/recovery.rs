//! L2 集成测试：崩溃/恢复状态机 + 磁盘满注入（REQ-22 的 M8 范围，specs/001-screen-recorder.md
//! §2.5 REQ-22（完整）、§3.3（recover_sessions/finalize_session/discard_session 语义）、
//! §3.4（SessionManifest v1）、§3.5（崩溃残留/磁盘不足行）、§4（REQ-22 两用例）、
//! §5 M8 行、§8 S5（崩溃/恢复实测与规则来源）。
//!
//! 运行方式（L2 门：`#[ignore]`，默认 `cargo test` 不执行本文件用例）：
//! ```text
//! cargo test -p sr-itests --test recovery -- --ignored --nocapture
//! cargo test -p sr-itests --test recovery it_crash_kill_matrix_recovery_state_machine -- --ignored --nocapture
//! cargo test -p sr-itests --test recovery it_fault_injection_disk_full -- --ignored --nocapture
//! ```
//! 前置条件：交互式用户会话 + 可用编码器（D4 链）+ 锁定 ffmpeg n8.1（§8）；缺失 →
//! BLOCKED（fail），不静默 skip（§4）。
//!
//! 接口契约（测试即契约，详见 docs/milestones/m8-red-report.md §3.2/§3.3）：
//! - 会话目录 = `<output_dir>/.screenrec_tmp/<session_id>/`（REQ-24/D12 默认；录制
//!   开始时创建），内含 `manifest.json`（`recovery::MANIFEST_FILE`）与段文件。
//! - `save_manifest_atomic(dir, &SessionManifest)`：原子更新（临时+rename；成功后不残留
//!   临时文件）；`load_manifest(dir)`：往返一致。
//! - `scan_sessions(output_dir) -> Vec<RecoverableSession>`：目录级重扫核对
//!   （「段完成但清单未更新」不丢）；排除 0 字节/坏 EBML/活动文件；会话活跃 → `active=true`。
//! - `finalize_session(dir) -> Result<RecoveryOutcome, RecoveryError>`：幂等（二次调用
//!   无操作）；成品（REQ-24 命名）位于 output_dir 根；M14c：成功且成品校验通过后按
//!   `keep_segments` 策略清理/保留原段（`finalize_session` 默认清理；
//!   `finalize_session_with_options(dir, true)` 保留）；失败/清单损坏/活跃拒绝路径
//!   保留原段（`RecoveryFailed`/`Active`）；WAV 恢复的成品在会话目录内，不触发清理。
//! - `discard_session(dir)`：删除会话目录；不存在 → `NotFound`。
//! - 磁盘写失败（含 ENOSPC 类）→ `RouteError::DiskFull`（E_DISK_FULL，§3.4/§3.5：安全停止、
//!   保留原段、收尾可重试）。
//!
//! 手法：①合成崩溃残留矩阵（锁定 ffmpeg 造段；manifest 由 API 写；覆盖陈旧清单/0 字节/
//! 坏 EBML/截断尾段/清单损坏/缺失/短段/WAV 残留）；②**真实子进程强杀矩阵**（本测试二进制
//! 再入录制 Monitor 0；杀主进程验证 Job 对象连带回收 sidecar、活动文件排除、收尾）；③磁盘满
//! 注入 = 录制中途 `icacls /deny ...(OI)(CI)W` 写拒绝（S5 §9a 同类写失败路径；机制见报告 §6）。

mod common;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use common::*;
use sr_audio::wav::repair_wav;
use sr_pipeline::naming::sanitize_target_name;
use sr_pipeline::recovery::{
    discard_session, finalize_session, finalize_session_with_options, load_manifest,
    save_manifest_atomic, scan_sessions, ManifestRoute, ManifestSegment, RecoverableSession,
    RecoveryError, RecoveryOutcome, RouteIdentity, SessionManifest,
};
use sr_pipeline::route::{start_route, RouteConfig, RouteError, RouteTarget, VideoCfg};

/// L2 串行门（M4–M7 先例）：两用例各自起真实路由/强杀子进程，串行执行。
static GATE: Mutex<()> = Mutex::new(());

fn gate() -> std::sync::MutexGuard<'static, ()> {
    GATE.lock().unwrap_or_else(|e| e.into_inner())
}

// ─────────────────────────── 阈值/常量 ───────────────────────────

/// 已封口段时长容差的宽松下界（合成段 1.5–2.0s；容器头尾抖动 <0.1s）。
const SEG_DUR_TOL: f64 = 0.6;
/// 崩溃会话收尾的时长下界容差（14s 录制 ≥2×5s 封口段；首段起录抖动余量）。
const FINAL_DUR_TOL: f64 = 1.5;

// ─────────────────────────── 锁定构建 / 目录 ───────────────────────────

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

fn fresh_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr_m8_red_recovery_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("创建临时目录失败");
    d
}

fn run_ffmpeg(args: &[&str], what: &str) {
    let out = Command::new(ffbin("ffmpeg.exe"))
        .args(["-hide_banner", "-nostdin"])
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn 锁定 ffmpeg（{what}）：{e}")));
    if !out.status.success() {
        blocked(&format!(
            "{what} 失败（exit={}；stderr={}）——锁定构建能力缺失",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
}

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

/// 首音轨形状（`codec,sample_rate,channels`；M15b/REQ-5 恢复率域断言）。
fn probe_audio_shape(path: &Path) -> String {
    ffprobe_csv(
        path,
        &[
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=codec_name,sample_rate,channels",
            "-of",
            "csv=p=0",
        ],
    )
}

/// 恢复成品解码：**容忍告警**（截断尾段回收会带 "File ended prematurely"），要求 exit 0。
fn decode_exit0(path: &Path) -> bool {
    let out = Command::new(ffbin("ffmpeg.exe"))
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn 锁定 ffmpeg 解码：{e}")));
    if !out.status.success() {
        println!(
            "[decode] {} 失败：{}",
            path.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    } else if !out.stderr.is_empty() {
        println!(
            "[decode] {} 告警（容忍）：{}",
            path.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    out.status.success()
}

fn count_mkv(dir: &Path) -> usize {
    fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    e.path()
                        .extension()
                        .map_or(false, |x| x.eq_ignore_ascii_case("mkv"))
                })
                .count()
        })
        .unwrap_or(0)
}

fn as_set(paths: &[PathBuf]) -> HashSet<PathBuf> {
    paths.iter().cloned().collect()
}

// ─────────────────────────── 合成素材（锁定构建） ───────────────────────────

/// 合成一个「1 视频 + 1 音轨」的小段（mpeg4/aac MKV；48k 基线）。
fn mk_seg(path: &Path, secs: f64, freq: u32) {
    mk_seg_with_rate(path, secs, freq, 48_000);
}

/// M15b/REQ-5：合成指定输出率的小段（恢复成品应保留该音轨采样率）。
fn mk_seg_with_rate(path: &Path, secs: f64, freq: u32, rate: u32) {
    let vf = format!("testsrc2=size=320x180:rate=15:duration={secs}");
    let af = format!("sine=frequency={freq}:sample_rate={rate}:duration={secs}");
    run_ffmpeg(
        &[
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &vf,
            "-f",
            "lavfi",
            "-i",
            &af,
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-c:v",
            "mpeg4",
            "-q:v",
            "5",
            "-ac",
            "2",
            "-c:a",
            "aac",
            "-b:a",
            "96k",
            "-f",
            "matroska",
            path.to_str().expect("utf-8 路径"),
        ],
        "合成测试段",
    );
}

/// 截断文件（模拟崩溃尾段：muxer 未写 trailer/被割断）。
fn truncate_to(path: &Path, ratio: f64) {
    let bytes = fs::read(path).expect("读待截断文件");
    let keep = ((bytes.len() as f64) * ratio) as usize;
    fs::write(path, &bytes[..keep.min(bytes.len())]).expect("写截断文件");
}

/// 写「未封口 WAV」崩溃残留（44 字节头部占位 + s16 数据；RIFF/data 长度字段为 0）。
fn write_unsealed_wav(path: &Path, secs: f64) {
    let ch = 2u16;
    let rate = 48000u32;
    let bits = 16u16;
    let block = (ch * (bits / 8)) as u32;
    let byte_rate = rate * block;
    let data_len = ((secs * rate as f64) as u32) * block;
    let mut bytes: Vec<u8> = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&0u32.to_le_bytes()); // 占位（崩溃残留）
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&ch.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&(block as u16).to_le_bytes());
    bytes.extend_from_slice(&bits.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&0u32.to_le_bytes()); // 占位
    for i in 0..(data_len as usize / 4) {
        let v = (((i % 109) as i32 - 54) * 200) as i16;
        bytes.extend_from_slice(&v.to_le_bytes());
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    fs::write(path, bytes).expect("写未封口 WAV");
}

/// 合成素材自检（防测试自身漂移）：段可解码且时长≈名义。
fn assert_seg_ok(path: &Path, nominal: f64) {
    let d = probe_duration(path);
    assert!(
        (d - nominal).abs() <= SEG_DUR_TOL,
        "合成段 {} 时长应≈{nominal}（得到 {d}）",
        path.display()
    );
    assert!(decode_exit0(path), "合成段应可解码（{}）", path.display());
}

// ─────────────────────────── manifest 构造 ───────────────────────────

fn identity_for(name: &str) -> RouteIdentity {
    // 合成会话用「已死 PID + created=0」身份：确定性非活跃（活跃判定由相位 R 的真实
    // 子进程覆盖）；避免把测试自身进程误判为会话持有者。
    RouteIdentity {
        pid: dead_pid(),
        created_at_unix_ms: 0,
        name: name.to_string(),
        path: std::env::current_exe().expect("current_exe"),
    }
}

/// 生成一个「已死进程」的 PID（spawn `cmd /c exit` 并收割；测试期内复用概率可忽略）。
fn dead_pid() -> u32 {
    let mut c = Command::new("cmd")
        .args(["/c", "exit", "0"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn cmd /c exit");
    let pid = c.id();
    let _ = c.wait();
    pid
}

fn seg_entry(path: &Path, dur: f64, closed: bool) -> ManifestSegment {
    ManifestSegment {
        path: path.to_path_buf(),
        t_start: 0.0,
        t_end: dur,
        closed,
    }
}

fn manifest_with(session_id: &str, routes: Vec<ManifestRoute>) -> SessionManifest {
    SessionManifest {
        version: 1,
        session_id: session_id.to_string(),
        created_at_unix_ms: self_created_unix_ms(),
        start_qpc: 123_456_789,
        routes,
    }
}

fn route_with(
    id: &str,
    name: &str,
    streams: &[&str],
    segments: Vec<ManifestSegment>,
) -> ManifestRoute {
    ManifestRoute {
        route_id: id.to_string(),
        identity: identity_for(name),
        streams: streams.iter().map(|s| s.to_string()).collect(),
        segments,
        state: "finalizing".to_string(),
        final_path: None,
    }
}

fn find_session<'a>(sessions: &'a [RecoverableSession], dir: &Path) -> &'a RecoverableSession {
    sessions.iter().find(|s| s.dir == dir).unwrap_or_else(|| {
        panic!(
            "scan_sessions 应发现会话 {}（得到 {:?}）",
            dir.display(),
            sessions.iter().map(|s| s.dir.clone()).collect::<Vec<_>>()
        )
    })
}

// ─────────────────────────── 进程工具（Toolhelp32；M3/M6 同款） ───────────────────────────

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

/// 指定父进程的 ffmpeg 子进程（Job 对象回收验证用）。
fn ffmpeg_children_of(parent_pid: u32) -> Vec<u32> {
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
                if pe.th32ParentProcessID == parent_pid && name.contains("ffmpeg") {
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

fn kill_pid(pid: u32) {
    let st = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("spawn taskkill");
    assert!(st.success(), "taskkill /PID {pid} /F 失败：{st}");
}

/// 整树强杀（taskkill /T /F；编码进程被杀注入用）。
fn kill_tree(pid: u32) {
    let st = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("spawn taskkill");
    assert!(st.success(), "taskkill /PID {pid} /T /F 失败：{st}");
}

/// 子进程守卫：任何路径退出都整树强杀（含 sidecar），防测试残留。
struct ChildGuard {
    child: Child,
    pid: u32,
}
impl ChildGuard {
    fn pid(&self) -> u32 {
        self.pid
    }
    fn kill_main_and_reap(&mut self) {
        kill_pid(self.pid);
        let _ = self.child.wait();
    }
    /// 等待子进程自行退出（≤timeout）；true=已退出。
    fn wait_exit(&mut self, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Ok(Some(_)) = self.child.try_wait() {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = Command::new("taskkill")
            .args(["/PID", &self.pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = self.child.wait();
    }
}

// ─────────────────────────── ACL 写拒绝注入（磁盘满/写失败；S5 §9a 同类路径） ───────────────────────────

struct AclDeny {
    path: PathBuf,
    user: String,
    active: bool,
}
impl AclDeny {
    fn apply(path: &Path, user: &str) -> AclDeny {
        let st = Command::new("icacls")
            .arg(path)
            .args(["/deny", &format!("{user}:(OI)(CI)W")])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .status()
            .unwrap_or_else(|e| blocked(&format!("无法 spawn icacls（写拒绝注入）：{e}")));
        if !st.success() {
            blocked(&format!(
                "icacls /deny 失败（{}；磁盘满注入机制不可用）",
                path.display()
            ));
        }
        AclDeny {
            path: path.to_path_buf(),
            user: user.to_string(),
            active: true,
        }
    }
    fn restore(&mut self) {
        if self.active {
            let _ = Command::new("icacls")
                .arg(&self.path)
                .args(["/remove:d", &self.user])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            self.active = false;
        }
    }
}
impl Drop for AclDeny {
    fn drop(&mut self) {
        self.restore();
    }
}

// ─────────────────────────── 录制子进程（本测试二进制再入；M6 先例） ───────────────────────────

const CHILD_ENV: &str = "SR_M8_CHILD";
const CHILD_OUTPUT_ENV: &str = "SR_M8_CHILD_OUTPUT";
const CHILD_MARKER_ENV: &str = "SR_M8_CHILD_MARKER";
const TEST_NAME: &str = "it_crash_kill_matrix_recovery_state_machine";

/// 子进程分支：录制 Monitor 0（无窗口依赖），写就绪标记后阻塞在 wait_finalized——
/// 主进程强杀覆盖「主进程死」，编码进程被杀覆盖「E_SIDECAR_EXIT → 结果文件 → 退出」。
fn child_record_main(output_dir: &Path, marker: &Path) -> ! {
    let cfg = RouteConfig {
        target: RouteTarget::Monitor { index: 0 },
        video: VideoCfg {
            canvas_w: 640,
            canvas_h: 360,
            fps: 30,
            cursor: false,
        },
        output_dir: output_dir.to_path_buf(),
    };
    match start_route(cfg) {
        Ok(handle) => {
            let _ = fs::write(
                marker,
                format!("ok {} {}", std::process::id(), self_created_unix_ms()),
            );
            println!(
                "[child] 录制已启动（Monitor 0，640x360@30，output={}）；进入 wait_finalized…",
                output_dir.display()
            );
            let res = handle.wait_finalized(Duration::from_secs(600));
            let result_path = PathBuf::from(format!("{}.result", marker.display()));
            match &res {
                Ok(fin) => {
                    let _ = fs::write(&result_path, format!("ok {}", fin.final_path.display()));
                    println!("[child] finalized：{}", fin.final_path.display());
                }
                Err(e) => {
                    let _ = fs::write(&result_path, format!("err {e:?}"));
                    println!("[child] 失败返回：{e:?}");
                }
            }
            std::process::exit(0);
        }
        Err(e) => {
            let _ = fs::write(marker, format!("err {e:?}"));
            eprintln!("[child] 启动失败：{e:?}");
            std::process::exit(2);
        }
    }
}

fn spawn_child_record(output_dir: &Path, marker: &Path, log_dir: &Path) -> ChildGuard {
    let exe = std::env::current_exe().expect("current_exe");
    let out_log = fs::File::create(log_dir.join("child_stdout.log")).expect("child stdout log");
    let err_log = fs::File::create(log_dir.join("child_stderr.log")).expect("child stderr log");
    let child = Command::new(exe)
        .args(["--exact", TEST_NAME, "--ignored", "--nocapture"])
        .env(CHILD_ENV, "record")
        .env(CHILD_OUTPUT_ENV, output_dir)
        .env(CHILD_MARKER_ENV, marker)
        .stdout(Stdio::from(out_log))
        .stderr(Stdio::from(err_log))
        .spawn()
        .expect("spawn 录制子进程");
    let pid = child.id();
    println!(
        "[recovery] 录制子进程 pid={pid}（output={}）",
        output_dir.display()
    );
    ChildGuard { child, pid }
}

fn wait_child_ready(marker: &Path, timeout: Duration) -> String {
    let content = wait_until(timeout, Duration::from_millis(200), || {
        fs::read_to_string(marker)
            .ok()
            .filter(|s| !s.trim().is_empty())
    })
    .unwrap_or_else(|| panic!("录制子进程就绪标记超时（{}）", marker.display()));
    let content = content.trim().to_string();
    if let Some(err) = content.strip_prefix("err ") {
        if err.contains("EncoderUnavailable") || err.contains("ConsentDenied") {
            blocked(&format!("录制子进程环境缺失（§9 T1/WGC）：{err}"));
        }
        panic!("录制子进程启动失败：{err}");
    }
    assert!(
        content.starts_with("ok "),
        "就绪标记形状应为 `ok pid created`（得到 {content:?}）"
    );
    content
}

// ─────────────────────────── 用例 1：崩溃/恢复状态机 ───────────────────────────

#[test]
#[ignore = "L2：需交互式会话 + 可用编码器 + 锁定 ffmpeg n8.1（specs §4）。显式运行：cargo test -p sr-itests --test recovery -- --ignored --nocapture"]
fn it_crash_kill_matrix_recovery_state_machine() {
    // 子进程分支（本测试二进制再入；Parent 见下）
    if let Ok(mode) = std::env::var(CHILD_ENV) {
        if mode == "record" {
            let _g = gate();
            let output =
                PathBuf::from(std::env::var(CHILD_OUTPUT_ENV).expect("SR_M8_CHILD_OUTPUT"));
            let marker =
                PathBuf::from(std::env::var(CHILD_MARKER_ENV).expect("SR_M8_CHILD_MARKER"));
            child_record_main(&output, &marker);
        }
    }

    let _g = gate();
    require_interactive_session();
    let _os = record_os_build();

    // ────────────────── 相位 S：合成崩溃残留矩阵（确定性；锁定 ffmpeg 造段） ──────────────────
    let root = fresh_dir("synth");
    let tmp_root = root.join(".screenrec_tmp");

    // S1：manifest 原子写/往返 + 目录内无临时文件残留
    let s_rt = tmp_root.join("sess-rt");
    fs::create_dir_all(&s_rt).expect("创建会话目录");
    let seg_rt = s_rt.join("out0_00000.mkv");
    mk_seg(&seg_rt, 2.0, 440);
    let m = manifest_with(
        "sess-rt",
        vec![route_with(
            "route-rt",
            "RoundTrip Target",
            &["v", "a"],
            vec![seg_entry(&seg_rt, 2.0, true)],
        )],
    );
    save_manifest_atomic(&s_rt, &m).expect("manifest 原子写应成功");
    let mut names: Vec<String> = fs::read_dir(&s_rt)
        .expect("读会话目录")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    let mut expected_names = vec!["manifest.json".to_string(), "out0_00000.mkv".to_string()];
    expected_names.sort();
    assert_eq!(
        names, expected_names,
        "原子写成功后会话目录应恰含 manifest.json 与既有段（无临时文件残留；得到 {names:?}）"
    );
    let loaded = load_manifest(&s_rt).expect("manifest 往返应可读");
    assert_eq!(loaded.version, 1, "schema v1");
    assert_eq!(loaded.session_id, "sess-rt");
    assert_eq!(loaded.start_qpc, 123_456_789);
    assert_eq!(loaded.routes.len(), 1);
    assert_eq!(loaded.routes[0].route_id, "route-rt");
    assert_eq!(
        loaded.routes[0].streams,
        vec!["v".to_string(), "a".to_string()]
    );
    assert_eq!(loaded.routes[0].segments.len(), 1);
    assert!(loaded.routes[0].segments[0].closed, "closed 标志应往返");
    assert_eq!(loaded.routes[0].segments[0].path, seg_rt);
    assert_eq!(loaded.routes[0].identity.name, "RoundTrip Target");
    let mut m2 = loaded.clone();
    m2.start_qpc = 999;
    save_manifest_atomic(&s_rt, &m2).expect("覆盖写应成功");
    assert_eq!(
        load_manifest(&s_rt).expect("重读").start_qpc,
        999,
        "覆盖写应生效（原子替换）"
    );
    println!("[recovery] S1 manifest 原子写/往返 OK");

    // S2：陈旧清单 + 目录重扫 + 截断尾段回收 + 幂等收尾 + 原段保留
    // M14c：本相位核对原段/幂等重试，显式 keep_segments=true（默认成功后清理会话目录）；
    // 默认清理由 L1 + 本套件 S3（合成会话）覆盖。
    let s_a = tmp_root.join("sess-A");
    fs::create_dir_all(&s_a).expect("创建会话 A");
    let a0 = s_a.join("out0_00000.mkv");
    let a1 = s_a.join("out0_00001.mkv");
    let a2 = s_a.join("out0_00002.mkv");
    mk_seg(&a0, 2.0, 440);
    mk_seg(&a1, 1.5, 660);
    mk_seg(&a2, 3.0, 880);
    truncate_to(&a2, 0.6); // 崩溃尾段（未封口形态）
    assert_seg_ok(&a0, 2.0);
    assert_seg_ok(&a1, 1.5);
    // 陈旧清单：只登记 a0（「段完成但清单未更新」→ 恢复必须目录重扫）
    let m_a = manifest_with(
        "sess-A",
        vec![route_with(
            "route-a",
            "Recovery Target A",
            &["v", "a"],
            vec![seg_entry(&a0, 2.0, true)],
        )],
    );
    save_manifest_atomic(&s_a, &m_a).expect("写会话 A 清单");
    let sizes_a: Vec<(PathBuf, u64)> = [&a0, &a1, &a2]
        .iter()
        .map(|p| ((*p).clone(), fs::metadata(p).expect("段元数据").len()))
        .collect();

    let sessions = scan_sessions(&root);
    let s = find_session(&sessions, &s_a);
    assert!(s.manifest.is_some(), "清单可读的会话应携带 manifest");
    assert!(!s.active, "无活跃进程的残留会话应为非活跃");
    assert_eq!(
        as_set(&s.sealed_segments),
        as_set(&[a0.clone(), a1.clone()]),
        "目录重扫应补出 a1（后者存在高序号文件 → 封口）"
    );
    assert_eq!(
        s.tail_segment.as_deref(),
        Some(a2.as_path()),
        "最高序号的可解析段应识别为崩溃尾段"
    );

    let fin: RecoveryOutcome = finalize_session_with_options(&s_a, true)
        .expect("崩溃残留应可收尾（重扫核对；M14c keep=true 以核对原段）");
    assert_eq!(fin.route_finals.len(), 1, "恰 1 条路由成品");
    let (route_id, final_path) = &fin.route_finals[0];
    assert_eq!(route_id, "route-a");
    assert_eq!(
        final_path.parent(),
        Some(root.as_path()),
        "成品应位于 output_dir 根（非会话临时目录）"
    );
    assert!(
        final_path
            .extension()
            .map_or(false, |e| e.eq_ignore_ascii_case("mkv")),
        "成品应为 MKV"
    );
    let stem = final_path
        .file_stem()
        .expect("stem")
        .to_string_lossy()
        .to_string();
    assert!(
        stem.contains(&sanitize_target_name("Recovery Target A")),
        "成品名应含 sanitize(目标名)（REQ-24；得到 {stem:?}）"
    );
    assert!(decode_exit0(final_path), "恢复成品应整文件可解码");
    let dur = probe_duration(final_path);
    println!(
        "[recovery] S2 成品 {}：时长 {dur:.3}s（封口段和 3.5s；清单条目 {}/{}/{})",
        final_path.display(),
        a0.display(),
        a1.display(),
        a2.display()
    );
    assert!(
        dur >= 3.0 && dur <= 6.5,
        "成品时长应含全部封口段（3.5s）并容忍尾段回收（≤6.5s；得到 {dur:.3}s）"
    );
    let used = as_set(&fin.segments_used);
    assert!(
        used.contains(&a0) && used.contains(&a1),
        "封口段必须参与收尾（segments_used={:?}）",
        fin.segments_used
    );
    assert!(
        !(used.contains(&a2) && fin.discarded.contains(&a2)),
        "尾段不得同时列入 used 与 discarded"
    );
    assert!(
        fin.discarded.iter().all(|p| p == &a2),
        "除尾段外不得丢弃任何段（discarded={:?}）",
        fin.discarded
    );
    for (p, sz) in &sizes_a {
        assert!(p.exists(), "收尾不得删除原段（{}）", p.display());
        if p != &a2 {
            assert_eq!(
                fs::metadata(p).expect("元数据").len(),
                *sz,
                "封口段不得被改写"
            );
        }
    }

    // 幂等：二次收尾无操作（同一成品、字节不变、根目录 mkv 数不变）
    let bytes1 = fs::read(final_path).expect("读成品");
    let mkv_count1 = count_mkv(&root);
    let fin2 = finalize_session_with_options(&s_a, true).expect("收尾应幂等（二次调用成功）");
    assert_eq!(
        fin2.route_finals[0].1, *final_path,
        "幂等收尾应复用既有成品路径"
    );
    assert_eq!(
        fs::read(final_path).expect("重读成品"),
        bytes1,
        "幂等收尾不得改写成品"
    );
    assert_eq!(count_mkv(&root), mkv_count1, "幂等收尾不得产生第二个成品");
    for (p, _) in &sizes_a {
        assert!(p.exists(), "幂等收尾后原段仍须保留");
    }
    println!("[recovery] S2 幂等收尾 OK（root mkv 数 {mkv_count1}）");

    // S3：末段 0 字节 / 末段坏 EBML → 均排除在封口集合外，无尾段可回收
    let s_b = tmp_root.join("sess-B");
    fs::create_dir_all(&s_b).expect("创建会话 B");
    let b0 = s_b.join("out0_00000.mkv");
    mk_seg(&b0, 2.0, 440);
    fs::write(s_b.join("out0_00001.mkv"), b"").expect("写 0 字节段");
    save_manifest_atomic(
        &s_b,
        &manifest_with(
            "sess-B",
            vec![route_with(
                "route-b",
                "Recovery Target B",
                &["v", "a"],
                vec![
                    seg_entry(&b0, 2.0, true),
                    seg_entry(&s_b.join("out0_00001.mkv"), 0.0, false),
                ],
            )],
        ),
    )
    .expect("写会话 B 清单");
    let sessions = scan_sessions(&root);
    let s = find_session(&sessions, &s_b);
    assert_eq!(
        as_set(&s.sealed_segments),
        as_set(&[b0.clone()]),
        "0 字节段不得进入封口集合"
    );
    assert_eq!(s.tail_segment, None, "0 字节末段不构成可回收尾段");
    let fin_b = finalize_session(&s_b).expect("含 0 字节段的会话应可收尾（排除坏段）");
    let dur_b = probe_duration(&fin_b.route_finals[0].1);
    assert!(
        (dur_b - 2.0).abs() <= SEG_DUR_TOL,
        "成品时长应≈封口段和（排除 0 字节段；得到 {dur_b:.3}s）"
    );
    assert!(
        !s_b.exists(),
        "M14c 默认：恢复收尾成功后应清理会话目录（{}）",
        s_b.display()
    );

    let s_c = tmp_root.join("sess-C");
    fs::create_dir_all(&s_c).expect("创建会话 C");
    let c0 = s_c.join("out0_00000.mkv");
    mk_seg(&c0, 2.0, 440);
    fs::write(
        s_c.join("out0_00001.mkv"),
        b"NOT-A-MATROSKA-SEGMENT (deterministic garbage)\n",
    )
    .expect("写坏段");
    save_manifest_atomic(
        &s_c,
        &manifest_with(
            "sess-C",
            vec![route_with(
                "route-c",
                "Recovery Target C",
                &["v", "a"],
                vec![
                    seg_entry(&c0, 2.0, true),
                    seg_entry(&s_c.join("out0_00001.mkv"), 0.0, false),
                ],
            )],
        ),
    )
    .expect("写会话 C 清单");
    let sessions = scan_sessions(&root);
    let s = find_session(&sessions, &s_c);
    assert_eq!(
        as_set(&s.sealed_segments),
        as_set(&[c0.clone()]),
        "坏 EBML 段不得进入封口集合"
    );
    assert_eq!(s.tail_segment, None, "坏 EBML 末段不构成可回收尾段");
    finalize_session(&s_c).expect("含坏段的会话应可收尾（排除坏段）");
    println!("[recovery] S3 坏段排除 OK（0 字节/Bad EBML）");

    // S4：清单损坏 / 清单缺失 → 扫描不静默（manifest=None），收尾拒绝且保留原段；丢弃可用
    let s_d1 = tmp_root.join("sess-D1");
    fs::create_dir_all(&s_d1).expect("创建会话 D1");
    let d1_0 = s_d1.join("out0_00000.mkv");
    let d1_1 = s_d1.join("out0_00001.mkv");
    mk_seg(&d1_0, 2.0, 440);
    mk_seg(&d1_1, 2.0, 660);
    fs::write(
        s_d1.join("manifest.json"),
        b"{{{ this is not valid manifest json",
    )
    .expect("写坏清单");
    let s_d2 = tmp_root.join("sess-D2");
    fs::create_dir_all(&s_d2).expect("创建会话 D2");
    mk_seg(&s_d2.join("out0_00000.mkv"), 2.0, 440);
    mk_seg(&s_d2.join("out0_00001.mkv"), 2.0, 660);

    for (dir, label) in [(&s_d1, "清单损坏"), (&s_d2, "清单缺失")] {
        let sessions = scan_sessions(&root);
        let s = find_session(&sessions, dir);
        assert!(
            s.manifest.is_none(),
            "{label}：扫描应保留会话（manifest=None，不静默丢弃）"
        );
        assert!(
            matches!(
                load_manifest(dir),
                Err(RecoveryError::RecoveryFailed { .. })
            ),
            "{label}：load_manifest 应报 RecoveryFailed"
        );
        let err = finalize_session(dir)
            .expect_err(&format!("{label}：无清单不得自动收尾（要求人工/丢弃路径）"));
        assert!(
            matches!(err, RecoveryError::RecoveryFailed { .. }),
            "{label}：应报 RecoveryFailed（得到 {err:?}）"
        );
        let before = count_mkv(dir);
        assert!(before >= 1, "{label}：原段保留");
        discard_session(dir).expect("坏清单会话应可显式丢弃");
        assert!(!dir.exists(), "{label}：discard 后会话目录应删除");
    }
    assert!(
        matches!(
            discard_session(&tmp_root.join("sess-nonexistent")),
            Err(RecoveryError::NotFound { .. })
        ),
        "丢弃不存在的会话应报 NotFound"
    );
    assert!(s_a.exists(), "丢弃其它会话不得影响 sess-A（会话隔离）");
    println!("[recovery] S4 清单损坏/缺失 + 丢弃 OK");

    // S5：WAV 崩溃残留并入恢复清单（REQ-22 尾部条款）：就地封口、幂等
    let s_w = tmp_root.join("sess-W");
    fs::create_dir_all(&s_w).expect("创建会话 W");
    let wav = s_w.join("crash.wav");
    write_unsealed_wav(&wav, 2.0);
    let unsealed_len = fs::metadata(&wav).expect("wav 元数据").len();
    save_manifest_atomic(
        &s_w,
        &manifest_with(
            "sess-W",
            vec![route_with(
                "route-w",
                "Recovery Wav",
                &["wav"],
                vec![seg_entry(&wav, 2.0, false)],
            )],
        ),
    )
    .expect("写会话 W 清单");
    let fin_w = finalize_session(&s_w).expect("WAV 残留会话应可收尾（修复封口）");
    assert!(
        fin_w.repaired_wavs.contains(&wav),
        "repaired_wavs 应含崩溃 WAV（得到 {:?}）",
        fin_w.repaired_wavs
    );
    let bytes = fs::read(&wav).expect("读修复后 wav");
    assert!(
        bytes.len() >= 44 && bytes.len() as u64 >= unsealed_len,
        "修复不得截掉数据"
    );
    let riff = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as u64;
    let data = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]) as u64;
    assert_eq!(
        riff,
        bytes.len() as u64 - 8,
        "RIFF 长度应封口（文件长度-8）"
    );
    assert_eq!(
        data,
        bytes.len() as u64 - 44,
        "data 长度应封口（文件长度-44）"
    );
    let again = repair_wav(&wav).expect("修复后应仍可读（幂等）");
    assert_eq!(again.trimmed_bytes, 0, "封口后 repair_wav 应无操作（幂等）");
    assert!(
        again.frames > 48_000,
        "修复后帧数应>1s（得到 {}）",
        again.frames
    );
    println!("[recovery] S5 WAV 残留修复 OK（frames={}）", again.frames);

    // ────────────────── 相位 R：真实子进程强杀矩阵（主进程死 → Job 回收 sidecar） ──────────────────
    let child_out = fresh_dir("child-out");
    let logs = fresh_dir("child-logs");
    let marker = logs.join("child.marker");
    let mut guard = spawn_child_record(&child_out, &marker, &logs);
    let ready = wait_child_ready(&marker, Duration::from_secs(90));
    println!("[recovery] 子进程就绪：{ready}");
    let t_ready = std::time::Instant::now();

    // 录制中（子进程存活）：会话应标记 active；收尾必须拒绝（恢复扫描不得动活动文件）
    std::thread::sleep(Duration::from_secs(3));
    let sessions = scan_sessions(&child_out);
    assert!(
        !sessions.is_empty(),
        "录制中的会话应可被扫描（至少发现会话目录）"
    );
    let active_dir = sessions[0].dir.clone();
    assert!(
        sessions[0].active,
        "子进程仍在录制：会话应标记 active（identity pid 存活）"
    );
    let refused = finalize_session(&active_dir);
    assert!(
        matches!(refused, Err(RecoveryError::Active { .. })),
        "活跃会话收尾必须拒绝（得到 {refused:?}）"
    );
    println!("[recovery] 活跃会话拒绝收尾 OK（{}）", active_dir.display());

    // 录满 ≥14s（≥2 个 5s 封口段 + 尾段）
    while t_ready.elapsed() < Duration::from_secs(14) {
        std::thread::sleep(Duration::from_millis(200));
    }

    // 杀主进程（不带 /T）：sidecar 应被 Job 对象连带终止（D8/REQ-22 sidecar 生命周期）
    let child_pid = guard.pid();
    guard.kill_main_and_reap();
    let reaped = wait_until(Duration::from_secs(15), Duration::from_millis(250), || {
        ffmpeg_children_of(child_pid).is_empty().then_some(())
    });
    assert!(
        reaped.is_some(),
        "主进程死亡后其 ffmpeg sidecar 应被终止（Job 对象绑定主进程，D8）；仍残留 {:?}",
        ffmpeg_children_of(child_pid)
    );
    println!("[recovery] Job 对象回收 sidecar OK（pid={child_pid} 无残留 ffmpeg）");

    // 崩溃后扫描/收尾：非活跃、封口段可救、成品可播、原段保留、幂等、丢弃
    let sessions = scan_sessions(&child_out);
    let s = find_session(&sessions, &active_dir);
    assert!(!s.active, "主进程与 sidecar 均已死亡：会话应转为非活跃");
    assert!(
        !s.sealed_segments.is_empty(),
        "14s 录制应至少留下 1 个封口段（得到 {}）",
        s.sealed_segments.len()
    );
    let sealed_before: Vec<PathBuf> = s.sealed_segments.clone();
    // M14c：崩溃收尾核对原段保留 + 幂等重试，显式 keep=true（默认清理另由 S3/L1 覆盖）。
    let fin = finalize_session_with_options(&s.dir, true).expect("崩溃会话应可收尾");
    assert_eq!(fin.route_finals.len(), 1, "单路会话应恰 1 个成品");
    let (rid, cfinal) = &fin.route_finals[0];
    assert!(!rid.is_empty(), "成品 route_id 不得为空");
    assert_eq!(
        cfinal.parent(),
        Some(child_out.as_path()),
        "成品应位于 output_dir 根"
    );
    assert!(decode_exit0(cfinal), "崩溃恢复成品应可解码");
    let cdur = probe_duration(cfinal);
    let sealed_floor = s.sealed_segments.len() as f64 * 5.0 - FINAL_DUR_TOL;
    println!(
        "[recovery] 崩溃恢复成品 {}：时长 {cdur:.3}s（封口段 {} 个）",
        cfinal.display(),
        sealed_before.len()
    );
    assert!(
        cdur >= sealed_floor,
        "成品时长应覆盖全部封口段（≥{sealed_floor:.1}s；得到 {cdur:.3}s）"
    );
    assert!(cdur <= 16.5, "成品时长不得超出录制上界（得到 {cdur:.3}s）");
    for p in &sealed_before {
        assert!(p.exists(), "封口段不得被收尾删除（{}）", p.display());
    }
    let cbytes = fs::read(cfinal).expect("读崩溃恢复成品");
    let ccount = count_mkv(&child_out);
    let fin2 = finalize_session_with_options(&s.dir, true).expect("崩溃收尾应幂等");
    assert_eq!(fin2.route_finals[0].1, *cfinal);
    assert_eq!(
        fs::read(cfinal).expect("重读"),
        cbytes,
        "幂等收尾不得改写成品"
    );
    assert_eq!(count_mkv(&child_out), ccount, "幂等收尾不得新增成品");
    discard_session(&s.dir).expect("崩溃会话应可显式丢弃");
    assert!(!s.dir.exists(), "丢弃后会话目录应删除");
    println!("[recovery] 相位 R 强杀矩阵/Job 回收/幂等/丢弃 OK");

    // ── 相位 R2：仅杀编码进程（主进程存活）→ E_SIDECAR_EXIT → 同进程恢复收尾（§3.5 可重试） ──
    let child2_out = fresh_dir("child2-out");
    let logs2 = fresh_dir("child2-logs");
    let marker2 = logs2.join("child.marker");
    let mut guard2 = spawn_child_record(&child2_out, &marker2, &logs2);
    let _ready2 = wait_child_ready(&marker2, Duration::from_secs(90));
    // 14s：至少跨过一个段切换点（段长受 GOP/关键帧对齐影响，实测 5.0–6.1s 波动）
    std::thread::sleep(Duration::from_secs(14));
    let sidecars = ffmpeg_children_of(guard2.pid());
    assert_eq!(
        sidecars.len(),
        1,
        "运行中应恰 1 个 ffmpeg sidecar（得到 {sidecars:?}）"
    );
    kill_tree(sidecars[0]);
    assert!(
        guard2.wait_exit(Duration::from_secs(30)),
        "编码进程被杀后主进程应检测到 E_SIDECAR_EXIT 并退出 wait_finalized（结果文件：{}）",
        marker2.with_extension("marker.result").display()
    );
    let result =
        fs::read_to_string(format!("{}.result", marker2.display())).expect("子进程结果文件应写入");
    assert!(
        result.starts_with("err"),
        "编码进程死亡应返回错误（得到 {result:?}）"
    );
    assert!(
        result.contains("SidecarExit"),
        "应报 E_SIDECAR_EXIT（§3.5；得到 {result:?}）"
    );
    let sessions = scan_sessions(&child2_out);
    assert!(
        !sessions.is_empty(),
        "编码死亡后应留下可恢复会话（保留原段）"
    );
    let s2 = &sessions[0];
    assert!(!s2.active, "编码进程已死且无活动文件：会话应可收尾");
    assert!(
        !s2.sealed_segments.is_empty(),
        "已封口段应保留（得到 {:?}）",
        s2.sealed_segments
    );
    let sealed2 = s2.sealed_segments.clone();
    let fin2b = finalize_session_with_options(&s2.dir, true)
        .expect("编码死亡后恢复收尾应成功（同进程可重试）");
    let final2 = &fin2b.route_finals[0].1;
    assert!(
        final2.exists() && decode_exit0(final2),
        "恢复成品应存在且可解码（{}）",
        final2.display()
    );
    let dur2 = probe_duration(final2);
    assert!(dur2 > 4.0, "成品时长应含首个封口段（得到 {dur2:.3}s）");
    for p in &sealed2 {
        assert!(p.exists(), "收尾不得删除原段（{}）", p.display());
    }
    discard_session(&s2.dir).expect("恢复后丢弃应成功");
    println!("[recovery] 相位 R2 编码进程死亡/E_SIDECAR_EXIT/恢复收尾 OK（成品 {dur2:.3}s）");

    drop(guard);
    drop(guard2);
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&child_out);
    let _ = fs::remove_dir_all(&child2_out);
    let _ = fs::remove_dir_all(&logs);
    let _ = fs::remove_dir_all(&logs2);
}

// ─────────────────────────── 用例 2：磁盘满故障注入 ───────────────────────────

/// start_route 的 BLOCKED 包装（与 multi_route/route 测试同款注释）。
fn start_route_ok(cfg: RouteConfig) -> sr_pipeline::route::RouteHandle {
    match start_route(cfg) {
        Ok(h) => h,
        Err(e) if matches!(e, RouteError::EncoderUnavailable) => {
            blocked(&format!("无可用编码器（§9 T1）：{e:?}"))
        }
        Err(e) if matches!(e, RouteError::ConsentDenied) => {
            blocked(&format!("WGC 授权被拒（E_CONSENT_DENIED）：{e:?}"))
        }
        Err(e) => panic!("start_route 应成功（得到 {e:?}）"),
    }
}

/// REQ-22/§3.5「磁盘不足（录制中/收尾中）」：写失败注入（ENOSPC 类；机制=录制中途
/// 对会话临时目录 `icacls /deny ...(OI)(CI)W`，S5 §9a 同类写失败路径）→ 该路安全停止
/// 并报 E_DISK_FULL、**保留原段**；解除注入后恢复收尾可重试成功。
#[test]
#[ignore = "L2：需交互式会话 + 可用编码器 + 锁定 ffmpeg n8.1（specs §4）。显式运行：cargo test -p sr-itests --test recovery -- --ignored --nocapture"]
fn it_fault_injection_disk_full() {
    let _g = gate();
    require_interactive_session();
    let _os = record_os_build();

    let user = std::env::var("USERNAME")
        .unwrap_or_else(|_| blocked("USERNAME 缺失（icacls 写拒绝注入需要当前用户）"));
    let out = fresh_dir("diskfull");

    let handle = start_route_ok(RouteConfig {
        target: RouteTarget::Monitor { index: 0 },
        video: VideoCfg {
            canvas_w: 640,
            canvas_h: 360,
            fps: 30,
            cursor: false,
        },
        output_dir: out.clone(),
    });

    // 等第一个封口段落盘（SEGMENT_SECS=5 + 首段起录余量）
    std::thread::sleep(Duration::from_secs(7));
    let tmp_root = out.join(".screenrec_tmp");
    assert!(
        tmp_root.exists(),
        "录制开始后应存在会话临时目录（REQ-24/D12）：{}",
        tmp_root.display()
    );

    // ── 注入：会话临时目录写拒绝（ffmpeg 段切换/CreateFile 失败 → ENOSPC 类写失败） ──
    let mut deny = AclDeny::apply(&tmp_root, &user);
    println!(
        "[diskfull] 已注入写拒绝：{}（user={user}）",
        tmp_root.display()
    );

    let err = handle
        .wait_finalized(Duration::from_secs(60))
        .expect_err("写失败注入后该路应失败（安全停止，§3.5）");
    match err {
        RouteError::DiskFull { ref detail } => println!("[diskfull] E_DISK_FULL：{detail}"),
        other => panic!("写失败应映射 E_DISK_FULL（§3.4/§3.5 磁盘不足；得到 {other:?}）"),
    }

    // 解除注入后恢复（先恢复再触碰目录树：deny 期间目录枚举亦被拒）
    deny.restore();
    println!("[diskfull] 已解除写拒绝，进入恢复");

    // ── 原段保留 + 可解码 ──
    let sessions = scan_sessions(&out);
    assert!(
        !sessions.is_empty(),
        "写失败后应留下可恢复会话（保留原段；§3.5）"
    );
    let s = &sessions[0];
    let sealed = s.sealed_segments.clone();
    assert!(
        !sealed.is_empty(),
        "注入前已封口段应保留（得到 {:?}）",
        sealed
    );
    for p in &sealed {
        assert!(p.exists(), "封口段应保留在磁盘（{}）", p.display());
        assert!(
            probe_duration(p) > 0.0,
            "封口段应可独立读出时长（{}）",
            p.display()
        );
    }
    println!("[diskfull] 失败后保留封口段 {} 个", sealed.len());

    // ── 收尾可重试：恢复收尾应成功且不删原段（M14c：核对原段用 keep=true；默认成功后清理）──
    let fin = finalize_session_with_options(&s.dir, true)
        .expect("解除写失败后恢复收尾应成功（可重试；§3.5）");
    assert!(!fin.route_finals.is_empty(), "应产出成品");
    let final_path = &fin.route_finals[0].1;
    assert!(
        final_path.exists(),
        "成品应存在（{}）",
        final_path.display()
    );
    assert!(decode_exit0(final_path), "恢复成品应可解码");
    let dur = probe_duration(final_path);
    assert!(dur > 0.0, "成品时长应为正（得到 {dur}）");
    for p in &sealed {
        assert!(p.exists(), "收尾不得删除原段（{}）", p.display());
    }
    discard_session(&s.dir).expect("恢复后丢弃会话应成功");

    let _ = fs::remove_dir_all(&out);
}

// ─────────────────────────── 用例 3：M15b/REQ-5 恢复率域子集 ───────────────────────────

/// 合成崩溃残留（2 段，尾段截断）→ 恢复收尾 → 成品音轨保留所选输出率。
/// 复用 S2/S3 的目录重扫与封口判定；不依赖 WGC/真实开录（确定性、快速）。
fn recovery_sample_rate_case(rate: u32) {
    let _g = gate();
    let root = fresh_dir(&format!("synth_rate_{rate}"));
    let session = root.join(".screenrec_tmp").join("sess-rate");
    fs::create_dir_all(&session).expect("创建会话目录");
    let a0 = session.join("out0_00000.mkv");
    let a1 = session.join("out0_00001.mkv");
    mk_seg_with_rate(&a0, 2.0, 440, rate);
    mk_seg_with_rate(&a1, 2.0, 660, rate);
    truncate_to(&a1, 0.6); // 崩溃尾段（未封口形态）
    save_manifest_atomic(
        &session,
        &manifest_with(
            "sess-rate",
            vec![route_with(
                "route-rate",
                "Recovery Rate",
                &["v", "a"],
                vec![seg_entry(&a0, 2.0, true), seg_entry(&a1, 2.0, false)],
            )],
        ),
    )
    .expect("写会话清单");

    let sessions = scan_sessions(&root);
    let s = find_session(&sessions, &session);
    assert!(!s.active, "合成残留会话应为非活跃");
    assert!(
        s.sealed_segments.contains(&a0),
        "首个封口段应可收尾（得到 {:?}）",
        s.sealed_segments
    );

    // M15b 核对原段/保留现场：显式 keep=true（M14c 默认清理单独覆盖）。
    let fin = finalize_session_with_options(&s.dir, true).expect("恢复收尾应成功");
    assert_eq!(fin.route_finals.len(), 1, "单路会话应恰 1 个成品");
    let final_path = &fin.route_finals[0].1;
    assert!(
        final_path.exists(),
        "成品应存在（{}）",
        final_path.display()
    );
    assert!(decode_exit0(final_path), "恢复成品应可解码");
    assert_eq!(
        probe_audio_shape(final_path),
        format!("aac,{rate},2"),
        "REQ-5：恢复成品音轨应保留 {rate} 输出率"
    );
    let dur = probe_duration(final_path);
    assert!(
        (dur - 2.0).abs() <= SEG_DUR_TOL,
        "成品时长应≈封口段和（2.0s；得到 {dur:.3}s）"
    );
    for p in &s.sealed_segments {
        assert!(p.exists(), "keep=true 收尾应保留原段（{}）", p.display());
    }
    println!(
        "[recovery-rate-{rate}] 成品 {} 时长 {dur:.3}s；音轨 aac,{rate},2；原段保留 {} 个",
        final_path.display(),
        s.sealed_segments.len()
    );
    let _ = fs::remove_dir_all(&root);
}

/// M15b/REQ-5：44.1k 输出率的恢复子集（isolation 由 audio_isolation 套件覆盖）。
#[test]
#[ignore = "L2/M15b：合成崩溃残留 + 44.1k 输出率恢复子集（锁定 ffmpeg）"]
fn it_recovery_sample_rate_44100() {
    recovery_sample_rate_case(44_100);
}

/// M15b/REQ-5：96k 输出率的恢复子集。
#[test]
#[ignore = "L2/M15b：合成崩溃残留 + 96k 输出率恢复子集（锁定 ffmpeg）"]
fn it_recovery_sample_rate_96000() {
    recovery_sample_rate_case(96_000);
}

/// M15c/acc-7：清单已记 `final_path` 但成品零字节（损坏）→ 收尾必须报错、
/// 保留会话目录与原段、不得按默认策略清理；错误信息须指出成品校验失败。
#[test]
#[ignore = "L2/M15c：合成清单 + 零字节成品回归（锁定 ffmpeg 造段；无需编码器）"]
fn it_recovery_corrupt_recorded_final_keeps_session() {
    let _g = gate();
    let root = fresh_dir("corrupt-final");
    let session = root.join(".screenrec_tmp").join("sess-corrupt-final");
    fs::create_dir_all(&session).expect("创建会话目录");
    let seg = session.join("out0_00000.mkv");
    mk_seg(&seg, 2.0, 440);
    let final_path = root.join("20261018_120000_Corrupt Final.mkv");
    fs::write(&final_path, b"").expect("写零字节成品");

    let mut route = route_with(
        "route-corrupt",
        "Corrupt Final",
        &["v", "a"],
        vec![seg_entry(&seg, 2.0, true)],
    );
    route.state = "final".to_string();
    route.final_path = Some(final_path.clone());
    save_manifest_atomic(&session, &manifest_with("sess-corrupt-final", vec![route]))
        .expect("写会话清单");

    let err = finalize_session(&session)
        .expect_err("清单已记成品但零字节：收尾不得判成功（否则默认清理会删掉原段）");
    let detail = match &err {
        RecoveryError::RecoveryFailed { detail } => detail.clone(),
        other => panic!("应报 RecoveryFailed（得到 {other:?}）"),
    };
    assert!(
        detail.contains("成品校验失败"),
        "错误应可见地指出成品校验失败（得到 {detail}）"
    );
    assert!(
        detail.contains(&final_path.display().to_string()),
        "错误应包含损坏成品路径（得到 {detail}）"
    );
    assert!(
        session.exists() && session.join("manifest.json").exists(),
        "校验失败必须保留会话目录/清单（{}）",
        session.display()
    );
    assert!(seg.exists(), "校验失败必须保留原段（{}）", seg.display());
    assert_eq!(
        fs::metadata(&final_path).expect("成品元数据").len(),
        0,
        "损坏成品不得被静默改写/删除"
    );
    println!("[recovery] M15c 零字节成品回归 OK：{detail}");
    let _ = fs::remove_dir_all(&root);
}
