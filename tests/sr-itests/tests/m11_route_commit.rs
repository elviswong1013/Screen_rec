//! P1-4：正常收尾安全提交（临时成品→验证→safe_commit）+ 同秒同目标并发预留。
//!
//! 断言：
//! - 同批两路（同秒、同目标、同输出目录）启动 → 预留的最终名互不相同；收尾后两个
//!   成品都存在且各自可解码；
//! - 收尾只提交到启动时预留的名字（`reserved_final_path`），成功后无 `.tmp` 残留；
//! - 提交阶段冲突（外部占用最终名）→ `FinalizeFailed`、不覆盖既有文件、无半成品、
//!   段保留（可恢复/重试）。
//!
//! 环境缺失（编码器/WGC）→ BLOCKED。

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use common::*;
use sr_pipeline::route::{
    start_routes_with_encoder, start_routes_with_options, MultiRouteConfig, RouteError,
    RouteTarget, VideoCfg,
};

static GATE: Mutex<()> = Mutex::new(());

fn ffbin(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/ffmpeg/ffmpeg-n8.1-latest-win64-lgpl-8.1/bin")
        .join(name)
}

fn probe_duration(path: &Path) -> f64 {
    let out = Command::new(ffbin("ffprobe.exe"))
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn 锁定 ffprobe：{e}")));
    assert!(
        out.status.success(),
        "ffprobe 失败: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0.0)
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

fn fresh_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr_m11_commit_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("创建临时目录失败");
    d
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

fn tmp_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir.join(".screenrec_tmp")) {
        for session in rd.flatten() {
            if let Ok(files) = std::fs::read_dir(session.path()) {
                out.extend(
                    files
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| p.to_string_lossy().contains(".tmp")),
                );
            }
        }
    }
    out
}

/// 会话临时目录下的残留子目录（M14：默认成功后应为空；keep_segments=true 时保留）。
fn session_dirs(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir.join(".screenrec_tmp"))
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

#[test]
#[ignore = "L2：需交互式会话/WGC/编码器；同秒同目标两路并发 + 安全提交（P1-4）"]
fn it_route_same_second_concurrent_safe_commit() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let out = fresh_dir("concurrent");
    let cfgs = vec![cfg(&out), cfg(&out)];
    let handles = match start_routes_with_encoder(cfgs, None) {
        Ok(h) => h,
        Err(e) if matches!(e, RouteError::EncoderUnavailable) => {
            blocked(&format!("无可用编码器：{e:?}"))
        }
        Err(e) if matches!(e, RouteError::ConsentDenied) => {
            blocked(&format!("WGC 授权被拒：{e:?}"))
        }
        Err(e) => panic!("两路启动应成功（得到 {e:?}）"),
    };
    assert_eq!(handles.len(), 2);
    let reserved: Vec<PathBuf> = handles.iter().map(|h| h.reserved_final_path()).collect();
    assert_ne!(
        reserved[0], reserved[1],
        "同秒同目标并发两路必须预留不同最终名"
    );
    println!(
        "[commit] 预留：{} / {}",
        reserved[0].display(),
        reserved[1].display()
    );

    std::thread::sleep(Duration::from_secs(4));
    for h in &handles {
        h.request_stop();
    }
    let finals: Vec<_> = handles
        .iter()
        .map(|h| {
            h.wait_finalized(Duration::from_secs(120))
                .expect("并发两路收尾均应成功")
        })
        .collect();

    let mut seen = std::collections::HashSet::new();
    for (i, fin) in finals.iter().enumerate() {
        assert_eq!(
            fin.final_path, reserved[i],
            "收尾必须提交到启动时预留的名字"
        );
        assert!(
            fin.final_path.exists(),
            "成品应存在：{}",
            fin.final_path.display()
        );
        assert!(seen.insert(fin.final_path.clone()), "成品名不得重复");
        let dur = probe_duration(&fin.final_path);
        assert!(dur > 2.0, "成品时长应>2s（得到 {dur:.3}s）");
        assert!(
            decode_clean(&fin.final_path),
            "成品应可解码：{}",
            fin.final_path.display()
        );
        println!(
            "[commit] 成品[{i}] {}（{dur:.3}s）",
            fin.final_path.display()
        );
    }
    assert!(
        tmp_files(&out).is_empty(),
        "成功提交后不应残留 .tmp：{:?}",
        tmp_files(&out)
    );
    // M14：默认（keep_segments=false）成品验证成功后清理会话临时目录。
    assert!(
        session_dirs(&out).is_empty(),
        "M14：默认成功后 .screenrec_tmp 下不应残留会话目录：{:?}",
        session_dirs(&out)
    );
    for (i, fin) in finals.iter().enumerate() {
        let session = fin
            .segment_list
            .parent()
            .expect("segment_list 应位于会话目录内");
        assert!(
            !session.exists(),
            "M14：成品[{i}] 成功后会话目录应被删除（{}）",
            session.display()
        );
    }
    println!("[commit] 结论：同秒同目标并发 → 两个独立成品，无覆盖/无 .tmp 残留/默认清理原段");
}

