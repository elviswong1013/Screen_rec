//! P1-3：纯录音进入恢复状态机（manifest + WAV 路径可被 recovery 扫描发现）。
//!
//! 场景（L2，`#[ignore]`）：子进程实际 `start_audio_only`（System 环回）→ 主进程强杀 →
//! `recovery::scan_sessions` 重扫发现会话（manifest.streams 含 "wav"、WAV 文件存在、
//! 非活动）→ `finalize_session` 就地封口/修复 WAV → 时长可读。
//!
//! 环境缺失（无默认渲染端点）→ BLOCKED（fail，不静默 skip）。

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use common::*;
use sr_audio::wasapi::AudioSourceSpec;
use sr_pipeline::recovery::{self, RecoveryError};
use sr_pipeline::route::{start_audio_only, AudioOnlyConfig};

static GATE: Mutex<()> = Mutex::new(());

const CHILD_ENV: &str = "SR_M11_AUDIO_ONLY_CHILD";
const CHILD_OUT_ENV: &str = "SR_M11_AUDIO_ONLY_OUT";
const CHILD_MARKER_ENV: &str = "SR_M11_AUDIO_ONLY_MARKER";
const CHILD_PID_ENV: &str = "SR_M11_AUDIO_ONLY_SRC_PID";
const CHILD_CREATED_ENV: &str = "SR_M11_AUDIO_ONLY_SRC_CREATED";
const CHILD_TEST: &str = "it_audio_only_recovery_child_mode";

fn ffbin(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin")
        .join(name)
}

