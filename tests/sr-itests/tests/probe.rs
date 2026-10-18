//! L2 集成测试：能力探测与降级（REQ-28 的 M8 范围，specs/001-screen-recorder.md
//! §2.7（REQ-28）、§3.3（probe_capabilities 契约）、§4（REQ-28 行）、§5 M8 行）。
//!
//! 运行方式（L2 门：`#[ignore]`，默认 `cargo test` 不执行本文件用例）：
//! ```text
//! cargo test -p sr-itests --test probe -- --ignored --nocapture
//! cargo test -p sr-itests --test probe it_probe_permission_matrix_injected -- --ignored --nocapture
//! ```
//! 前置条件：交互式用户会话（§4 L2 通则；真实探测含 WGC/环回/设备面）；缺失 → BLOCKED。
//!
//! 接口契约（测试即契约，详见 docs/milestones/m8-red-report.md §3.1）：
//! - `sr_pipeline::probe::probe_capabilities() -> ProbeCapabilities`：分层探测（OS build /
//!   D4 编码器链**实际初始化** / loopback 试激活 / 麦克风隐私 / WGC 授权 / 画布上限）。
//! - `sr_pipeline::probe::probe_capabilities_injected(&ProbeInjections) -> ProbeCapabilities`：
//!   **可注入故障钩子**（测试/预演）：`Some(_)` 的层整体以注入结果代替真实探测（`None` =
//!   真实探测）；用于矩阵化「OS 过旧 / 编码器逐个失败 / 激活失败/超时/取消/拒绝」及降级断言。
//! - `LayerStatus { Ok, Denied, Timeout, Canceled, Unavailable, Failed }` + `LayerReport
//!   { status, detail }`：三类权限/激活面（loopback/mic/wgc）**各自**给出归类与明细——
//!   超时/取消不得归因为权限拒绝（REQ-28「各自处理取消/超时」）。
//! - `selected_encoder` = D4 链序中**首个 init_ok**；全部失败 = `None`（由调用方明确报
//!   E_ENCODER_UNAVAILABLE，§9 T1，不静默降级）。
//!
//! 手法：注入矩阵（确定性）+ 真实探测 smoke（结构性不变量：链序/一致性/环境前提）。

mod common;

use std::sync::Mutex;

use common::*;
use sr_pipeline::probe::{
    probe_capabilities, probe_capabilities_injected, CanvasLimits, EncoderProbe, LayerReport,
    LayerStatus, ProbeCapabilities, ProbeInjections,
};

/// L2 串行门（M4–M7 先例）。
static GATE: Mutex<()> = Mutex::new(());

fn gate() -> std::sync::MutexGuard<'static, ()> {
    GATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// D4 定稿编码器链序（S2 矩阵裁决：NVENC→QSV→AMF→h264_mf）。
const D4_CHAIN: [&str; 4] = ["h264_nvenc", "h264_qsv", "h264_amf", "h264_mf"];

fn status_of(r: &LayerReport) -> LayerStatus {
    r.status
}

fn all_injected(
    os_build: &str,
    encoders: Vec<(&str, bool)>,
    loopback: LayerStatus,
    mic: LayerStatus,
    wgc: LayerStatus,
) -> ProbeInjections {
    ProbeInjections {
        os_build: Some(os_build.to_string()),
        encoder_init: Some(
            encoders
                .into_iter()
                .map(|(n, ok)| (n.to_string(), ok))
                .collect(),
        ),
        loopback: Some(loopback),
        mic: Some(mic),
        wgc: Some(wgc),
        canvas_limits: Some(CanvasLimits {
            max_width: 4096,
            max_height: 2160,
        }),
    }
}

fn assert_encoder_shape(caps: &ProbeCapabilities, label: &str) {
    assert_eq!(
        caps.encoders.len(),
        D4_CHAIN.len(),
        "{label}：编码器链应含 D4 四个候选（得到 {}）",
        caps.encoders.len()
    );
    for (i, probe) in caps.encoders.iter().enumerate() {
        assert_eq!(
            probe.name, D4_CHAIN[i],
            "{label}：编码器链序应与 D4 定稿一致（位置 {i}）"
        );
        assert!(
            !probe.detail.is_empty(),
            "{label}：{} 的探测明细不得为空",
            probe.name
        );
    }
}

