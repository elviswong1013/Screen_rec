//! L2 集成测试：合并（REQ-23 的 M7 范围，specs/001-screen-recorder.md §2.5
//!（REQ-23）、§3.2（crates/sr-merge：合并（异构多轨）/REQ-23）、§3.3
//!（`merge_recordings(Vec<PathBuf>, out)` 命令）、§3.4（E_MERGE_INCOMPATIBLE）、
//! §3.5（合并输入不可读/不被 MKV 接纳 → 拒绝并说明；异构允许不得误拒）、
//! §4（REQ-23 行）、§5 M7 行、§8 M0 冻结项（锁定 ffmpeg n8.1 release））。
//!
//! 运行方式（L2 门：`#[ignore]`，默认 `cargo test` 不执行本文件用例）：
//! ```text
//! cargo test -p sr-itests --test merge -- --ignored --nocapture
//! cargo test -p sr-itests --test merge it_merge_heterogeneous_multivideo_mkv -- --ignored --nocapture   # 单用例
//! ```
//! 前置条件：交互式用户会话（§4 L2 通则）+ 仓库内锁定 ffmpeg n8.1 release 构建
//! （spec §8；含 ffprobe）；缺失 → BLOCKED（fail），不静默 skip（§4）。
//!
//! 接口契约（测试即契约，详见 docs/milestones/m7-red-report.md §3）：
//! - `sr_merge::merge::merge_recordings(MergeConfig) -> Result<MergeOutcome, MergeError>`
//!   （§3.3 `merge_recordings(Vec<PathBuf>, out)` 的管线层对应物；crate 归属按 §3.2
//!   模块表——sr-merge = 合并（异构多轨）；骨架 `pub mod merge` 的 TODO(M7) 已指名本
//!   用例）：
//! - `MergeConfig { inputs: Vec<PathBuf>, out: PathBuf }`（inputs ≥2；out 不得与任一
//!   输入同路径）。
//! - `MergeOutcome { out: PathBuf, video_tracks: usize, audio_tracks: usize }`。
//! - `MergeError` 最小变体集：`InvalidInput { input, detail }`（E_MERGE_INCOMPATIBLE：
//!   输入不可读/空/非 MKV 接纳；**异构布局不属于此错误**）、
//!   `OutputUnavailable { out, detail }`（out 与输入同路径/不可安全创建）、
//!   `SidecarFailed { detail }`（ffmpeg 执行失败）。
//! 语义（REQ-23）：≥2 产物 → 单 MKV；`-map 0 -map 1 … -c copy` 全轨保留（多视频轨
//! 并存）；**异构（分辨率/帧率/时长/轨数不同）不得误拒**；不得覆盖输入；不用
//! `-shortest`（保留完整时长=各文件按本地零点合并后的 max 而非 min/和）；失败不提交
//! （输入保留）。轨道命名/默认选择：本测试只钉「至多一个默认轨/类型」的无歧义下限。
//!
//! 手法：用锁定 ffmpeg lavfi 源**合成异构输入**（不同分辨率/帧率/时长/轨数；A=2 视频
//! +1 音频、B=1 视频+2 音频；多视频轨并存），经 `merge_recordings` 合并 → ffprobe
//! 逐轨交叉验证（轨数/每轨 codec/尺寸/声道/时长/起点）+ 整文件解码 + 时长口径（非
//! `-shortest`、非顺序拼接）+ 输入逐字节不变；负相位（非法输入/不存在输入/out==输入）
//! 断言 Err 变体与「失败不提交、输入保留」。

mod common;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use common::*;
use sr_merge::merge::{merge_recordings, MergeConfig, MergeError, MergeOutcome};

/// L2 串行门（M4 R5/M5 R6/M6 先例）：本文件用例与录制族 L2 同机运行时串行不混跑；
/// 后续新增用例沿用本门。
static GATE: Mutex<()> = Mutex::new(());

fn gate() -> std::sync::MutexGuard<'static, ()> {
    GATE.lock().unwrap_or_else(|e| e.into_inner())
}

// ─────────────────────────── 阈值（构造性保证 + scratch 实测标定） ───────────────────────────

