//! L2 集成测试：音频来源枚举与快照（REQ-4，specs/001-screen-recorder.md §2.1 / §4）。
//!
//! 运行方式（L2 门：`#[ignore]`，默认 `cargo test` 不执行本文件用例）：
//! ```text
//! cargo test -p sr-itests --test enum_audio -- --ignored --nocapture
//! ```
//! 前置条件：交互式用户会话 + tools/ffmpeg/ffmpeg-master-latest-win64-lgpl/bin/{ffmpeg,ffplay}.exe
//! + 可用音频渲染端点；缺失 → BLOCKED（fail），不静默 skip（§4）。
//! 本机环境（红测试编写时）：Windows 11 25H2，build 26200.9457，SESSIONNAME=Console。
//!
//! 手法：ffmpeg 临时生成 440/880Hz 测试音 WAV → ffplay（锁定构建，绝对路径）循环产出
//! 活动音频会话 → 断言快照含对应 PID/进程名/身份字段；「全选=快照」语义在 M1 的
//! 可测部分：枚举返回的是「值快照」，启动后新增音源不会自动并入已取快照。
//!
//! 接口契约（测试即契约，详见 docs/milestones/m1-red-report.md）：
//! - `sr_enum::audio_sessions::list_audio_sessions() -> Result<Vec<AudioSessionEntry>, E>`
//!   `AudioSessionEntry { pid: u32, process_created_at: u64(UNIX ms),
//!                        process_name: String, app_name: String }`
//! E 仅要求 `.expect` 可用（具体错误枚举见规格 §3.4，后续任务钉死）。

mod common;

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use common::*;
use sr_enum::audio_sessions::{list_audio_sessions, AudioSessionEntry};

/// ffplay/ffmpeg 位置：任务锁定的仓库内构建（tests/sr-itests 为 manifest 根）。
fn ffmpeg_bin(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/ffmpeg/ffmpeg-master-latest-win64-lgpl/bin")
        .join(name)
        .canonicalize()
        .unwrap_or_else(|e| {
            blocked(&format!(
                "无法定位 tools/ffmpeg/.../{name}.exe（canonicalize 失败：{e}）"
            ))
        });
    if !p.exists() {
        blocked(&format!("锁定 ffmpeg 构建缺失：{}", p.display()));
    }
    p
}

/// 进程退出即清理的守卫（杀 ffplay，避免音频残留）。
struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// 用锁定 ffmpeg 生成测试音 WAV（sine → pcm_s16le 48k 立体声）。
fn gen_tone_wav(dir: &std::path::Path, name: &str, hz: u32) -> PathBuf {
    let out = dir.join(name);
    let st = Command::new(ffmpeg_bin("ffmpeg.exe"))
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency={hz}:sample_rate=48000:duration=120"),
            "-ac",
            "2",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(&out)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap_or_else(|e| blocked(&format!("无法运行锁定 ffmpeg 生成测试音：{e}")));
    if !st.success() {
        blocked(&format!("ffmpeg 生成测试音失败（exit={st}）"));
    }
    out
}

fn spawn_ffplay(wav: &std::path::Path) -> KillOnDrop {
    let child = Command::new(ffmpeg_bin("ffplay.exe"))
        .args([
            "-nodisp",
            "-autoexit",
            "-loglevel",
            "quiet",
            "-volume",
            "40",
        ])
        .arg(wav)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn 锁定 ffplay：{e}")));
    KillOnDrop(child)
}

/// 轮询直到快照中出现指定 PID 的音源（返回该条目）。
fn wait_for_pid(pid: u32, timeout: Duration) -> Option<AudioSessionEntry> {
    wait_until(timeout, Duration::from_millis(400), || {
        list_audio_sessions()
            .expect("REQ-4：list_audio_sessions 应成功")
            .into_iter()
            .find(|s| s.pid == pid)
    })
}

fn assert_entry_identity(
    entry: &AudioSessionEntry,
    child: &KillOnDrop,
    label: &str,
    true_created_ms: u64,
) {
    assert_eq!(
        entry.pid,
        child.0.id(),
        "REQ-4：{label} 的 PID 应为 ffplay 进程"
    );
    assert!(
        entry.process_name.to_lowercase().contains("ffplay"),
        "REQ-4：{label} 进程名应解析为 ffplay（得到 {:?}）",
        entry.process_name
    );
    assert!(
        !entry.app_name.trim().is_empty(),
        "REQ-4：{label} 应用名不得为空"
    );
    assert!(
        entry.process_created_at.abs_diff(true_created_ms) <= 2_000,
        "REQ-4：{label} 身份创建时间偏差 > 2s（与 REQ-1 同源的身份字段）"
    );
}