#[test]
#[ignore = "L2：需交互式会话（真实探测面含 WGC/设备/编码器）。显式运行：cargo test -p sr-itests --test probe -- --ignored --nocapture"]
fn it_probe_permission_matrix_injected() {
    let _g = gate();
    require_interactive_session();
    let _os = record_os_build();

    // ── 矩阵 1：OS 过旧 + 编码器全败 + 三类激活面分别 Timeout/Denied/Canceled ──
    // REQ-28：超时不得归因为权限拒绝；取消与拒绝可区分；全败不 panic（降级=selected None）。
    let caps = probe_capabilities_injected(&all_injected(
        "10.0.17763",
        vec![
            ("h264_nvenc", false),
            ("h264_qsv", false),
            ("h264_amf", false),
            ("h264_mf", false),
        ],
        LayerStatus::Timeout,
        LayerStatus::Denied,
        LayerStatus::Canceled,
    ));
    assert_eq!(caps.os_build, "10.0.17763", "注入的 OS build 应原样回传");
    assert!(
        !caps.os_supported,
        "过旧 OS（10.0.17763）应判不支持（降级路径）"
    );
    assert_encoder_shape(&caps, "矩阵1");
    assert!(
        caps.encoders.iter().all(|e| !e.init_ok),
        "矩阵1：注入后应全部初始化失败"
    );
    assert_eq!(
        caps.selected_encoder, None,
        "矩阵1：编码器全败时 selected_encoder 应为 None（调用方明确报错，§9 T1）"
    );
    assert_eq!(
        status_of(&caps.loopback),
        LayerStatus::Timeout,
        "环回超时应归类 Timeout（不得归因权限拒绝）"
    );
    assert_eq!(
        status_of(&caps.mic),
        LayerStatus::Denied,
        "麦克风隐私拒绝应归类 Denied"
    );
    assert_eq!(
        status_of(&caps.wgc),
        LayerStatus::Canceled,
        "WGC 取消应归类 Canceled（与 Denied 可区分）"
    );
    for (label, r) in [
        ("loopback", &caps.loopback),
        ("mic", &caps.mic),
        ("wgc", &caps.wgc),
    ] {
        assert!(
            !r.detail.is_empty(),
            "矩阵1：{label} 明细不得为空（可诊断）"
        );
    }
    assert_eq!(
        caps.canvas_limits,
        CanvasLimits {
            max_width: 4096,
            max_height: 2160
        },
        "注入的画布上限应原样回传"
    );

    // ── 矩阵 2：链序回退（首个 init_ok 胜出；全链顺序不变） ──
    let caps = probe_capabilities_injected(&all_injected(
        "10.0.26200.9711",
        vec![
            ("h264_nvenc", false),
            ("h264_qsv", false),
            ("h264_amf", false),
            ("h264_mf", true),
        ],
        LayerStatus::Ok,
        LayerStatus::Unavailable,
        LayerStatus::Ok,
    ));
    assert!(caps.os_supported, "当前级 OS build 应判支持");
    assert_encoder_shape(&caps, "矩阵2");
    assert_eq!(
        caps.selected_encoder.as_deref(),
        Some("h264_mf"),
        "矩阵2：仅链末可用时应选 h264_mf"
    );
    assert_eq!(
        status_of(&caps.mic),
        LayerStatus::Unavailable,
        "无麦克风设备应归类 Unavailable（≠ Denied）"
    );

    let caps = probe_capabilities_injected(&all_injected(
        "10.0.26200.9711",
        vec![
            ("h264_nvenc", true),
            ("h264_qsv", true),
            ("h264_amf", false),
            ("h264_mf", true),
        ],
        LayerStatus::Ok,
        LayerStatus::Ok,
        LayerStatus::Ok,
    ));
    assert_eq!(
        caps.selected_encoder.as_deref(),
        Some("h264_nvenc"),
        "链序优先：nvenc 可用时应选链首"
    );

    // ── 矩阵 3：四类状态可区分（Ok/Failed 补充）+ 注入覆盖整层 ──
    let caps = probe_capabilities_injected(&all_injected(
        "10.0.26200.9711",
        vec![
            ("h264_nvenc", true),
            ("h264_qsv", false),
            ("h264_amf", false),
            ("h264_mf", true),
        ],
        LayerStatus::Failed,
        LayerStatus::Ok,
        LayerStatus::Timeout,
    ));
    assert_eq!(
        status_of(&caps.loopback),
        LayerStatus::Failed,
        "环回失败应归类 Failed"
    );
    assert_eq!(status_of(&caps.mic), LayerStatus::Ok, "麦克风可用应归类 Ok");
    assert_eq!(
        status_of(&caps.wgc),
        LayerStatus::Timeout,
        "WGC 超时应归类 Timeout"
    );
    assert_eq!(caps.selected_encoder.as_deref(), Some("h264_nvenc"));

    // ── 真实探测 smoke（无注入）：结构性不变量（REQ-28 分层探测实际可达） ──
    let real = probe_capabilities();
    println!(
        "[probe] 真实探测：os={} supported={} selected={:?} loopback={:?} mic={:?} wgc={:?} canvas={}x{}",
        real.os_build,
        real.os_supported,
        real.selected_encoder,
        real.loopback.status,
        real.mic.status,
        real.wgc.status,
        real.canvas_limits.max_width,
        real.canvas_limits.max_height
    );
    assert!(
        !real.os_build.is_empty(),
        "真实探测应返回 OS build（REQ-28 分层探测）"
    );
    assert!(
        real.os_build.chars().any(|c| c.is_ascii_digit()),
        "OS build 应含版本号（得到 {:?}）",
        real.os_build
    );
    assert!(
        real.os_supported,
        "本机为 Windows 11（26200）应判 OS 支持（§9 T1 环境前提）"
    );
    assert_encoder_shape(&real, "真实探测");
    let first_ok = real
        .encoders
        .iter()
        .find(|e| e.init_ok)
        .map(|e| e.name.clone());
    assert_eq!(
        real.selected_encoder, first_ok,
        "真实探测：selected_encoder 应等于链序首个 init_ok"
    );
    if real.selected_encoder.is_none() {
        blocked("D4 编码器链全部初始化失败（§9 T1 环境缺失；L2 计 BLOCKED）");
    }
    assert!(
        real.canvas_limits.max_width > 0 && real.canvas_limits.max_height > 0,
        "画布上限应为正（REQ-28）"
    );
    for (label, r) in [
        ("loopback", &real.loopback),
        ("mic", &real.mic),
        ("wgc", &real.wgc),
    ] {
        assert!(!r.detail.is_empty(), "真实探测：{label} 明细不得为空");
    }
    for (label, r) in [
        ("loopback", &real.loopback),
        ("mic", &real.mic),
        ("wgc", &real.wgc),
    ] {
        match r.status {
            LayerStatus::Ok
            | LayerStatus::Denied
            | LayerStatus::Timeout
            | LayerStatus::Canceled
            | LayerStatus::Unavailable
            | LayerStatus::Failed => {}
        }
        println!("[probe] {label}: {:?} — {}", r.status, r.detail);
    }
}
