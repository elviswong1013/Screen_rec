//! 能力探测与降级（M8/REQ-28）：分层探测（OS build / D4 编码器链实际初始化 /
//! process loopback 试激活 / 麦克风隐私 / WGC 授权 / 画布上限）+ 可注入故障钩子。
//!
//! 契约（测试即契约，docs/milestones/m8-red-report.md §3.1 + probe.rs）：
//! - 三类权限/激活面（[`ProbeCapabilities::loopback`]/[`ProbeCapabilities::mic`]/
//!   [`ProbeCapabilities::wgc`]）各自归类为 [`LayerStatus`] 六态——超时 ≠ 权限拒绝 ≠
//!   取消（REQ-28「各自处理取消/超时」）；
//! - `selected_encoder` = D4 链序中**首个 init_ok**；全败 = `None`（调用方明确报
//!   E_ENCODER_UNAVAILABLE，§9 T1，不静默降级）；
//! - [`probe_capabilities_injected`]：`Some(_)` 的层**整体以注入结果代替真实探测**
//!   （`None` = 真实探测）；用于矩阵化降级断言与 UI 预演。

use std::process::Command;
use std::time::Duration;

use sr_audio::wasapi::{
    capture_microphone, list_capture_devices, process_created_unix_ms, CaptureError, LoopbackStream,
};

/// 层归类（REQ-28：拒绝/超时/取消/不可用/失败可区分；不得混淆为「权限问题」）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerStatus {
    /// 探测通过（可用）。
    Ok,
    /// 权限/授权被明确拒绝（隐私拒绝、WGC 拒绝）。
    Denied,
    /// 探测超时（≠ 拒绝）。
    Timeout,
    /// 用户/系统取消（≠ 拒绝）。
    Canceled,
    /// 环境不可用（无设备/无端点；≠ 拒绝）。
    Unavailable,
    /// 探测执行失败（非权限、非环境；可诊断）。
    Failed,
}

/// 单层探测报告（状态 + 可诊断明细）。
#[derive(Clone, Debug)]
pub struct LayerReport {
    pub status: LayerStatus,
    pub detail: String,
}

/// 单编码器探测结果（D4 链序候选；`detail` 不得为空）。
#[derive(Clone, Debug)]
pub struct EncoderProbe {
    pub name: String,
    pub init_ok: bool,
    pub detail: String,
}

/// 画布上限（D6 固定画布的编码器实测安全上界）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CanvasLimits {
    pub max_width: u32,
    pub max_height: u32,
}

/// 分层探测结果（REQ-28）。
#[derive(Clone, Debug)]
pub struct ProbeCapabilities {
    /// OS build（如 `"10.0.26200.9711"`）。
    pub os_build: String,
    /// 最低支持阈值：Win10 2004（build ≥ 19041）；低于 → 明确不支持（降级路径）。
    pub os_supported: bool,
    /// D4 链序逐候选实际初始化结果。
    pub encoders: Vec<EncoderProbe>,
    /// 链序首个 init_ok；全败 → None（§9 T1）。
    pub selected_encoder: Option<String>,
    /// 音频激活类（process loopback 试激活；超时/取消独立归类）。
    pub loopback: LayerReport,
    /// 麦克风隐私类。
    pub mic: LayerReport,
    /// WGC 授权类。
    pub wgc: LayerReport,
    /// 画布上限。
    pub canvas_limits: CanvasLimits,
}

/// 可注入故障钩子（测试/预演）：`Some(_)` 的层整体以注入结果代替真实探测。
#[derive(Clone, Debug, Default)]
pub struct ProbeInjections {
    pub os_build: Option<String>,
    /// 链序整体替换（name, init_ok）。
    pub encoder_init: Option<Vec<(String, bool)>>,
    pub loopback: Option<LayerStatus>,
    pub mic: Option<LayerStatus>,
    pub wgc: Option<LayerStatus>,
    pub canvas_limits: Option<CanvasLimits>,
}

/// D4 定稿编码器链序（S2 矩阵裁决：NVENC→QSV→AMF→h264_mf）。
const D4_CHAIN: [&str; 4] = ["h264_nvenc", "h264_qsv", "h264_amf", "h264_mf"];

/// 画布上限保守值（8K；D6 固定画布策略下不触及编码器实测上界）。
const DEFAULT_CANVAS_LIMITS: CanvasLimits = CanvasLimits {
    max_width: 7680,
    max_height: 4320,
};

/// 最低支持的 OS build 锚点：Win10 2004（19041；§1 目标环境 Win10 2004+）。
const MIN_SUPPORTED_BUILD: u32 = 19041;

