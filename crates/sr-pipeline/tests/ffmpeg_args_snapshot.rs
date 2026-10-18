//! L1 单元测试：ffmpeg 编码命令参数构建（REQ-20，specs/001-screen-recorder.md §3.3
//! 基准命令 / §8 M0 冻结项；docs/spikes/s1b §7）。
//!
//! 运行方式（L1：随默认 `cargo test` 执行）：
//! ```text
//! cargo test -p sr-pipeline --test ffmpeg_args_snapshot -- --nocapture
//! ```
//!
//! 接口契约（测试即契约，详见 docs/milestones/m3-red-report.md）：
//! - `sr_pipeline::sidecar::VideoPipeSpec { width, height, fps, pipe_path: String }`
//! - `sr_pipeline::sidecar::AudioPipeSpec { pipe_path: String }`（f32le/48k/立体声固定，§3.3）
//! - `sr_pipeline::sidecar::build_encode_args(video, audio, video_encoder, segment_secs,
//!    segment_list, output_pattern) -> Vec<String>`
//!
//! 断言策略：只断言 §3.3 基准的**关键要素**（存在性 + 语义相关的相邻性），不断言完整
//! argv 顺序（避免脆断言）：
//! - `-hide_banner -nostdin`（stderr 持续排空 + 禁 stdin，S1b §7.6）
//! - 视频输入：`-f rawvideo -pix_fmt nv12 -s {W}x{H} -framerate {fps} -i {pipe}`（连续序列）
//! - 音频输入：`-f f32le -ar 48000 -ac 2 -i {pipe}`；无音频时不出现
//! - 映射：`-map 0:v` + 每路音频 `-map {k}:a`
//! - `-c:v {encoder}`（D4 链探测结果的透传）+ `-g 2*fps`（闭合 GOP）+
//!   `-force_key_frames expr:gte(t,n_forced*{segment_secs})`（段首 IDR，S5 冻结）
//! - `-c:a aac -b:a 192k`（有音频时）
//! - 分段：`-f segment -segment_time {secs} -reset_timestamps 1 -segment_format matroska` +
//!   `-segment_list {list}`；输出 pattern 为最后一个参数

use std::path::PathBuf;

use sr_pipeline::sidecar::{build_encode_args, AudioPipeSpec, VideoPipeSpec};

/// args 中是否存在按序相邻的连续子序列 seq。
fn has_seq(args: &[String], seq: &[&str]) -> bool {
    if seq.is_empty() || args.len() < seq.len() {
        return false;
    }
    args.windows(seq.len())
        .any(|w| w.iter().zip(seq).all(|(a, e)| a == e))
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sr_m3_red_args_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&d);
    d.join(name)
}

