//! P2：编码器设置生效（settings.encoder → 命令 DTO → 管线）。
//!
//! 断言：
//! - 显式不存在的编码器 → `EncoderUnavailable`（**不得**静默回退到 D4 链）；
//! - 显式可用编码器（本机 `h264_mf` 若可得）→ 正常录制/收尾，成品可解码。

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use common::*;
use sr_pipeline::route::{
    start_routes_with_encoder, MultiRouteConfig, RouteError, RouteTarget, VideoCfg,
};

static GATE: Mutex<()> = Mutex::new(());

fn ffbin(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin")
        .join(name)
}

fn cfg(out: &Path) -> MultiRouteConfig {
    MultiRouteConfig {
        target: RouteTarget::Monitor { index: 0 },
        video: VideoCfg {
            canvas_w: 640,
            canvas_h: 360,
            fps: 30,
            cursor: false,
        },
        audio: Vec::new(),
        output_dir: out.to_path_buf(),
    }
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

fn fresh_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr_m11_encoder_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("创建临时目录失败");
    d
}

#[test]
#[ignore = "L2：显式编码器覆盖（P2；非法名不得回退 D4 链）"]
fn it_encoder_override_is_authoritative() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let out = fresh_dir("bad");
    let err = start_routes_with_encoder(vec![cfg(&out)], Some("__no_such_encoder__".to_string()))
        .err()
        .expect("非法显式编码器必须启动失败（不得回退探测链）");
    assert!(
        matches!(err, RouteError::EncoderUnavailable),
        "应为 EncoderUnavailable（得到 {err:?}）"
    );
    println!("[encoder] 非法名 → EncoderUnavailable（无回退）");

    if !sr_pipeline::sidecar::probe_named_encoder("h264_mf") {
        blocked("本机锁定 ffmpeg 的 h264_mf 不可用（显式设置正路径 BLOCKED）");
    }
    let out = fresh_dir("ok");
    let handles = start_routes_with_encoder(vec![cfg(&out)], Some("h264_mf".to_string()))
        .expect("显式可用的 h264_mf 应可启动");
    std::thread::sleep(Duration::from_secs(4));
    handles[0].request_stop();
    let fin = handles[0]
        .wait_finalized(Duration::from_secs(120))
        .expect("收尾应成功");
    assert!(fin.final_path.exists());
    assert!(decode_clean(&fin.final_path), "显式编码器成品应可解码");
    println!(
        "[encoder] h264_mf 显式设置 → 成品 {}（可解码）",
        fin.final_path.display()
    );
}