/// 输出容器时长与 max(输入时长) 的容差（非 -shortest 判定：min 差 ≥0.8s，和差 ≥2s
/// 量级，0.5s 裕度充分；mpeg4/AAC 封装头尾抖动 ≈0.03s——scratch 实测）。
const DUR_TOL: f64 = 0.5;
/// 逐轨终点与源轨终点的容差（scratch 实测:输出与输入逐包终点差 ≤0.01s）。
const TRACK_END_TOL: f64 = 0.35;
/// 轨首包 pts 下界（AAC priming 轻微为负；-0.2s 覆盖）。
const TRACK_START_MIN: f64 = -0.2;
/// 轨首包 pts 上界（「各文件按本地零点合并」：顺序拼接的次输入轨会≈首个输入时长）。
const TRACK_START_MAX: f64 = 0.5;

// ─────────────────────────── 锁定构建 / 输出目录（M3–M6 同款） ───────────────────────────

/// 锁定 ffmpeg/ffprobe 位置：spec §8 M0 冻结的 BtbN **n8.1 release** 构建。
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
    let d = std::env::temp_dir().join(format!("sr_m7_red_merge_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("创建输出目录失败");
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
            "{what} 失败（exit={}；stderr={}）——锁定构建应支持 lavfi/testsrc2/sine/mpeg4/aac（本报告 §6 scratch 已实测）",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
}

// ─────────────────────────── ffprobe 交叉验证原语 ───────────────────────────

/// ffprobe key=value 输出（`-of default=noprint_wrappers=1:nokey=0`）。
fn ffprobe_kv(path: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&out.stdout).to_string()
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

/// 流计数（`v`/`a` 选择器；M3–M6 同款）。
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

/// 整文件可解码（ffmpeg -v error -f null -；M3–M6 同款）。
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

/// `"30000/1001"` → 29.97（`"0/0"`（无 fps 的音频轨）→ 0.0）。
fn parse_rational(s: &str) -> f64 {
    let mut it = s.trim().split('/');
    let n = it.next().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    let d = it.next().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    if d != 0.0 {
        n / d
    } else {
        n
    }
}

/// 单条轨道（ffprobe 流级事实；多轨匹配键见 `Track::key`）。
#[derive(Debug, Clone)]
struct Track {
    index: u32,
    codec: String,
    kind: String, // "video" | "audio"
    width: u32,
    height: u32,
    fps: f64,
    sample_rate: u32,
    channels: u32,
    default: bool,
}

impl Track {
    /// 异构匹配键：本测试 6 条输入轨两两可区分（构造性保证），输出按多重集合一一对应。
    fn key(&self) -> String {
        if self.kind == "video" {
            format!(
                "video/{}:{}x{}@{:.2}fps",
                self.codec, self.width, self.height, self.fps
            )
        } else {
            format!(
                "audio/{}:{}Hz/{}ch",
                self.codec, self.sample_rate, self.channels
            )
        }
    }
}

fn track_from_map(m: &HashMap<String, String>) -> Track {
    let get = |k: &str| m.get(k).cloned().unwrap_or_default();
    let num = |k: &str| get(k).parse::<u32>().unwrap_or(0);
    Track {
        index: num("index"),
        codec: get("codec_name"),
        kind: get("codec_type"),
        width: num("width"),
        height: num("height"),
        fps: parse_rational(&get("avg_frame_rate")),
        sample_rate: num("sample_rate"),
        channels: num("channels"),
        default: m
            .get("DISPOSITION:default")
            .map(|v| v == "1")
            .unwrap_or(false),
    }
}

/// 探测全部轨道（顺序=容器流序；kv 输出以 `index=` 起新块，DISPOSITION 行随块）。
fn probe_tracks(path: &Path) -> Vec<Track> {
    let s = ffprobe_kv(
        path,
        &[
            "-show_entries",
            "stream=index,codec_name,codec_type,width,height,avg_frame_rate,sample_rate,channels:stream_disposition=default",
            "-of",
            "default=noprint_wrappers=1:nokey=0",
        ],
    );
    let mut out: Vec<Track> = Vec::new();
    let mut cur: Option<HashMap<String, String>> = None;
    for line in s.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if k == "index" {
            if let Some(prev) = cur.take() {
                out.push(track_from_map(&prev));
            }
            cur = Some(HashMap::new());
        }
        cur.get_or_insert_with(HashMap::new)
            .insert(k.to_string(), v.trim().to_string());
    }
    if let Some(prev) = cur.take() {
        out.push(track_from_map(&prev));
    }
    out
}