/// 真实 OS build（`cmd /c ver` → `Microsoft Windows [Version 10.0.<build>.<ubr>]`）。
fn probe_os_build() -> String {
    let out = Command::new("cmd").args(["/c", "ver"]).output();
    let text = match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => String::new(),
    };
    text.split("Version ")
        .nth(1)
        .and_then(|s| s.split(']').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// OS build 第三段（build 号；解析失败 = 0 → 判不支持）。
fn build_number(os_build: &str) -> u32 {
    os_build
        .split('.')
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn os_supported(os_build: &str) -> bool {
    build_number(os_build) >= MIN_SUPPORTED_BUILD
}

/// 单编码器实际初始化（D4 语义：lavfi 短编码实际初始化，非仅枚举）。
fn real_encoder_probe(ffmpeg: &std::path::Path, name: &str) -> EncoderProbe {
    let (init_ok, detail) = crate::sidecar::probe_encoder_detail(ffmpeg, name);
    EncoderProbe {
        name: name.to_string(),
        init_ok,
        detail,
    }
}

/// process loopback 试激活（REQ-28：激活后立即停止；失败按类归因）。
fn probe_loopback() -> LayerReport {
    let created = process_created_unix_ms(std::process::id()).unwrap_or(0);
    match LoopbackStream::start(std::process::id(), created) {
        Ok(stream) => {
            stream.stop();
            LayerReport {
                status: LayerStatus::Ok,
                detail: "process loopback 试激活成功（激活即停）".into(),
            }
        }
        Err(CaptureError::LoopbackUnavailable { detail }) => LayerReport {
            status: LayerStatus::Unavailable,
            detail: format!("环回不可用：{detail}"),
        },
        Err(e) => LayerReport {
            status: LayerStatus::Failed,
            detail: format!("环回试激活失败：{e:?}"),
        },
    }
}

/// 麦克风隐私类（枚举采集设备；设备存在时短试录以区分「隐私拒绝」与「无设备」）。
fn probe_mic() -> LayerReport {
    match list_capture_devices() {
        Ok(devices) if devices.is_empty() => LayerReport {
            status: LayerStatus::Unavailable,
            detail: "无采集（麦克风）设备".into(),
        },
        Ok(devices) => {
            let dev = devices.iter().find(|d| d.is_default).unwrap_or(&devices[0]);
            match capture_microphone(&dev.id, Duration::from_millis(200)) {
                Ok(_) => LayerReport {
                    status: LayerStatus::Ok,
                    detail: format!("试录成功（{}，~200ms）", dev.name),
                },
                Err(CaptureError::MicUnavailable { detail }) => LayerReport {
                    status: LayerStatus::Denied,
                    detail: format!("采集设备不可打开（隐私/占用）：{detail}"),
                },
                Err(e) => LayerReport {
                    status: LayerStatus::Failed,
                    detail: format!("试录失败：{e:?}"),
                },
            }
        }
        Err(e) => LayerReport {
            status: LayerStatus::Failed,
            detail: format!("枚举采集设备失败：{e:?}"),
        },
    }
}

/// WGC 授权类（Monitor 0 最小会话试激活后立即停止）。
fn probe_wgc() -> LayerReport {
    use sr_capture::wgc::{self, LatestFrame, StreamStartError};
    match wgc::start_monitor_stream(0, 320, 180, 15, false, LatestFrame::new()) {
        Ok(control) => {
            let _ = control.stop();
            LayerReport {
                status: LayerStatus::Ok,
                detail: "WGC 试激活成功（Monitor 0，320x180@15，即时停止）".into(),
            }
        }
        Err(StreamStartError::ConsentDenied) => LayerReport {
            status: LayerStatus::Denied,
            detail: "WGC 授权被拒（E_CONSENT_DENIED）".into(),
        },
        Err(StreamStartError::TargetInvalid { detail }) => LayerReport {
            status: LayerStatus::Unavailable,
            detail: format!("显示器 0 不可用：{detail}"),
        },
        Err(StreamStartError::Failed { detail }) => LayerReport {
            status: LayerStatus::Failed,
            detail,
        },
    }
}

/// 真实分层探测（REQ-28；无设备/无权限等环境缺失按状态归类，不 panic）。
pub fn probe_capabilities() -> ProbeCapabilities {
    probe_capabilities_injected(&ProbeInjections::default())
}

/// 注入式探测：`Some(_)` 的层整体以注入结果代替真实探测；`None` = 真实探测。
pub fn probe_capabilities_injected(injections: &ProbeInjections) -> ProbeCapabilities {
    let os_build = match &injections.os_build {
        Some(b) => b.clone(),
        None => probe_os_build(),
    };
    let encoders: Vec<EncoderProbe> = match &injections.encoder_init {
        Some(list) => list
            .iter()
            .map(|(name, ok)| EncoderProbe {
                name: name.clone(),
                init_ok: *ok,
                detail: "注入替身（链序整体替换）".into(),
            })
            .collect(),
        None => match crate::sidecar::ffmpeg_exe() {
            Ok(ffmpeg) => D4_CHAIN
                .iter()
                .map(|name| real_encoder_probe(&ffmpeg, name))
                .collect(),
            Err(e) => D4_CHAIN
                .iter()
                .map(|name| EncoderProbe {
                    name: (*name).to_string(),
                    init_ok: false,
                    detail: format!("ffmpeg 不可用：{e}"),
                })
                .collect(),
        },
    };
    let selected_encoder = encoders.iter().find(|e| e.init_ok).map(|e| e.name.clone());
    let injected = |s: LayerStatus| LayerReport {
        status: s,
        detail: "注入替身".into(),
    };
    ProbeCapabilities {
        os_supported: os_supported(&os_build),
        os_build,
        encoders,
        selected_encoder,
        loopback: injections
            .loopback
            .map(injected)
            .unwrap_or_else(probe_loopback),
        mic: injections.mic.map(injected).unwrap_or_else(probe_mic),
        wgc: injections.wgc.map(injected).unwrap_or_else(probe_wgc),
        canvas_limits: injections.canvas_limits.unwrap_or(DEFAULT_CANVAS_LIMITS),
    }
}