#[test]
fn unit_ffmpeg_args_snapshot() {
    // ── 用例 A：1080p30 + 1 路音频 + qsv（§3.3 基准形状） ──
    let pipe_v = r"\\.\pipe\screenrec_s1_v".to_string();
    let pipe_a = r"\\.\pipe\screenrec_s1_a0".to_string();
    let list = tmp("segments.ffconcat");
    let pattern = tmp("out_%05d.mkv");
    let args = build_encode_args(
        &VideoPipeSpec {
            width: 1920,
            height: 1080,
            fps: 30,
            pipe_path: pipe_v.clone(),
        },
        &[AudioPipeSpec {
            pipe_path: pipe_a.clone(),
        }],
        "h264_qsv",
        5,
        &list,
        &pattern,
    );

    assert!(!args.is_empty(), "argv 不应为空");
    assert!(args.iter().all(|a| !a.is_empty()), "argv 不应含空串");
    assert!(
        args.contains(&"-hide_banner".to_string()),
        "应含 -hide_banner"
    );
    assert!(
        args.contains(&"-nostdin".to_string()),
        "应含 -nostdin（优雅停止只走 EOF，S1b §7.6）"
    );

    // 视频输入（rawvideo/nv12/尺寸/帧率 → 管道，连续序列）
    assert!(
        has_seq(
            &args,
            &["-f", "rawvideo", "-pix_fmt", "nv12", "-s", "1920x1080", "-framerate", "30", "-i", &pipe_v]
        ),
        "视频输入应为 -f rawvideo -pix_fmt nv12 -s 1920x1080 -framerate 30 -i <管道>（argv: {args:?}）"
    );

    // 音频输入（f32le/48k/立体声 → 管道）
    assert!(
        has_seq(
            &args,
            &["-f", "f32le", "-ar", "48000", "-ac", "2", "-i", &pipe_a]
        ),
        "音频输入应为 -f f32le -ar 48000 -ac 2 -i <管道>"
    );

    // 显式映射（视频 + 每路音频）
    assert!(has_seq(&args, &["-map", "0:v"]), "应含 -map 0:v");
    assert!(has_seq(&args, &["-map", "1:a"]), "音频路应映射 -map 1:a");

    // 编码器与 GOP（-g = 2*fps；force_key_frames 段首 IDR，S5 冻结）
    assert!(
        has_seq(&args, &["-c:v", "h264_qsv"]),
        "编码器名应透传（-c:v h264_qsv）"
    );
    assert!(
        has_seq(&args, &["-g", "60"]),
        "30fps ⇒ -g 60（闭合 GOP，2s）"
    );
    assert!(
        has_seq(&args, &["-force_key_frames", "expr:gte(t,n_forced*5)"]),
        "应含 -force_key_frames expr:gte(t,n_forced*5)（segment_secs=5）"
    );
    assert!(
        has_seq(&args, &["-c:a", "aac", "-b:a", "192k"]),
        "音频应编码 aac 192k"
    );

    // 分段（matroska / 5s / reset_timestamps / 段清单 / 输出 pattern 收尾）
    assert!(
        has_seq(
            &args,
            &[
                "-f",
                "segment",
                "-segment_time",
                "5",
                "-reset_timestamps",
                "1",
                "-segment_format",
                "matroska"
            ]
        ),
        "分段参数应为 -f segment -segment_time 5 -reset_timestamps 1 -segment_format matroska"
    );
    assert!(
        has_seq(&args, &["-segment_list", &list.to_string_lossy()]),
        "应含 -segment_list <ffconcat 清单路径>"
    );
    assert_eq!(
        args.last().map(String::as_str),
        Some(pattern.to_string_lossy().as_ref()),
        "输出 pattern 应为最后一个参数"
    );

    // ── 用例 B：720p25、无音频（映射/音频参数随输入收缩；-g 随 fps 变化） ──
    let pipe_v2 = r"\\.\pipe\screenrec_s2_v".to_string();
    let list2 = tmp("segments2.ffconcat");
    let pattern2 = tmp("out2_%05d.mkv");
    let args2 = build_encode_args(
        &VideoPipeSpec {
            width: 1280,
            height: 720,
            fps: 25,
            pipe_path: pipe_v2.clone(),
        },
        &[],
        "h264_mf",
        5,
        &list2,
        &pattern2,
    );
    assert!(
        has_seq(
            &args2,
            &[
                "-f",
                "rawvideo",
                "-pix_fmt",
                "nv12",
                "-s",
                "1280x720",
                "-framerate",
                "25",
                "-i",
                &pipe_v2
            ]
        ),
        "无音频用例：视频输入仍应完整（1280x720@25）"
    );
    assert!(
        !has_seq(&args2, &["-f", "f32le"]),
        "无音频时不应出现 f32le 输入"
    );
    assert!(!args2.contains(&"-c:a".to_string()), "无音频时不应有 -c:a");
    assert_eq!(
        args2.iter().filter(|a| a.as_str() == "-map").count(),
        1,
        "无音频时只有 -map 0:v 一个映射"
    );
    assert!(has_seq(&args2, &["-map", "0:v"]), "无音频时仍映射 0:v");
    assert!(
        has_seq(&args2, &["-g", "50"]),
        "25fps ⇒ -g 50（-g 随 fps 计算，不得硬编码 60）"
    );
    assert!(
        has_seq(&args2, &["-c:v", "h264_mf"]),
        "编码器名应透传（-c:v h264_mf）"
    );
    assert_eq!(
        args2.last().map(String::as_str),
        Some(pattern2.to_string_lossy().as_ref()),
        "输出 pattern 应为最后一个参数"
    );
}