/// 轨首/末包时间跨度（pts_time/duration_time 行对；MKV 无每轨 duration 头字段，
/// 以包终点代替——scratch 实测与源轨差 ≤0.01s）。
fn stream_span(path: &Path, selector: &str) -> (f64, f64) {
    let s = ffprobe_kv(
        path,
        &[
            "-select_streams",
            selector,
            "-show_entries",
            "packet=pts_time,duration_time",
            "-of",
            "default=noprint_wrappers=1:nokey=0",
        ],
    );
    let mut start = f64::INFINITY;
    let mut end = f64::NEG_INFINITY;
    let mut pending: Option<f64> = None;
    for line in s.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match k {
            "pts_time" => {
                if let Ok(p) = v.trim().parse::<f64>() {
                    start = start.min(p);
                    pending = Some(p);
                }
            }
            "duration_time" => {
                if let (Some(p), Ok(d)) = (pending.take(), v.trim().parse::<f64>()) {
                    end = end.max(p + d);
                }
            }
            _ => {}
        }
    }
    if let Some(p) = pending {
        end = end.max(p);
    }
    assert!(
        start.is_finite() && end.is_finite(),
        "流 {selector} 无包（{}；空轨/未封口？）",
        path.display()
    );
    (start, end)
}

/// 全部轨道 + 各自首/末包时间（按同类型序号构造 `v:N`/`a:N` 选择器）。
fn probe_tracks_with_spans(path: &Path) -> Vec<(Track, f64, f64)> {
    let tracks = probe_tracks(path);
    let (mut vn, mut an) = (0usize, 0usize);
    tracks
        .into_iter()
        .map(|t| {
            let sel = if t.kind == "video" {
                let s = format!("v:{vn}");
                vn += 1;
                s
            } else {
                let s = format!("a:{an}");
                an += 1;
                s
            };
            let (start, end) = stream_span(path, &sel);
            (t, start, end)
        })
        .collect()
}

// ─────────────────────────── 异构输入合成（锁定构建；lavfi，无需采集） ───────────────────────────

/// 合成两个异构 MKV 输入（scratch 已实测轨布局/时长/per-stream 终点）：
/// - A（≈3.0s）：视频 640x360@30 + 320x180@15（**多视频轨**）+ 音频 48kHz/2ch；
/// - B（≈2.0s）：视频 1280x720@25 + 音频 44.1kHz/2ch + 48kHz/1ch（**多音轨、不同
///   采样率/声道数、不同时长**）。
/// 两输入的分辨率/帧率/时长/轨数全不同（异构面最大化；6 条轨匹配键两两可区分）。
fn synth_inputs(dir: &Path) -> (PathBuf, PathBuf) {
    let a = dir.join("input_a_2video1audio.mkv");
    let b = dir.join("input_b_1video2audio.mkv");
    let a_s = a.to_string_lossy().to_string();
    let b_s = b.to_string_lossy().to_string();

    run_ffmpeg(
        &[
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=30:duration=3",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=320x180:rate=15:duration=3",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=3",
            "-map",
            "0:v",
            "-map",
            "1:v",
            "-map",
            "2:a",
            "-c:v",
            "mpeg4",
            "-q:v",
            "5",
            "-c:a",
            "aac",
            "-b:a",
            "96k",
            "-ac",
            "2",
            "-f",
            "matroska",
            a_s.as_str(),
        ],
        "合成异构输入 A（2 视频轨 + 1 音轨）",
    );
    run_ffmpeg(
        &[
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1280x720:rate=25:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880:sample_rate=44100:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=1200:sample_rate=48000:duration=2",
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-map",
            "2:a",
            "-c:v",
            "mpeg4",
            "-q:v",
            "5",
            "-c:a",
            "aac",
            "-b:a",
            "96k",
            "-ac:a:0",
            "2",
            "-ac:a:1",
            "1",
            "-f",
            "matroska",
            b_s.as_str(),
        ],
        "合成异构输入 B（1 视频轨 + 2 音轨）",
    );
    (a, b)
}