/// REQ-4：列出活动音频会话（PID/进程名/应用名）；「全选=启动时快照」在 M1 的可测部分
/// （新增音源不自动并入已取快照）；同根去重的弱形式（同 PID 恰一条）。
#[test]
#[ignore = "L2：需交互式会话/可用音频端点/锁定 ffplay（specs §4）。显式运行：cargo test -p sr-itests --test enum_audio -- --ignored --nocapture"]
fn it_list_active_audio_sources_snapshot() {
    require_interactive_session();
    let _os = record_os_build();

    let tmp = std::env::temp_dir().join(format!("sr_m1_red_audio_{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("创建临时目录");
    let wav_a = gen_tone_wav(&tmp, "tone_a_440.wav", 440);

    // 音源 A 启动 → 活动会话
    let mut player_a = spawn_ffplay(&wav_a);
    let pid_a = player_a.0.id();
    let created_a = process_created_unix_ms(pid_a)
        .unwrap_or_else(|| blocked("无法读取 ffplay A 创建时间（身份地面真值）"));
    let entry_a = wait_for_pid(pid_a, Duration::from_secs(20)).unwrap_or_else(|| {
        if player_a.0.try_wait().map_or(false, |s| s.is_some()) {
            blocked("ffplay A 已退出（音频端点不可用？），无法构造活动音频会话前置条件")
        } else {
            panic!("REQ-4：ffplay A 播放中，20s 内快照未出现其会话（pid={pid_a}）")
        }
    });
    assert_entry_identity(&entry_a, &player_a, "音源 A", created_a);

    // 同根去重（弱形式）：同一 PID 恰好一条（同应用多会话归一，REQ-4）
    let snap1 = list_audio_sessions().expect("REQ-4：list_audio_sessions 应成功");
    let count_a = snap1.iter().filter(|s| s.pid == pid_a).count();
    assert_eq!(
        count_a, 1,
        "REQ-4：同一 PID（同根）的音频会话应归一为恰好 1 条（得到 {count_a}）"
    );

    // 「全选=启动时快照」可测部分：以 snap1 为「启动时快照」；
    // 启动后新增音源 B（880Hz）→ 旧快照不得自动并入 B；新取快照应含 A 与 B。
    let wav_b = gen_tone_wav(&tmp, "tone_b_880.wav", 880);
    let mut player_b = spawn_ffplay(&wav_b);
    let pid_b = player_b.0.id();
    let created_b = process_created_unix_ms(pid_b)
        .unwrap_or_else(|| blocked("无法读取 ffplay B 创建时间（身份地面真值）"));
    let entry_b = wait_for_pid(pid_b, Duration::from_secs(20)).unwrap_or_else(|| {
        if player_b.0.try_wait().map_or(false, |s| s.is_some()) {
            blocked("ffplay B 已退出（音频端点不可用？），无法构造新增音源前置条件")
        } else {
            panic!("REQ-4：新增音源 B 播放中，20s 内新快照未出现其会话（pid={pid_b}）")
        }
    });
    assert_entry_identity(&entry_b, &player_b, "音源 B", created_b);

    assert!(
        !snap1.iter().any(|s| s.pid == pid_b),
        "REQ-4：启动时快照 snap1 不得自动并入启动后新增的音源 B（快照语义；新增须 UI 手动追加）"
    );
    let snap2 = list_audio_sessions().expect("REQ-4：list_audio_sessions 应成功");
    assert!(
        snap2.iter().any(|s| s.pid == pid_a) && snap2.iter().any(|s| s.pid == pid_b),
        "REQ-4：新取快照应同时含 A 与 B（枚举反映当前活动会话）"
    );

    // 音源退出 → 会话从新快照消失（容忍 WASAPI 会话回收延迟，轮询 ≤20s）
    drop(player_b);
    let gone = wait_until(Duration::from_secs(20), Duration::from_millis(400), || {
        let still = list_audio_sessions()
            .expect("REQ-4：list_audio_sessions 应成功")
            .iter()
            .any(|s| s.pid == pid_b);
        if still {
            None
        } else {
            Some(())
        }
    });
    assert!(
        gone.is_some(),
        "REQ-4：音源 B 退出后其会话应从后续快照消失（20s 内）"
    );

    let _ = std::fs::remove_dir_all(&tmp);
    // player_a 由 KillOnDrop 清理
}