#[test]
#[ignore = "L2：需交互式会话/WGC/编码器；提交阶段冲突 → 不覆盖/无半成品（P1-4）"]
fn it_route_commit_collision_keeps_existing_and_segments() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let out = fresh_dir("collision");
    let handles = match start_routes_with_encoder(vec![cfg(&out)], None) {
        Ok(h) => h,
        Err(e) if matches!(e, RouteError::EncoderUnavailable) => {
            blocked(&format!("无可用编码器：{e:?}"))
        }
        Err(e) if matches!(e, RouteError::ConsentDenied) => {
            blocked(&format!("WGC 授权被拒：{e:?}"))
        }
        Err(e) => panic!("启动应成功（得到 {e:?}）"),
    };
    let handle = &handles[0];
    let reserved = handle.reserved_final_path();
    // 模拟提交窗口内的外部占用：既有文件必须原样保留（禁止覆盖）。
    std::fs::write(&reserved, b"EXTERNAL-SENTINEL").expect("写入占位最终文件失败");
    println!("[commit] 占用最终名：{}", reserved.display());

    std::thread::sleep(Duration::from_secs(4));
    handle.request_stop();
    let res = handle.wait_finalized(Duration::from_secs(120));
    assert!(
        matches!(res, Err(RouteError::FinalizeFailed { .. })),
        "最终名被占用时收尾应显式失败（得到 {res:?}）"
    );
    assert_eq!(
        std::fs::read(&reserved).expect("占位文件应仍存在"),
        b"EXTERNAL-SENTINEL",
        "既有文件不得被覆盖"
    );
    let segs = handle.segments();
    assert!(!segs.is_empty(), "收尾失败必须保留分段（可恢复）");
    let tmp = tmp_files(&out);
    assert!(tmp.is_empty(), "失败路径应清理临时成品（残留 {tmp:?}）");
    println!(
        "[commit] 结论：提交冲突 → 既有文件保留、无半成品、段保留（{} 段）",
        segs.len()
    );
}

#[test]
#[ignore = "L2：需交互式会话/WGC/编码器；M14 keep_segments=true 成功后保留会话目录"]
fn it_route_keep_segments_preserves_session_tmp() {
    let _g = GATE.lock().unwrap_or_else(|e| e.into_inner());
    require_interactive_session();
    let _os = record_os_build();

    let out = fresh_dir("keep");
    let handles = match start_routes_with_options(vec![cfg(&out)], None, true) {
        Ok(h) => h,
        Err(e) if matches!(e, RouteError::EncoderUnavailable) => {
            blocked(&format!("无可用编码器：{e:?}"))
        }
        Err(e) if matches!(e, RouteError::ConsentDenied) => {
            blocked(&format!("WGC 授权被拒：{e:?}"))
        }
        Err(e) => panic!("启动应成功（得到 {e:?}）"),
    };
    let handle = &handles[0];
    std::thread::sleep(Duration::from_secs(4));
    handle.request_stop();
    let fin = handle
        .wait_finalized(Duration::from_secs(120))
        .expect("keep_segments=true 时收尾应成功");

    assert!(
        fin.final_path.exists(),
        "成品应存在：{}",
        fin.final_path.display()
    );
    let session = fin
        .segment_list
        .parent()
        .expect("segment_list 应位于会话目录内");
    assert!(
        session.exists(),
        "M14：keep_segments=true 时会话临时目录应保留（{}）",
        session.display()
    );
    assert!(
        fin.segment_list.exists(),
        "M14：keep_segments=true 时 ffconcat 清单应保留（{}）",
        fin.segment_list.display()
    );
    assert!(
        !fin.segments.is_empty(),
        "M14：keep_segments=true 时应保留参与收尾的原段"
    );
    for s in &fin.segments {
        assert!(
            s.exists() && s.metadata().map_or(false, |m| m.len() > 0),
            "M14：keep_segments=true 时原段应保留且非空（{}）",
            s.display()
        );
    }
    println!(
        "[commit] 结论：keep_segments=true → 成品存在且会话目录/原段保留（{} 段）",
        fin.segments.len()
    );
    let _ = std::fs::remove_dir_all(&out);
}