// ─────────────────────────── 用例 ───────────────────────────

/// REQ-23（§4/§5 点名）：合并 ≥2 产物 → 单 MKV。核心语义：异构输入（不同分辨率/
/// 帧率/时长/轨数）**不得误拒**；全轨保留（多视频轨并存）；每轨与源逐轨一致；时长
/// 完整（非 `-shortest`、非顺序拼接——各文件按本地零点）；输入不被修改/覆盖；失败
/// 或非法输入时拒绝且输入保留（含 out 与输入同路径的禁覆盖校验）。
#[test]
#[ignore = "L2：需交互式会话 + 锁定 ffmpeg n8.1（specs §4）。显式运行：cargo test -p sr-itests --test merge -- --ignored --nocapture"]
fn it_merge_heterogeneous_multivideo_mkv() {
    let _g = gate();
    require_interactive_session();
    let _os = record_os_build();

    let dir = fresh_out_dir("heterogeneous");
    // ── 合成异构输入（无采集依赖：锁定构建 lavfi） ──
    let (in_a, in_b) = synth_inputs(&dir);
    let a_before = std::fs::read(&in_a).expect("读取输入 A 字节");
    let b_before = std::fs::read(&in_b).expect("读取输入 B 字节");

    // ── 合成前提自检（防测试自身漂移）：异构面（轨数/时长）成立且 6 轨键两两可区分 ──
    let a_tracks = probe_tracks_with_spans(&in_a);
    let b_tracks = probe_tracks_with_spans(&in_b);
    let (a_v, a_a) = (
        a_tracks
            .iter()
            .filter(|(t, _, _)| t.kind == "video")
            .count(),
        a_tracks
            .iter()
            .filter(|(t, _, _)| t.kind == "audio")
            .count(),
    );
    let (b_v, b_a) = (
        b_tracks
            .iter()
            .filter(|(t, _, _)| t.kind == "video")
            .count(),
        b_tracks
            .iter()
            .filter(|(t, _, _)| t.kind == "audio")
            .count(),
    );
    assert_eq!(
        (a_v, a_a),
        (2, 1),
        "合成输入 A 应为 2 视频轨 + 1 音轨（异构前提）"
    );
    assert_eq!(
        (b_v, b_a),
        (1, 2),
        "合成输入 B 应为 1 视频轨 + 2 音轨（异构前提）"
    );
    let dur_a = probe_duration(&in_a);
    let dur_b = probe_duration(&in_b);
    assert!(
        (dur_a - 3.0).abs() <= 0.5 && (dur_b - 2.0).abs() <= 0.5,
        "合成输入时长应≈3s/2s（得到 {dur_a:.3}/{dur_b:.3}）"
    );
    assert!(
        (dur_a - dur_b).abs() >= 0.8,
        "异构时长前提：两输入时长应显著不同（得到 {dur_a:.3}/{dur_b:.3}）"
    );
    let mut in_keys: HashSet<String> = HashSet::new();
    for (t, _, _) in a_tracks.iter().chain(b_tracks.iter()) {
        assert!(
            in_keys.insert(t.key()),
            "合成输入轨道键应两两可区分（{} 重复）",
            t.key()
        );
    }
    for (label, tracks) in [("A", &a_tracks), ("B", &b_tracks)] {
        for (t, s, e) in tracks {
            println!(
                "[merge] 输入{label} 轨 #{} {}（起点 {s:.3}s / 终点 {e:.3}s）",
                t.index,
                t.key()
            );
        }
    }

    // ── 正向：≥2 产物 → 单 MKV（异构不得误拒） ──
    let out = dir.join("merged_heterogeneous.mkv");
    assert!(!out.exists(), "输出路径应为全新（防误读旧产物）");
    let outcome: MergeOutcome = merge_recordings(MergeConfig {
        inputs: vec![in_a.clone(), in_b.clone()],
        out: out.clone(),
    })
    .unwrap_or_else(|e| {
        panic!("REQ-23：异构输入（分辨率/帧率/时长/轨数不同）不得误拒（得到 Err {e:?}）")
    });
    assert_eq!(outcome.out, out, "MergeOutcome.out 应为请求的输出路径");
    assert_eq!(
        outcome.video_tracks, 3,
        "成品视频轨总数 = 2+1（全轨保留：多视频轨并存）"
    );
    assert_eq!(outcome.audio_tracks, 3, "成品音频轨总数 = 1+2（全轨保留）");
    println!(
        "[merge] 合并完成：video_tracks={} audio_tracks={} → {}",
        outcome.video_tracks,
        outcome.audio_tracks,
        out.display()
    );

    // ── 输出：单个可播 MKV ──
    assert!(out.exists(), "成品应存在（{}）", out.display());
    assert!(
        out.extension()
            .map_or(false, |e| e.to_string_lossy() == "mkv"),
        "成品应为 MKV（{}）",
        out.display()
    );
    assert!(decode_clean(&out), "REQ-23：合并成品应整文件可解码（可播）");

    // ── 流布局与输入一致（轨数 + 逐轨 codec/尺寸/声道/时长/起点） ──
    let out_tracks = probe_tracks_with_spans(&out);
    let (n_v, n_a) = (probe_stream_count(&out, "v"), probe_stream_count(&out, "a"));
    assert_eq!(
        (n_v, n_a),
        (3, 3),
        "成品轨布局应为输入之和（3 视频 + 3 音频；得到 {n_v}+{n_a}）"
    );
    assert!(n_v >= 2, "REQ-23：多视频轨并存（得到 {n_v} 条视频轨）");
    let mut expected: HashMap<String, (&'static str, f64, f64)> = HashMap::new();
    for (t, s, e) in &a_tracks {
        expected.insert(t.key(), ("A", *s, *e));
    }
    for (t, s, e) in &b_tracks {
        expected.insert(t.key(), ("B", *s, *e));
    }
    let mut seen: HashSet<String> = HashSet::new();
    for (t, s, e) in &out_tracks {
        let key = t.key();
        let (src, src_start, src_end) = expected.get(&key).unwrap_or_else(|| {
            panic!("REQ-23：输出轨道在输入中无对应（{key}；-map 全轨保留？——输出 {out_tracks:?}）")
        });
        assert!(
            seen.insert(key.clone()),
            "REQ-23：输出出现多余/重复轨道（{key}）"
        );
        println!(
            "[merge] 输出轨 #{} {key} ← 输入{src}（起点 {s:.3}s vs {src_start:.3}s；终点 {e:.3}s vs {src_end:.3}s）",
            t.index
        );
        assert!(
            (e - src_end).abs() <= TRACK_END_TOL,
            "REQ-23：轨 {key} 时长应随源（输出终点 {e:.3}s vs 输入{src} {src_end:.3}s，容差 {TRACK_END_TOL}）"
        );
        assert!(
            *s >= TRACK_START_MIN && *s <= TRACK_START_MAX,
            "REQ-23：各文件按本地零点合并（轨 {key} 起点 {s:.3}s 应≈0：不得顺序偏移）"
        );
    }
    assert_eq!(
        seen.len(),
        out_tracks.len(),
        "成品轨道应全部可与输入一一对应"
    );
    // 默认选择规则无歧义下限（REQ-23「默认选择规则明确」；不钉具体哪条为默认）。
    let v_def = out_tracks
        .iter()
        .filter(|(t, _, _)| t.kind == "video" && t.default)
        .count();
    let a_def = out_tracks
        .iter()
        .filter(|(t, _, _)| t.kind == "audio" && t.default)
        .count();
    assert!(
        v_def <= 1 && a_def <= 1,
        "默认轨选择不得歧义（video default={v_def}，audio default={a_def}）"
    );

    // ── 时长完整：非 -shortest（≈min）且非顺序拼接（≈和） ──
    let dur_out = probe_duration(&out);
    let dur_max = dur_a.max(dur_b);
    let dur_min = dur_a.min(dur_b);
    println!("[merge] 时长：输入 A {dur_a:.3}s / B {dur_b:.3}s / 输出 {dur_out:.3}s（期望≈max {dur_max:.3}s）");
    assert!(
        (dur_out - dur_max).abs() <= DUR_TOL,
        "REQ-23：不得 -shortest（截断到最短输入 {dur_min:.3}s）也不得顺序拼接（时长和 {:.3}s）；得到 {dur_out:.3}s vs max {dur_max:.3}s",
        dur_a + dur_b
    );
    assert!(
        dur_out >= dur_min + 0.8,
        "REQ-23：输出时长应保留完整时长（得到 {dur_out:.3}s，最短输入 {dur_min:.3}s）"
    );

    // ── 输入未被修改/覆盖（逐字节） ──
    assert_ne!(out, in_a, "成品路径不得与输入 A 相同");
    assert_ne!(out, in_b, "成品路径不得与输入 B 相同");
    assert_eq!(
        std::fs::read(&in_a).unwrap(),
        a_before,
        "REQ-23：合并不得修改/覆盖输入 A"
    );
    assert_eq!(
        std::fs::read(&in_b).unwrap(),
        b_before,
        "REQ-23：合并不得修改/覆盖输入 B"
    );

    // ── 负相位：失败/非法输入 → 拒绝 + 不提交成品 + 输入保留 ──
    // (1) 非法输入（非 MKV 内容，确定性载荷）：E_MERGE_INCOMPATIBLE 语义（§3.5）
    let bad = dir.join("input_bad_not_matroska.mkv");
    std::fs::write(
        &bad,
        b"SR-M7-RED deterministic non-Matroska payload (must be rejected)\n",
    )
    .expect("写入坏输入");
    let bad_before = std::fs::read(&bad).expect("读取坏输入字节");
    let out_bad = dir.join("merged_with_bad_input.mkv");
    let err = merge_recordings(MergeConfig {
        inputs: vec![in_a.clone(), bad.clone()],
        out: out_bad.clone(),
    })
    .expect_err("REQ-23：不可读/非 MKV 输入必须拒绝（E_MERGE_INCOMPATIBLE），不得静默成功");
    assert!(
        matches!(err, MergeError::InvalidInput { .. }),
        "非法输入应报 MergeError::InvalidInput（得到 {err:?}）"
    );
    assert!(
        !out_bad.exists(),
        "失败不得提交成品（{} 不应存在；输入保留）",
        out_bad.display()
    );
    assert_eq!(
        std::fs::read(&bad).unwrap(),
        bad_before,
        "失败时坏输入文件不得被修改"
    );
    assert_eq!(
        std::fs::read(&in_a).unwrap(),
        a_before,
        "失败时有效输入不得被修改"
    );

    // (2) 不可读（不存在）输入：同样拒绝且有效输入保留
    let missing = dir.join("input_missing.mkv");
    let out_missing = dir.join("merged_with_missing_input.mkv");
    let err = merge_recordings(MergeConfig {
        inputs: vec![in_a.clone(), missing.clone()],
        out: out_missing.clone(),
    })
    .expect_err("REQ-23：不存在的输入必须拒绝");
    assert!(
        matches!(err, MergeError::InvalidInput { .. }),
        "不存在输入应报 MergeError::InvalidInput（得到 {err:?}）"
    );
    assert!(
        !out_missing.exists(),
        "失败不得提交成品（{} 不应存在）",
        out_missing.display()
    );
    assert!(!missing.exists(), "拒绝后不存在的输入不得被创建");
    assert_eq!(
        std::fs::read(&in_a).unwrap(),
        a_before,
        "失败时有效输入不得被修改"
    );

    // (3) out 与输入同路径：必须拒绝（不得覆盖输入）
    let err = merge_recordings(MergeConfig {
        inputs: vec![in_a.clone(), in_b.clone()],
        out: in_a.clone(),
    })
    .expect_err("REQ-23：out 与输入相同必须拒绝（不得覆盖输入）");
    assert!(
        matches!(err, MergeError::OutputUnavailable { .. }),
        "out==输入 应报 MergeError::OutputUnavailable（得到 {err:?}）"
    );
    assert_eq!(
        std::fs::read(&in_a).unwrap(),
        a_before,
        "out==输入 被拒后输入 A 必须逐字节不变"
    );
    assert_eq!(
        std::fs::read(&in_b).unwrap(),
        b_before,
        "out==输入 被拒后输入 B 必须逐字节不变"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