/// 生成 40s 正弦 WAV（480Hz），供 ffplay 播放 → 子进程进程环回有真实音频包。
fn gen_tone_wav(dir: &Path) -> PathBuf {
    let path = dir.join("tone480.wav");
    let out = Command::new(ffbin("ffmpeg.exe"))
        .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg("sine=frequency=480:sample_rate=48000:duration=40,volume=0.25")
        .args(["-ac", "2", "-c:a", "pcm_s16le"])
        .arg(&path)
        .output()
        .unwrap_or_else(|e| blocked(&format!("无法生成测试音：{e}")));
    assert!(
        out.status.success(),
        "生成测试音失败: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    path
}

struct Player(Child);
impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_player(wav: &Path) -> Player {
    let child = Command::new(ffbin("ffplay.exe"))
        .args(["-nodisp", "-autoexit", "-loglevel", "error"])
        .arg(wav)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn ffplay：{e}")));
    Player(child)
}

fn ffprobe_csv(path: &Path, args: &[&str]) -> String {
    let ffprobe = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin/ffprobe.exe");
    let out = Command::new(&ffprobe)
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

fn wav_duration(path: &Path) -> f64 {
    let s = ffprobe_csv(
        path,
        &["-show_entries", "format=duration", "-of", "csv=p=0"],
    );
    s.parse::<f64>().unwrap_or(0.0)
}

fn fresh_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr_m11_audio_only_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("创建临时目录失败");
    d
}

fn wait_ready(marker: &Path, timeout: Duration) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(s) = std::fs::read_to_string(marker) {
            if !s.trim().is_empty() {
                return s;
            }
        }
        if Instant::now() >= deadline {
            blocked(&format!(
                "子进程 {timeout:?} 内未就绪（{}）",
                marker.display()
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// 子进程分支：实际开录（System 环回）→ 写就绪标记 → 阻塞等待被杀。
#[test]
#[ignore = "P1-3 子进程分支（由 it_audio_only_crash_recovery 以环境变量触发）"]
fn it_audio_only_recovery_child_mode() {
    if std::env::var(CHILD_ENV).is_err() {
        return; // 直接运行（非子进程）时无副作用
    }
    let out = PathBuf::from(std::env::var(CHILD_OUT_ENV).expect("child output env"));
    let marker = PathBuf::from(std::env::var(CHILD_MARKER_ENV).expect("child marker env"));
    let source = match (
        std::env::var(CHILD_PID_ENV),
        std::env::var(CHILD_CREATED_ENV),
    ) {
        (Ok(pid), Ok(created)) => AudioSourceSpec::App {
            pid: pid.parse().expect("src pid"),
            created_at_unix_ms: created.parse().expect("src created"),
        },
        _ => AudioSourceSpec::System,
    };
    let handle = match start_audio_only(AudioOnlyConfig {
        sources: vec![source],
        output_dir: out,
    }) {
        Ok(h) => h,
        Err(e) => {
            let _ = std::fs::write(&marker, format!("err {e:?}"));
            std::process::exit(3);
        }
    };
    let _ = std::fs::write(&marker, format!("ok {}", std::process::id()));
    println!("[child] 纯录音已启动；阻塞等待强杀…");
    let _ = handle.wait_finalized(Duration::from_secs(600));
    std::process::exit(0);
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "L2：需交互式会话 + 默认渲染端点；实际开录→强杀→重扫→恢复（P1-3）"]
fn it_audio_only_crash_recovery() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let root = fresh_dir("crash");
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("创建输出目录失败");
    let marker = root.join("child.ready");
    let logs = root.join("logs");
    std::fs::create_dir_all(&logs).expect("创建日志目录失败");
    // 真实音频源：ffplay 播放 480Hz 连续音 → 子进程 App 环回有音频包（非静音）。
    let tone = gen_tone_wav(&root);
    let player = spawn_player(&tone);
    std::thread::sleep(Duration::from_millis(800));
    let src_pid = player.0.id();
    let src_created =
        process_created_unix_ms(src_pid).unwrap_or_else(|| blocked("无法读取播放器进程创建时间"));
    let exe = std::env::current_exe().expect("current_exe");
    let child = Command::new(exe)
        .args(["--exact", CHILD_TEST, "--ignored", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env(CHILD_OUT_ENV, &out)
        .env(CHILD_MARKER_ENV, &marker)
        .env(CHILD_PID_ENV, src_pid.to_string())
        .env(CHILD_CREATED_ENV, src_created.to_string())
        .stdout(Stdio::from(
            std::fs::File::create(logs.join("child.out")).expect("child stdout"),
        ))
        .stderr(Stdio::from(
            std::fs::File::create(logs.join("child.err")).expect("child stderr"),
        ))
        .spawn()
        .expect("spawn 纯录音子进程");
    let mut guard = ChildGuard(child);
    let ready = wait_ready(&marker, Duration::from_secs(30));
    println!("[audio-recovery] 子进程就绪：{ready}");
    if ready.starts_with("err") {
        let stderr = std::fs::read_to_string(logs.join("child.err")).unwrap_or_default();
        if stderr.contains("E_LOOPBACK_UNAVAILABLE") || ready.contains("LoopbackUnavailable") {
            blocked(&format!("默认渲染端点不可用（REQ-15 环境缺失）：{ready}"));
        }
        panic!("子进程纯录音启动失败：{ready}");
    }

    // 录制约 4s（跨过 WAV 头写与若干采集包），再强杀（模拟主进程崩溃）。
    let span = {
        let t = Instant::now();
        std::thread::sleep(Duration::from_secs(4));
        t.elapsed().as_secs_f64()
    };
    guard.0.kill().expect("强杀子进程");
    let _ = guard.0.wait();
    let _ = guard.0.try_wait();
    std::thread::sleep(Duration::from_millis(500)); // 句柄关闭

    // 重扫：会话可发现（manifest 含 wav 路由 + WAV 路径在会话目录内）。
    let sessions = recovery::scan_sessions(&out);
    assert!(
        !sessions.is_empty(),
        "recovery 应发现纯录音会话（{}）",
        out.display()
    );
    let session = sessions
        .iter()
        .find(|s| {
            s.manifest
                .as_ref()
                .map(|m| {
                    m.routes
                        .iter()
                        .any(|r| r.streams.iter().any(|x| x == "wav"))
                })
                .unwrap_or(false)
        })
        .unwrap_or_else(|| panic!("应存在含 wav 流的会话（扫描到 {} 个）", sessions.len()));
    assert!(!session.active, "强杀后会话不应为活动（WAV 句柄应已关闭）");
    let wavs: Vec<PathBuf> = std::fs::read_dir(&session.dir)
        .expect("读取会话目录失败")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .map_or(false, |x| x.eq_ignore_ascii_case("wav"))
        })
        .collect();
    assert_eq!(wavs.len(), 1, "会话目录应有 1 个 WAV（得到 {wavs:?}）");
    println!(
        "[audio-recovery] 重扫发现：{}（wav={}，active=false）",
        session.dir.display(),
        wavs[0].display()
    );

    // 恢复：就地封口/修复；时长应覆盖录制段（±2s）。
    let outcome = match recovery::finalize_session(&session.dir) {
        Ok(o) => o,
        Err(RecoveryError::Active { .. }) => panic!("强杀后不应判定活动"),
        Err(e) => panic!("恢复应成功（得到 {e:?}）"),
    };
    assert_eq!(
        outcome.repaired_wavs.len(),
        1,
        "应修复 1 个 WAV（得到 {:?}）",
        outcome.repaired_wavs
    );
    let dur = wav_duration(&outcome.repaired_wavs[0]);
    println!("[audio-recovery] 修复后 WAV 时长 {dur:.3}s（录制段 {span:.3}s）");
    assert!(
        (dur - span).abs() <= 2.0,
        "修复后 WAV 时长应≈录制段（{dur:.3}s vs {span:.3}s）"
    );
    recovery::discard_session(&session.dir).expect("恢复后应可丢弃会话");
    println!("[audio-recovery] 结论：manifest 可发现 + 强杀后可恢复 + 时长正确");
}
