//! §3.3 Tauri 命令接线（M9）：把既有 crate（sr-enum / sr-pipeline / sr-shot / sr-merge）
//! 的能力暴露为前端 invoke 面，并完成 REQ-31 的 Channel 订阅（快照 + 事件分离）。
//!
//! 线程规则（§3.1 / REQ-31）：
//! - 可能阻塞的命令（枚举、探测、批量启动/停止、截图、合并、恢复扫描/收尾）一律以
//!   `async` + [`tauri::async_runtime::spawn_blocking`] 执行：UI（主）线程只做参数转换，
//!   COM/MTA 与 ffmpeg sidecar 等待不落在 WebView 线程上；
//! - 录制状态经 [`sr_core::events::ChannelHub`] 统一发布：**告警/状态事件**与**电平事件**
//!   是两条独立通道（电平 ≤10Hz 由 hub 节流），电平洪泛不得饿死告警；UI 重载后
//!   [`subscribe`] 以快照恢复视图；
//! - Channel 写入失败（WebView 重载/销毁）只退出转发线程，**不得中断录制**（REQ-31）。
//!
//! 范围说明（含实现期已知缺口，详见 docs/milestones/m9-commands-report.md）：
//! - `route_level` 的载荷形状已按契约定型（`{id,track,rms}`），但 sr-pipeline 目前不暴露
//!   任何电平/RMS 读数，命令层因此**不伪造**电平事件；待管线提供读数后由同一 hub 的
//!   `publish_level` 发布（节流已由 hub 保证）。
//! - `start_routes` 的 `region` 目标：sr-pipeline 的 Region 录制尚未实现（M3 形态），
//!   命令接受该 JSON 形状并原样透传其显式错误（E_TARGET_LOST）。
//! - 批量启动沿用 sr-pipeline `start_routes` 的「全成功或回滚」语义；失败时已启动各路
//!   由管线侧回滚，命令层仅返回带错误码的 invoke 错误（不产生半批次路由 id）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};

use sr_audio::wasapi::AudioSourceSpec;
use sr_core::events::{ChannelHub, Receiver, Subscription};
use sr_enum::windows as window_enum;
use sr_enum::{audio_sessions, monitors};
use sr_merge::merge::{MergeConfig, MergeError};
use sr_pipeline::probe::{LayerStatus, ProbeCapabilities};
use sr_pipeline::route::{
    self, AudioOnlyConfig, AudioOnlyHandle, MultiRouteConfig, RouteError, RouteHandle, StopNotice,
};
use sr_pipeline::{naming, probe, recovery};
use sr_shot::screenshot::{self, ScreenRect, ScreenshotRequest, ShotError, ShotTarget};

use crate::settings::{AudioMode, EncoderSetting, Settings};
use crate::SettingsState;

/// 路由运行状态（事件/快照的 `state` 字段）：recording → stopping → finalized | failed。
pub const STATE_RECORDING: &str = "recording";
/// 已请求停止（`request_stop` 已返回；成品经 `route_finalized` 通知，REQ-31）。
pub const STATE_STOPPING: &str = "stopping";
/// 已收尾（`route_finalized` 携带成品路径）。
pub const STATE_FINALIZED: &str = "finalized";
/// 路级失败（`route_error` 携带错误码与明细）。
pub const STATE_FAILED: &str = "failed";

/// 事件严重度：本层目前发布的均为「已终止该路/批次」的错误（REQ-31 告警独立于电平流）。
const SEVERITY_ERROR: &str = "error";

// ─────────────────────────── JSON DTO（§3.3：与前端约定一致） ───────────────────────────

/// 物理像素矩形（区域目标；负坐标合法，REQ-6）。
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RectIn {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// 录制/截图目标（内部标签 `kind`）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TargetIn {
    /// 窗口（HWND 数值化；身份由 sr-pipeline/sr-enum 侧解析）。
    Window { hwnd: u64 },
    /// 显示器（与 sr-enum 枚举同一索引空间，0..n-1）。
    Monitor { index: u32 },
    /// 矩形区域（物理桌面坐标；M18 录制目标）。
    ///
    /// `display_device` 为稳定设备路径（P2 `device_path`；不可得回退 GDI `szDevice`）。
    /// 共享字段以 serde 默认值保持**旧截图 JSON**（仅 `rect`）反序列化兼容；
    /// 录制入口强制校验非空，截图入口忽略该字段。
    Region {
        #[serde(default)]
        display_device: String,
        rect: RectIn,
    },
}

/// 固定输出画布（D6：启动时固定宽高/fps/像素格式）。
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct VideoCfgIn {
    pub canvas_w: u32,
    pub canvas_h: u32,
    pub fps: u32,
    /// 光标捕获开关（REQ-11：默认开）。
    #[serde(default = "default_cursor")]
    pub cursor: bool,
}

fn default_cursor() -> bool {
    true
}

/// 音源（§3.4 `AudioSourceSpec` 的 JSON 投影）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AudioSourceIn {
    /// 按应用（PID + 进程创建时间，REQ-14 身份防 PID 复用）。
    App { pid: u32, created_at_unix_ms: u64 },
    /// 系统音频（默认渲染端点环回，REQ-15）。
    System,
    /// 麦克风（`device_id` = `list_audio_devices` 返回的 id，REQ-16）。
    Mic { device_id: String },
    /// M16/REQ-11：显示器输出端点（`device_id` = `list_render_endpoints` 返回的 id）。
    RenderEndpoint { device_id: String },
}

/// 音轨计划（REQ-17：隔离/全音轨；`sources` 为实际音轨清单，每条一条独立轨）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AudioPlanIn {
    /// 音轨组织模式（`isolated` | `all_tracks`）。
    pub mode: AudioMode,
    #[serde(default)]
    pub sources: Vec<AudioSourceIn>,
}

/// 单路录制配置（§3.3 `RouteConfig`）。
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RouteConfigIn {
    pub target: TargetIn,
    pub video: VideoCfgIn,
    pub audio: AudioPlanIn,
    pub output_dir: String,
}

/// 截图请求（`output_dir` 省略时取设置中的输出目录）。
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScreenshotRequestIn {
    pub target: TargetIn,
    #[serde(default)]
    pub output_dir: Option<String>,
}

/// `start_routes` / `start_audio_only` 回执：与请求同序的 route id。
#[derive(Clone, Debug, Serialize)]
pub struct StartRoutesOut {
    pub route_ids: Vec<String>,
}

/// `stop_route` 回执（REQ-31：「已请求停止」；成品经事件通知）。
#[derive(Clone, Debug, Serialize)]
pub struct StopRouteOut {
    pub route_id: String,
    /// false = 该路已处于终态（finalized/failed），未重复请求。
    pub requested: bool,
}

/// `stop_all` 回执：本次实际请求停止的路由 id。
#[derive(Clone, Debug, Serialize)]
pub struct StopAllOut {
    pub requested: Vec<String>,
}

/// `take_screenshot` 回执（REQ-5：输出目录内的 PNG）。
#[derive(Clone, Debug, Serialize)]
pub struct ScreenshotOut {
    pub path: String,
}

/// `merge_recordings` 回执（REQ-23）。
#[derive(Clone, Debug, Serialize)]
pub struct MergeOut {
    pub out: String,
}

/// `list_targets` 回执（REQ-1/REQ-2）。
#[derive(Clone, Debug, Serialize)]
pub struct TargetsOut {
    pub monitors: Vec<MonitorOut>,
    pub windows: Vec<WindowOut>,
}

/// 显示器条目（REQ-2：物理像素；`primary` 有且仅有一台）。
/// M16/REQ-10：新增 friendly_name/device_path/container_id（元数据失败仅字段降级）。
#[derive(Clone, Debug, Serialize)]
pub struct MonitorOut {
    pub index: u32,
    /// 显示名（友好名缺失时回退 `Monitor {index}`；保持既有字段兼容）。
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub primary: bool,
    /// 显示器友好名（缺失 → 回退 `Monitor {index}`）。
    pub friendly_name: String,
    /// GDI 设备名（`\\.\DISPLAY1`；M18/REQ-22 device_path 缺失时的回退键）。
    pub gdi_device_name: Option<String>,
    /// DisplayConfig monitorDevicePath（稳定键；缺失 → null）。
    pub device_path: Option<String>,
    /// PnP ContainerId（缺失/全零 → null）。
    pub container_id: Option<String>,
}

/// 渲染端点（M16/REQ-9）。
#[derive(Clone, Debug, Serialize)]
pub struct RenderEndpointOut {
    pub device_id: String,
    pub friendly_name: String,
    pub mix_rate: Option<u32>,
    pub is_active: bool,
    pub container_id: Option<String>,
}

/// 窗口条目（REQ-1：身份 = HWND + PID + 进程创建时间）。
#[derive(Clone, Debug, Serialize)]
pub struct WindowOut {
    pub hwnd: u64,
    pub title: String,
    pub app_name: String,
    pub process_name: String,
    pub pid: u32,
    pub created_at_unix_ms: u64,
}

/// 活动音频会话（REQ-4：值快照；`active` 恒为 true，保留字段供前端统一渲染）。
#[derive(Clone, Debug, Serialize)]
pub struct AudioSourceOut {
    pub pid: u32,
    /// 进程创建时间（构造 `AudioSourceIn::App` 的身份字段；缺此字段无法通过 REQ-14 校验）。
    pub created_at_unix_ms: u64,
    pub process_name: String,
    pub app_name: String,
    pub active: bool,
}

/// 采集（麦克风）设备（REQ-16）。
#[derive(Clone, Debug, Serialize)]
pub struct AudioDeviceOut {
    pub id: String,
    pub name: String,
    /// 设备类别（当前仅采集设备）。
    pub kind: String,
    pub is_default: bool,
}

/// 端点原生混音率（M15/REQ-2：UI 只读展示；`kind` = `render` | `capture`）。
#[derive(Clone, Debug, Serialize)]
pub struct EndpointMixRateOut {
    pub rate: u32,
    pub kind: String,
    pub device_name: String,
}

/// `probe_capabilities` 回执（REQ-28；`*_ok` 为各层归类的布尔投影，明细见 `layers`）。
#[derive(Clone, Debug, Serialize)]
pub struct ProbeOut {
    pub os_build: String,
    pub os_supported: bool,
    pub encoders: Vec<EncoderOut>,
    /// D4 链序首个初始化成功的编码器；全败 → null（§9 T1：明确报错，不静默降级）。
    pub selected_encoder: Option<String>,
    /// process loopback 试激活通过（status == ok）。
    pub loopback_ok: bool,
    /// 麦克风可用（隐私允许且设备可打开；status == ok）。
    pub mic_privacy: bool,
    /// WGC 授权可用（status == ok）。
    pub wgc_ok: bool,
    pub canvas_limits: CanvasLimitsOut,
    /// 三层归类明细（拒绝/超时/取消/不可用/失败可区分，REQ-28）。
    pub layers: LayersOut,
}

/// 单编码器探测结果。
#[derive(Clone, Debug, Serialize)]
pub struct EncoderOut {
    pub name: String,
    pub init_ok: bool,
    pub detail: String,
}

/// 画布上限（D6 固定画布安全上界）。
#[derive(Clone, Copy, Debug, Serialize)]
pub struct CanvasLimitsOut {
    pub max_width: u32,
    pub max_height: u32,
}

/// 三类权限/激活面的归类明细。
#[derive(Clone, Debug, Serialize)]
pub struct LayersOut {
    pub loopback: LayerOut,
    pub mic: LayerOut,
    pub wgc: LayerOut,
}

/// 单层归类（status ∈ ok/denied/timeout/canceled/unavailable/failed）。
#[derive(Clone, Debug, Serialize)]
pub struct LayerOut {
    pub status: String,
    pub detail: String,
}

/// 可恢复会话（REQ-22：崩溃残留；`manifest_ok=false` 时仅可显式丢弃）。
#[derive(Clone, Debug, Serialize)]
pub struct SessionOut {
    pub session_id: String,
    /// 会话临时目录绝对路径（诊断/展示；收尾/丢弃仍以 `session_id` 定位）。
    pub dir: String,
    /// 清单 `created_at_unix_ms`；清单不可读 → 0。
    pub created_at: u64,
    /// 会话内仍有活动文件（录制中）→ 收尾会被拒绝。
    pub active: bool,
    /// 清单可读（`false` = 清单损坏/缺失，不静默自动收尾）。
    pub manifest_ok: bool,
    pub routes: Vec<SessionRouteOut>,
    /// 可收尾封口段（已排除 0 字节/坏 EBML/活动文件）。
    pub sealed_segments: Vec<String>,
    /// 末可解析段（未封口形态；不得当封口段用）。
    pub tail_segment: Option<String>,
}

/// 会话内单路（清单视角）。
#[derive(Clone, Debug, Serialize)]
pub struct SessionRouteOut {
    pub route_id: String,
    /// 目标名（清单 `identity.name`）。
    pub name: String,
    pub streams: Vec<String>,
    pub state: String,
    /// 清单登记的段条目数。
    pub segments: usize,
    pub final_path: Option<String>,
}

/// `recover_sessions` 回执。
#[derive(Clone, Debug, Serialize)]
pub struct RecoverSessionsOut {
    pub sessions: Vec<SessionOut>,
}

/// `finalize_session` 回执（REQ-22：幂等；成功后按 `keep_segments` 清理/保留原段）。
#[derive(Clone, Debug, Serialize)]
pub struct FinalizeSessionOut {
    pub finals: Vec<RouteFinalOut>,
    /// 就地封口的 WAV 崩溃残留（REQ-19/22）。
    pub repaired_wavs: Vec<String>,
}

/// 恢复收尾成品（route_id → 输出目录根下的 MKV）。
#[derive(Clone, Debug, Serialize)]
pub struct RouteFinalOut {
    pub route_id: String,
    pub path: String,
}

/// `discard_session` 回执。
#[derive(Clone, Debug, Serialize)]
pub struct DiscardSessionOut {
    pub session_id: String,
    pub discarded: bool,
}

// ─────────────────────────── Channel（REQ-31） ───────────────────────────

/// Channel 事件（内部标签 `event`；与前端的 §3.3 事件名一一对应）。
///
/// 状态/告警事件与电平事件是 hub 的两条独立通道：本枚举两侧共用同一 JSON 形状，
/// 但**告警不会排在电平队列后面**（REQ-27：告警不被电平流量饿死）。
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EventOut {
    /// 路由状态变化（recording/stopping/finalized/failed）。
    RouteState {
        id: String,
        state: String,
        /// M16/REQ-12：停止原因（无 → 省略）。
        #[serde(skip_serializing_if = "Option::is_none")]
        stop_notice: Option<StopNoticeOut>,
    },
    /// 电平（≤10Hz，hub 节流）。当前管线无读数来源，见模块注释。
    RouteLevel { id: String, track: String, rms: f32 },
    /// 路级告警/错误（`severity` 目前恒为 error：发布时该路已终止）。
    RouteError {
        id: String,
        code: String,
        message: String,
        severity: String,
        /// M16/REQ-12：停止原因（无 → 省略）。
        #[serde(skip_serializing_if = "Option::is_none")]
        stop_notice: Option<StopNoticeOut>,
    },
    /// 成品就绪（停止后经事件通知，REQ-31）。
    RouteFinalized {
        id: String,
        path: String,
        /// M16/REQ-12：停止原因（无 → 省略）。
        #[serde(skip_serializing_if = "Option::is_none")]
        stop_notice: Option<StopNoticeOut>,
    },
    /// 崩溃残留被发现（UI 提示收尾/丢弃，REQ-22）。
    RecoveryFound { session: SessionOut },
}

/// `subscribe` 返回的当前状态快照（REQ-31：UI 重载后以快照恢复视图）。
#[derive(Clone, Debug, Serialize)]
pub struct StateSnapshot {
    /// 进程内已知路由（按 id 字典序，不代表启动顺序）。
    pub routes: Vec<RouteStateOut>,
    /// 最近一次 `recover_sessions` 的扫描结果（含已收尾/已丢弃前的条目）。
    pub recovery: Vec<SessionOut>,
}

/// M16/REQ-12：路级停止原因（端点失效；成功/失败分支都进入快照与终态事件）。
#[derive(Clone, Debug, Serialize)]
pub struct StopNoticeOut {
    pub code: String,
    pub endpoint: Option<String>,
    pub detail: String,
}

/// 快照内的单路状态。
#[derive(Clone, Debug, Serialize)]
pub struct RouteStateOut {
    pub id: String,
    pub state: String,
    /// 终态（finalized）时的成品路径；其余为 null。
    pub final_path: Option<String>,
    /// M16/REQ-12：停止原因（非端点失效 → 省略字段，保持既有 JSON 形状）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_notice: Option<StopNoticeOut>,
}

// ─────────────────────────── 运行时状态 ───────────────────────────

/// 命令层运行时状态（Tauri managed state；Hub + 活动路由表 + 恢复扫描缓存）。
pub struct CommandsState {
    hub: Arc<ChannelHub<StateSnapshot, EventOut, EventOut>>,
    /// route_id → 槽位（音频纯录音的 N 个来源共享同一 handle，故用 Arc）。
    routes: Mutex<BTreeMap<String, RouteSlot>>,
    next_id: AtomicU64,
    /// 最近一次 `recover_sessions` 结果（快照/事件去重用）。
    recoveries: Mutex<Vec<SessionOut>>,
    /// 最近一次成功启动的配置（托盘「开始」重放；无 → 明确提示未选择目标）。
    last_routes: Mutex<Option<Vec<RouteConfigIn>>>,
    /// M16/REQ-12：快照生成与 Hub 提交串行化（A 取旧快照 / B 提交终态不得交错回退）。
    snapshot_lock: Mutex<()>,
}

struct RouteSlot {
    /// 纯录音/视频路句柄；仅测试夹具可为 None（生产启动路径必为 Some）。
    handle: Option<Arc<ActiveRoute>>,
    state: String,
    final_path: Option<String>,
    /// M16/REQ-12：路级停止原因（端点失效）。
    stop_notice: Option<StopNoticeOut>,
}

/// 活动路由句柄（视频路 / 纯录音会话）。
enum ActiveRoute {
    Video(RouteHandle),
    AudioOnly(AudioOnlyHandle),
}

/// 收尾结果（视频 = MKV 成品；纯录音 = 各来源 WAV）。
enum RouteOutcome {
    Video(route::Finalized),
    AudioOnly(route::AudioOnlyFinalized),
}

impl ActiveRoute {
    /// 「已请求停止」：立即返回（§9 T5；实际停止/收尾在 worker 线程后台完成）。
    fn request_stop(&self) {
        match self {
            Self::Video(h) => h.request_stop(),
            Self::AudioOnly(h) => h.request_stop(),
        }
    }

    /// 等待后台收尾（超时返回 `RouteError::WaitTimeout`，调用方可继续等待）。
    fn wait(&self, timeout: Duration) -> Result<RouteOutcome, RouteError> {
        match self {
            Self::Video(h) => h.wait_finalized(timeout).map(RouteOutcome::Video),
            Self::AudioOnly(h) => h.wait_finalized(timeout).map(RouteOutcome::AudioOnly),
        }
    }

    /// M16/REQ-12：失败分支的 stop_notice（成功分支由 Finalized 携带）。
    fn stop_notice(&self) -> Option<StopNoticeOut> {
        match self {
            Self::Video(h) => h.stop_notice().as_ref().map(notice_out),
            Self::AudioOnly(_) => None,
        }
    }
}

impl Default for CommandsState {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandsState {
    pub fn new() -> Self {
        Self {
            hub: Arc::new(ChannelHub::new(StateSnapshot {
                routes: Vec::new(),
                recovery: Vec::new(),
            })),
            routes: Mutex::new(BTreeMap::new()),
            next_id: AtomicU64::new(1),
            recoveries: Mutex::new(Vec::new()),
            last_routes: Mutex::new(None),
            snapshot_lock: Mutex::new(()),
        }
    }

    /// 记录最近一次成功启动的配置（托盘「开始」语义 = 重放该配置）。
    fn remember_routes(&self, routes: Vec<RouteConfigIn>) {
        *self.last_routes.lock().expect("最近配置锁") = Some(routes);
    }

    /// 托盘「开始」取最近配置；None = 尚未选择/启动过任何目标。
    pub(crate) fn last_routes(&self) -> Option<Vec<RouteConfigIn>> {
        self.last_routes.lock().expect("最近配置锁").clone()
    }

    /// 是否仍有活动（非终态）路由（托盘「开始」防重复）。
    pub(crate) fn has_active_routes(&self) -> bool {
        self.routes
            .lock()
            .expect("路由表锁")
            .values()
            .any(|slot| !is_terminal(&slot.state))
    }

    /// 分配 n 个进程内唯一 route id（`route-{n}`；与管线内部 route_id 无关）。
    fn alloc_ids(&self, n: usize) -> Vec<String> {
        (0..n)
            .map(|_| {
                let n = self.next_id.fetch_add(1, Ordering::Relaxed);
                format!("route-{n}")
            })
            .collect()
    }

    /// 注册一组（同 handle 的）路由、发布 `recording` 状态并启动收尾监视线程。
    fn launch_group(&self, app: &AppHandle, ids: Vec<String>, handle: Arc<ActiveRoute>) {
        {
            let mut routes = self.routes.lock().expect("路由表锁");
            for id in &ids {
                routes.insert(
                    id.clone(),
                    RouteSlot {
                        handle: Some(handle.clone()),
                        state: STATE_RECORDING.to_string(),
                        final_path: None,
                        stop_notice: None,
                    },
                );
            }
        }
        self.publish_snapshot();
        for id in &ids {
            self.hub.publish_alert(EventOut::RouteState {
                id: id.clone(),
                state: STATE_RECORDING.to_string(),
                stop_notice: None,
            });
        }
        spawn_monitor(app.clone(), ids, handle);
    }

    /// 当前快照（路由表按 id 字典序 + 最近恢复扫描）。
    fn build_snapshot(&self) -> StateSnapshot {
        let routes = self
            .routes
            .lock()
            .expect("路由表锁")
            .iter()
            .map(|(id, slot)| RouteStateOut {
                id: id.clone(),
                state: slot.state.clone(),
                final_path: slot.final_path.clone(),
                stop_notice: slot.stop_notice.clone(),
            })
            .collect();
        let recovery = self.recoveries.lock().expect("恢复缓存锁").clone();
        StateSnapshot { routes, recovery }
    }

    /// M16/REQ-12：快照生成与 Hub 提交在**同一锁内**完成；任何终态提交不会与
    /// 旧快照的生成/提交交错（A 取旧快照 → B 提交终态 → A 回退缓存 不可能发生）。
    fn publish_snapshot(&self) {
        let _serial = self.snapshot_lock.lock().expect("快照串行锁");
        self.hub.update_state(self.build_snapshot());
    }

    /// 收尾成功：更新终态 → 快照 → `route_finalized`（每 id 一条，顺序同 ids）。
    /// M16/REQ-12：stop_notice（端点失效）进入快照与终态事件载荷。
    fn mark_finalized(&self, ids: &[String], outcome: &RouteOutcome) {
        let notice = match outcome {
            RouteOutcome::Video(fin) => fin.stop_notice.as_ref().map(notice_out),
            RouteOutcome::AudioOnly(_) => None,
        };
        let mut events = Vec::with_capacity(ids.len());
        {
            let mut routes = self.routes.lock().expect("路由表锁");
            for (i, id) in ids.iter().enumerate() {
                let path = match outcome {
                    RouteOutcome::Video(fin) => fin.final_path.to_string_lossy().into_owned(),
                    // 纯录音：ids 与 sources 同序；wav_paths 同序（REQ-19）。
                    RouteOutcome::AudioOnly(fin) => fin
                        .wav_paths
                        .get(i)
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                };
                if let Some(slot) = routes.get_mut(id) {
                    slot.state = STATE_FINALIZED.to_string();
                    slot.final_path = Some(path.clone());
                    slot.stop_notice = notice.clone();
                }
                events.push((id.clone(), path));
            }
        }
        self.publish_snapshot();
        for (id, path) in events {
            self.hub.publish_alert(EventOut::RouteFinalized {
                id,
                path,
                stop_notice: notice.clone(),
            });
        }
    }

    /// 路级失败：更新终态 → 快照 → `route_error`（每 id 一条）。
    fn mark_failed(&self, ids: &[String], error: &RouteError, notice: Option<StopNoticeOut>) {
        let code = route_error_code(error).to_string();
        let message = route_error_message(error);
        {
            let mut routes = self.routes.lock().expect("路由表锁");
            for id in ids {
                if let Some(slot) = routes.get_mut(id) {
                    slot.state = STATE_FAILED.to_string();
                    slot.stop_notice = notice.clone();
                }
            }
        }
        self.publish_snapshot();
        for id in ids {
            self.hub.publish_alert(EventOut::RouteError {
                id: id.clone(),
                code: code.clone(),
                message: message.clone(),
                severity: SEVERITY_ERROR.to_string(),
                stop_notice: notice.clone(),
            });
        }
    }

    /// 覆盖恢复扫描缓存（`recover_sessions` 成功路径）。
    fn set_recoveries(&self, sessions: Vec<SessionOut>) {
        *self.recoveries.lock().expect("恢复缓存锁") = sessions;
        self.publish_snapshot();
    }

    /// 从恢复缓存中移除已处理会话（收尾/丢弃成功后）。
    fn remove_recovery(&self, session_id: &str) {
        self.recoveries
            .lock()
            .expect("恢复缓存锁")
            .retain(|s| s.session_id != session_id);
        self.publish_snapshot();
    }
}

/// M16/REQ-12：管线 StopNotice → JSON 投影。
fn notice_out(notice: &StopNotice) -> StopNoticeOut {
    StopNoticeOut {
        code: notice.code.clone(),
        endpoint: notice.endpoint.clone(),
        detail: notice.detail.clone(),
    }
}

/// 收尾监视线程：等待该 handle 的最终结果 → 事件；发送端失败不影响录制（REQ-31）。
fn spawn_monitor(app: AppHandle, ids: Vec<String>, handle: Arc<ActiveRoute>) {
    let label = ids.first().cloned().unwrap_or_default();
    let thread_ids = ids.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("sr-route-monitor-{label}"))
        .spawn(move || loop {
            // 长超时 + 重试：等待期间无其它可观测性（wait_finalized 可重复调用）。
            match handle.wait(Duration::from_secs(3600)) {
                Err(RouteError::WaitTimeout) => continue,
                Ok(outcome) => {
                    let Some(state) = app.try_state::<CommandsState>() else {
                        return;
                    };
                    state.mark_finalized(&thread_ids, &outcome);
                    return;
                }
                Err(error) => {
                    let Some(state) = app.try_state::<CommandsState>() else {
                        return;
                    };
                    // M16/REQ-12：失败分支仍携带 stop_notice（端点失效 → E_TARGET_LOST）。
                    state.mark_failed(&thread_ids, &error, handle.stop_notice());
                    return;
                }
            }
        });
    if let Err(e) = spawned {
        // 仅有事件通知缺失（录制/收尾本身不受影响）：如实记录，不伪造成路由失败。
        eprintln!("[commands] 路由监视线程启动失败（{}）：{e}", ids.join(","));
    }
}

// ─────────────────────────── 命令 ───────────────────────────

/// §3.3：`probe_capabilities()`——分层探测（REQ-28）。
#[tauri::command]
pub async fn probe_capabilities() -> Result<ProbeOut, String> {
    blocking(|| Ok(probe_view(probe::probe_capabilities()))).await
}

/// §3.3：`list_targets()`——显示器 + 可见顶层窗口（REQ-1/2/3）。
#[tauri::command]
pub async fn list_targets() -> Result<TargetsOut, String> {
    blocking(|| {
        let monitors = monitors::list_monitors()
            .map_err(|e| invalid(e.code(), format!("显示器枚举失败：{e}")))?;
        let windows = window_enum::list_windows()
            .map_err(|e| invalid(e.code(), format!("窗口枚举失败：{e}")))?;
        Ok(TargetsOut {
            monitors: monitors
                .into_iter()
                .map(|m| {
                    let name = m
                        .friendly_name
                        .clone()
                        .unwrap_or_else(|| format!("Monitor {}", m.index));
                    MonitorOut {
                        index: m.index,
                        name: name.clone(),
                        width: m.width_px,
                        height: m.height_px,
                        x: m.x,
                        y: m.y,
                        primary: m.is_primary,
                        friendly_name: name,
                        gdi_device_name: m.gdi_device_name,
                        device_path: m.device_path,
                        container_id: m.container_id,
                    }
                })
                .collect(),
            windows: windows
                .into_iter()
                .map(|w| WindowOut {
                    hwnd: w.hwnd,
                    title: w.title,
                    app_name: w.app_name,
                    process_name: w.process_name,
                    pid: w.pid,
                    created_at_unix_ms: w.process_created_at,
                })
                .collect(),
        })
    })
    .await
}

/// §3.3：`list_audio_sources()`——活动音频会话快照（REQ-4）。
#[tauri::command]
pub async fn list_audio_sources() -> Result<Vec<AudioSourceOut>, String> {
    blocking(|| {
        let list = audio_sessions::list_audio_sessions()
            .map_err(|e| invalid(e.code(), format!("音频会话枚举失败：{e}")))?;
        Ok(list
            .into_iter()
            .map(|s| AudioSourceOut {
                pid: s.pid,
                created_at_unix_ms: s.process_created_at,
                process_name: s.process_name,
                app_name: s.app_name,
                active: true,
            })
            .collect())
    })
    .await
}

/// §3.3：`list_audio_devices()`——采集（麦克风）设备（REQ-16）。
#[tauri::command]
pub async fn list_audio_devices() -> Result<Vec<AudioDeviceOut>, String> {
    blocking(|| {
        let devices = sr_audio::wasapi::list_capture_devices()
            .map_err(|e| invalid("E_MIC_UNAVAILABLE", format!("采集设备枚举失败：{e:?}")))?;
        Ok(devices
            .into_iter()
            .map(|d| AudioDeviceOut {
                id: d.id,
                name: d.name,
                kind: "mic".to_string(),
                is_default: d.is_default,
            })
            .collect())
    })
    .await
}

/// M16/REQ-9：`list_render_endpoints() -> Vec<{device_id,friendly_name,mix_rate,is_active,container_id}>`。
///
/// 枚举失败返回空列表 + 日志（不阻断 UI/录制）；MTA 线程在 `sr-enum` 内执行。
#[tauri::command]
pub async fn list_render_endpoints() -> Vec<RenderEndpointOut> {
    tauri::async_runtime::spawn_blocking(
        || match sr_enum::render_endpoints::list_render_endpoints() {
            Ok(list) => list
                .into_iter()
                .map(|e| RenderEndpointOut {
                    device_id: e.device_id,
                    friendly_name: e.friendly_name,
                    mix_rate: e.mix_rate,
                    is_active: e.is_active,
                    container_id: e.container_id,
                })
                .collect(),
            Err(e) => {
                eprintln!("[commands] 渲染端点枚举失败（返回空列表）：{e}");
                Vec::new()
            }
        },
    )
    .await
    .unwrap_or_default()
}

/// M15/REQ-2：`get_endpoint_mix_rate(device_id?) -> Option<{rate,kind,device_name}>`。
///
/// `device_id=None` → 默认渲染端点（`kind="render"`）；`Some(id)` → 麦克风采集
/// 设备（`kind="capture"`）。MTA 线程在 `sr-audio` 内执行；任何失败返回 `null`
/// （UI 显示「未知」）。与 `probe_capabilities` 完全隔离：查询失败不影响能力
/// 探测/录制（捕获初始化自身还有 REQ-7 的回退）。**查询值不用于推导重采样比率。**
#[tauri::command]
pub async fn get_endpoint_mix_rate(device_id: Option<String>) -> Option<EndpointMixRateOut> {
    tauri::async_runtime::spawn_blocking(move || {
        sr_audio::wasapi::query_endpoint_mix_rate(device_id).map(|m| EndpointMixRateOut {
            rate: m.rate,
            kind: m.kind.to_string(),
            device_name: m.device_name,
        })
    })
    .await
    .ok()
    .flatten()
}

/// §3.3：`start_routes(Vec<RouteConfigIn>) -> {route_ids}`——批量启动（**全成功或回滚**，
/// 复用 sr-pipeline `start_routes`）。Ok = 各路均已「输出开始」（§3.1）。
#[tauri::command]
pub async fn start_routes(
    app: AppHandle,
    routes: Vec<RouteConfigIn>,
) -> Result<StartRoutesOut, String> {
    blocking(move || start_routes_impl(app, routes)).await
}

/// `start_routes` 的同步实现（命令层与托盘「开始」共用同一语义；P2）。
pub(crate) fn start_routes_impl(
    app: AppHandle,
    routes: Vec<RouteConfigIn>,
) -> Result<StartRoutesOut, String> {
    let cfgs = plan_routes(routes.clone())?;
    // P2：设置中的编码器经命令 DTO → 管线（Auto=探测；显式名须实际初始化成功）。
    // M14：设置 keep_segments 原样透传（默认 false = 成功后自动清理原段）。
    // M15/REQ-1/8：启动读取一次设置快照，输出率随 start_routes 入口传入管线；
    // 在录路由不随保存变更（托盘「开始」同样在调用时取新快照）。
    let settings = current_settings(&app);
    let encoder = encoder_override(&settings);
    let handles = route::start_routes_with_options_rate_container_and_privacy(
        cfgs,
        encoder,
        settings.keep_segments,
        settings.audio_sample_rate,
        settings.output_container,
        settings.include_window_title_in_filename,
    )
    .map_err(|e| e.to_string())?;
    let state = app.state::<CommandsState>();
    let ids = state.alloc_ids(handles.len());
    for (id, handle) in ids.iter().zip(handles) {
        state.launch_group(&app, vec![id.clone()], Arc::new(ActiveRoute::Video(handle)));
    }
    state.remember_routes(routes); // 托盘「开始」= 重放最近一次成功配置
    Ok(StartRoutesOut { route_ids: ids })
}

/// §3.3：`start_audio_only(Vec<AudioSourceSpec>) -> {route_ids}`——纯录音（REQ-19）。
///
/// `sources` 与返回的 `route_ids` 同序（每来源一条独立 WAV）；省略 `output_dir` 时取
/// 设置中的输出目录。纯录音为**一次会话**：停止任一 id 即停止全部来源（停止即成品）。
/// M14b：成功后按设置 `keep_segments` 清理/保留 `.screenrec_tmp/<session_id>`（默认清理）。
#[tauri::command]
pub async fn start_audio_only(
    app: AppHandle,
    sources: Vec<AudioSourceIn>,
    output_dir: Option<String>,
) -> Result<StartRoutesOut, String> {
    blocking(move || {
        let specs = plan_audio_only(sources)?;
        let count = specs.len();
        // M15/REQ-1/3：一次设置快照决定 WAV 输出率；在录路由不随保存变更。
        let settings = current_settings(&app);
        let dir = resolve_output_dir(&app, output_dir);
        // M14b：纯录音成功收尾按设置清理/保留会话目录（默认 false = 自动清理）。
        let handle = route::start_audio_only_with_options_and_rate(
            AudioOnlyConfig {
                sources: specs,
                output_dir: PathBuf::from(dir),
            },
            settings.keep_segments,
            settings.audio_sample_rate,
        )
        .map_err(|e| e.to_string())?;
        let state = app.state::<CommandsState>();
        let ids = state.alloc_ids(count);
        state.launch_group(&app, ids.clone(), Arc::new(ActiveRoute::AudioOnly(handle)));
        Ok(StartRoutesOut { route_ids: ids })
    })
    .await
}

/// §3.3：`stop_route(route_id)`——返回「已请求停止」；成品就绪经 `route_finalized`（REQ-31）。
///
/// 纯录音的 N 个来源共享一次会话（同一 handle）：停止任一 id 即停止整组，簇内其余
/// id 同样进入 `stopping`（避免 UI 出现「已停止 / 仍在录制」的矛盾状态）。
#[tauri::command]
pub fn stop_route(app: AppHandle, route_id: String) -> Result<StopRouteOut, String> {
    let state = app.state::<CommandsState>();
    let mut affected = Vec::new();
    let handle = {
        let mut routes = state.routes.lock().expect("路由表锁");
        let slot = routes
            .get(&route_id)
            .ok_or_else(|| invalid("E_ROUTE_NOT_FOUND", format!("未知路由：{route_id}")))?;
        if is_terminal(&slot.state) {
            return Ok(StopRouteOut {
                route_id,
                requested: false,
            });
        }
        let handle = slot
            .handle
            .clone()
            .ok_or_else(|| invalid("E_ROUTE_NOT_FOUND", format!("未知路由句柄：{route_id}")))?;
        for (id, slot) in routes.iter_mut() {
            // 已在停止中的路由不重复发状态事件（request_stop 幂等，重复调用无副作用）。
            if slot
                .handle
                .as_ref()
                .map(|h| Arc::ptr_eq(h, &handle))
                .unwrap_or(false)
                && !is_terminal(&slot.state)
                && slot.state != STATE_STOPPING
            {
                slot.state = STATE_STOPPING.to_string();
                affected.push(id.clone());
            }
        }
        handle
    };
    handle.request_stop();
    state.publish_snapshot();
    for id in &affected {
        state.hub.publish_alert(EventOut::RouteState {
            id: id.clone(),
            state: STATE_STOPPING.to_string(),
            stop_notice: None,
        });
    }
    Ok(StopRouteOut {
        route_id,
        requested: true,
    })
}

/// §3.3：`stop_all()`——请求停止全部活动路由（终态路由跳过）。
#[tauri::command]
pub fn stop_all(app: AppHandle) -> Result<StopAllOut, String> {
    stop_all_impl(&app)
}

/// `stop_all` 的同步实现（命令层与托盘「停止」共用；P2）。
pub(crate) fn stop_all_impl(app: &AppHandle) -> Result<StopAllOut, String> {
    let state = app.state::<CommandsState>();
    let mut requested = Vec::new();
    {
        let mut routes = state.routes.lock().expect("路由表锁");
        for (id, slot) in routes.iter_mut() {
            if is_terminal(&slot.state) || slot.state == STATE_STOPPING {
                continue;
            }
            slot.state = STATE_STOPPING.to_string();
            if let Some(handle) = &slot.handle {
                handle.request_stop();
            }
            requested.push(id.clone());
        }
    }
    state.publish_snapshot();
    for id in &requested {
        state.hub.publish_alert(EventOut::RouteState {
            id: id.clone(),
            state: STATE_STOPPING.to_string(),
            stop_notice: None,
        });
    }
    Ok(StopAllOut { requested })
}

/// 托盘「开始」（P2）：重放最近一次成功配置；无配置 → 明确错误文本（由托盘提示）。
pub(crate) fn start_from_tray(app: &AppHandle) -> Result<StartRoutesOut, String> {
    let state = app.state::<CommandsState>();
    if state.has_active_routes() {
        return Err(invalid("E_ALREADY_RECORDING", "已有录制进行中；请先停止"));
    }
    let routes = state.last_routes().ok_or_else(|| {
        invalid(
            "E_NO_TARGET",
            "未选择录制目标：请先在主窗口选择窗口/显示器/区域并开始一次录制（托盘「开始」将复用该配置）",
        )
    })?;
    start_routes_impl(app.clone(), routes)
}

/// §3.3：`take_screenshot(request) -> {path}`——显示器/窗口/区域（REQ-5..7）。
#[tauri::command]
pub async fn take_screenshot(
    app: AppHandle,
    request: ScreenshotRequestIn,
) -> Result<ScreenshotOut, String> {
    blocking(move || {
        let dir = resolve_output_dir(&app, request.output_dir);
        std::fs::create_dir_all(&dir).map_err(|e| {
            invalid(
                "E_OUTPUT_UNAVAILABLE",
                format!("创建截图输出目录失败（{dir}）：{e}"),
            )
        })?;
        let saved = screenshot::take_screenshot(ScreenshotRequest {
            target: shot_target(&request.target)?,
            output_dir: PathBuf::from(dir),
        })
        .map_err(shot_error_string)?;
        Ok(ScreenshotOut {
            path: saved.path.to_string_lossy().into_owned(),
        })
    })
    .await
}

/// §3.3：`merge_recordings(paths, out) -> {out}`——异构多轨合并（REQ-23）。
///
/// 同步一次调用（无取消点）：合并进度不在本命令承载（sr-merge API 无进度钩子）。
#[tauri::command]
pub async fn merge_recordings(paths: Vec<String>, out: String) -> Result<MergeOut, String> {
    blocking(move || {
        if paths.len() < 2 {
            return Err(invalid(
                "E_MERGE_INCOMPATIBLE",
                format!("合并需要 ≥2 个输入（得到 {}）", paths.len()),
            ));
        }
        let cfg = MergeConfig {
            inputs: paths.into_iter().map(PathBuf::from).collect(),
            out: PathBuf::from(out),
        };
        let outcome = sr_merge::merge::merge_recordings(cfg).map_err(merge_error_string)?;
        Ok(MergeOut {
            out: outcome.out.to_string_lossy().into_owned(),
        })
    })
    .await
}

/// §3.3：`recover_sessions() -> {sessions}`——扫描输出目录下的崩溃残留（REQ-22）。
///
/// 新发现且**非活动**的会话额外经 Channel 发布 `recovery_found`（告警通道）。
#[tauri::command]
pub async fn recover_sessions(app: AppHandle) -> Result<RecoverSessionsOut, String> {
    blocking(move || {
        let output_dir = current_settings(&app).output_dir;
        let sessions: Vec<SessionOut> = recovery::scan_sessions(Path::new(&output_dir))
            .into_iter()
            .map(session_view)
            .collect();
        let state = app.state::<CommandsState>();
        let previous: Vec<String> = state
            .recoveries
            .lock()
            .expect("恢复缓存锁")
            .iter()
            .map(|s| s.session_id.clone())
            .collect();
        for session in &sessions {
            if !session.active && !previous.contains(&session.session_id) {
                state.hub.publish_alert(EventOut::RecoveryFound {
                    session: session.clone(),
                });
            }
        }
        state.set_recoveries(sessions.clone());
        Ok(RecoverSessionsOut { sessions })
    })
    .await
}

/// §3.3：`finalize_session(session_id)`——崩溃残留收尾（幂等；成功后按 `keep_segments`
/// 清理/保留原段，REQ-22 + M14c）。
#[tauri::command]
pub async fn finalize_session(
    app: AppHandle,
    session_id: String,
) -> Result<FinalizeSessionOut, String> {
    blocking(move || {
        // M17/REQ-18：执行时一次读取 output_dir/keep_segments/output_container 同一快照
        // （扫描时不读取；容器设置仅作用于未完成路由）。
        // M19a：窗口标题隐私开关同快照读取（默认 false = 恢复名不含旧标题）。
        let snap = finalize_session_settings_snapshot(|| current_settings(&app));
        let dir = session_dir_for_output(&snap.output_dir, &session_id)?;
        let outcome = recovery::finalize_session_with_options_container_and_privacy(
            &dir,
            snap.keep_segments,
            snap.output_container,
            snap.include_window_title_in_filename,
        )
        .map_err(|e| recovery_error_string(&e))?;
        app.state::<CommandsState>().remove_recovery(&session_id);
        Ok(FinalizeSessionOut {
            finals: outcome
                .route_finals
                .into_iter()
                .map(|(route_id, path)| RouteFinalOut {
                    route_id,
                    path: path.to_string_lossy().into_owned(),
                })
                .collect(),
            repaired_wavs: outcome
                .repaired_wavs
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
        })
    })
    .await
}

/// §3.3：`discard_session(session_id)`——显式丢弃（删除会话临时目录，REQ-22）。
#[tauri::command]
pub async fn discard_session(
    app: AppHandle,
    session_id: String,
) -> Result<DiscardSessionOut, String> {
    blocking(move || {
        let dir = session_dir_for(&app, &session_id)?;
        recovery::discard_session(&dir).map_err(|e| recovery_error_string(&e))?;
        app.state::<CommandsState>().remove_recovery(&session_id);
        Ok(DiscardSessionOut {
            session_id,
            discarded: true,
        })
    })
    .await
}

/// §3.3：`subscribe(channel)`——返回当前状态快照，并把后续事件转发到该 Channel。
///
/// 转发线程：告警优先（先取告警通道），随后批量排空电平通道（REQ-31：告警不被电平
/// 饿死）；Channel 写入失败即退出（录制侧永不感知，REQ-31）。
#[tauri::command]
pub fn subscribe(app: AppHandle, channel: Channel<EventOut>) -> StateSnapshot {
    let Subscription {
        snapshot,
        alerts,
        levels,
    } = app.state::<CommandsState>().hub.subscribe();
    if let Err(e) = spawn_forwarder(channel, alerts, levels) {
        eprintln!("[commands] Channel 转发线程启动失败：{e}");
    }
    snapshot
}

/// Channel 转发线程：告警优先 + 电平有界批量排空（≤10Hz 节流由 hub 保证）。
///
/// 发送失败（WebView 重载/销毁）只退出转发；旧接收端随之 drop，hub 在下次发布时
/// 回收失效订阅（REQ-31：发送失败不得中断录制）。
fn spawn_forwarder(
    channel: Channel<EventOut>,
    alerts: Receiver<EventOut>,
    levels: Receiver<EventOut>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("sr-tauri-channel".to_string())
        .spawn(move || loop {
            // 1) 告警/状态：独立通道，优先取（50ms 轮询；电平节流上限即 100ms）。
            match alerts.recv_timeout(Duration::from_millis(50)) {
                Ok(event) => {
                    if channel.send(event).is_err() {
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            // 2) 电平：单次最多排空 32 条（有界，避免告警被批量电平延迟）。
            for _ in 0..32 {
                match levels.try_recv() {
                    Ok(event) => {
                        if channel.send(event).is_err() {
                            return;
                        }
                    }
                    Err(_) => break,
                }
            }
        })
        .map(|_| ())
}

// ─────────────────────────── 校验 / 映射 ───────────────────────────

/// 构造带错误码的 invoke 错误文本（`CODE: 明细`；前端按前缀判别）。
pub(crate) fn invalid(code: &str, detail: impl Into<String>) -> String {
    format!("{code}: {}", detail.into())
}

/// 阻塞任务下放到阻塞线程池（长任务不得占用 UI 线程）。
pub(crate) async fn blocking<F, T>(job: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(job).await {
        Ok(result) => result,
        Err(e) => Err(invalid("E_INTERNAL", format!("后台任务执行失败：{e}"))),
    }
}

fn current_settings(app: &AppHandle) -> Settings {
    app.state::<SettingsState>().snapshot()
}

/// M17/REQ-18：恢复收尾执行时一次读取的设置快照（output_dir/keep_segments/
/// output_container 必须来自**同一** Settings 快照；不得扫描时读取、执行时再读）。
/// M19a：窗口标题隐私开关同样进入快照（恢复命名与启动命名口径一致）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FinalizeSessionSettings {
    pub output_dir: String,
    pub keep_segments: bool,
    pub output_container: sr_pipeline::naming::OutputContainer,
    pub include_window_title_in_filename: bool,
}

/// 生产读取边界（命令层与 L1 注入测试共用）：`provider` 只允许调用一次。
pub(crate) fn finalize_session_settings_snapshot(
    mut provider: impl FnMut() -> Settings,
) -> FinalizeSessionSettings {
    let s = provider();
    FinalizeSessionSettings {
        output_dir: s.output_dir,
        keep_segments: s.keep_segments,
        output_container: s.output_container,
        include_window_title_in_filename: s.include_window_title_in_filename,
    }
}

/// P2：设置 → 管线编码器覆盖（Auto → None 走 D4 链；Named → 显式名）。
pub(crate) fn encoder_override(settings: &Settings) -> Option<String> {
    match &settings.encoder {
        EncoderSetting::Auto => None,
        EncoderSetting::Named(name) => Some(name.clone()),
    }
}

/// 显式输出目录；缺省/空白 → 设置中的输出目录（REQ-24）。
pub(crate) fn resolve_output_dir(app: &AppHandle, value: Option<String>) -> String {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| current_settings(app).output_dir)
}

/// `RouteConfigIn` 列表 → 管线配置（校验：非空/画布/fps/输出目录/音轨模式，REQ-17）。
fn plan_routes(routes: Vec<RouteConfigIn>) -> Result<Vec<MultiRouteConfig>, String> {
    if routes.is_empty() {
        return Err(invalid(
            "E_INVALID_CONFIG",
            "start_routes 至少需要 1 路配置",
        ));
    }
    routes
        .into_iter()
        .enumerate()
        .map(|(i, cfg)| to_multi_route(cfg).map_err(|e| format!("{e}（routes[{i}]）")))
        .collect()
}

/// 单路配置映射（`isolated` 不得含系统音源：REQ-17 隔离=目标应用轨[+麦克风]）。
fn to_multi_route(cfg: RouteConfigIn) -> Result<MultiRouteConfig, String> {
    if cfg.video.canvas_w == 0 || cfg.video.canvas_h == 0 {
        return Err(invalid(
            "E_INVALID_CONFIG",
            "video.canvas_w/canvas_h 必须 > 0",
        ));
    }
    if cfg.video.fps == 0 {
        return Err(invalid("E_INVALID_CONFIG", "video.fps 必须 > 0"));
    }
    if cfg.output_dir.trim().is_empty() {
        return Err(invalid("E_INVALID_CONFIG", "output_dir 不得为空"));
    }
    if cfg.audio.mode == AudioMode::Isolated
        && cfg
            .audio
            .sources
            .iter()
            .any(|s| matches!(s, AudioSourceIn::System))
    {
        return Err(invalid(
            "E_INVALID_AUDIO_PLAN",
            "isolated 模式不得包含 system 音源（REQ-17：隔离=目标应用轨[+麦克风]；需要系统轨请用 all_tracks）",
        ));
    }
    // M16/REQ-13 + M18/REQ-23：端点音源适用于显示器目标；区域目标仅全音轨可选
    // （隔离模式 = 仅所选应用，不隐式加入设备混音）；窗口/纯录音拒绝不变。
    let is_region = matches!(cfg.target, TargetIn::Region { .. });
    let is_monitor = matches!(cfg.target, TargetIn::Monitor { .. });
    let endpoint_allowed = is_monitor || (is_region && cfg.audio.mode == AudioMode::AllTracks);
    if !endpoint_allowed
        && cfg
            .audio
            .sources
            .iter()
            .any(|s| matches!(s, AudioSourceIn::RenderEndpoint { .. }))
    {
        return Err(invalid(
            "E_INVALID_AUDIO_PLAN",
            if is_region {
                "区域隔离模式不得包含 render_endpoint 音源（REQ-23：端点仅全音轨可选；隔离=仅所选应用）"
            } else {
                "render_endpoint 音源仅适用于显示器目标（REQ-13：窗口目标隔离限制不变）"
            },
        ));
    }
    // M18/REQ-23：区域隔离模式 = 仅所选应用（不隐式系统/麦克风/端点）。
    if is_region && cfg.audio.mode == AudioMode::Isolated {
        if let Some(kind) = cfg.audio.sources.iter().find_map(|s| match s {
            AudioSourceIn::System => Some("system"),
            AudioSourceIn::Mic { .. } => Some("mic"),
            AudioSourceIn::RenderEndpoint { .. } => Some("render_endpoint"),
            AudioSourceIn::App { .. } => None,
        }) {
            return Err(invalid(
                "E_INVALID_AUDIO_PLAN",
                format!(
                    "区域隔离模式只接受 app 音源（得到 {kind}；REQ-23：隔离=仅所选应用各自独立成轨）"
                ),
            ));
        }
    }
    // M16/REQ-14 + M18/REQ-23：显示器/区域目标零音源 → 启动被拒 + 明确引导。
    if (is_monitor || is_region) && cfg.audio.sources.is_empty() {
        return Err(invalid(
            "E_INVALID_AUDIO_PLAN",
            if is_region {
                "区域目标没有任何音源：请选择至少一个应用（隔离）或系统/麦克风/端点（全音轨）（不静默录制；REQ-23）"
            } else {
                "显示器目标没有任何音源：请勾选该屏输出端点、系统音频或麦克风（不静默录制；REQ-14）"
            },
        ));
    }
    Ok(MultiRouteConfig {
        target: route_target(&cfg.target)?,
        video: route::VideoCfg {
            canvas_w: cfg.video.canvas_w,
            canvas_h: cfg.video.canvas_h,
            fps: cfg.video.fps,
            cursor: cfg.video.cursor,
        },
        audio: cfg.audio.sources.iter().map(audio_source).collect(),
        output_dir: PathBuf::from(cfg.output_dir),
    })
}

/// 纯录音来源映射（REQ-19：解码器/编码器不参与）。
fn plan_audio_only(sources: Vec<AudioSourceIn>) -> Result<Vec<AudioSourceSpec>, String> {
    if sources.is_empty() {
        return Err(invalid(
            "E_INVALID_CONFIG",
            "start_audio_only 至少需要 1 个音源",
        ));
    }
    if sources
        .iter()
        .any(|s| matches!(s, AudioSourceIn::RenderEndpoint { .. }))
    {
        return Err(invalid(
            "E_INVALID_AUDIO_PLAN",
            "纯录音不接受 render_endpoint 音源（REQ-13：端点音源仅适用于显示器目标）",
        ));
    }
    Ok(sources.iter().map(audio_source).collect())
}

fn audio_source(src: &AudioSourceIn) -> AudioSourceSpec {
    match src {
        AudioSourceIn::App {
            pid,
            created_at_unix_ms,
        } => AudioSourceSpec::App {
            pid: *pid,
            created_at_unix_ms: *created_at_unix_ms,
        },
        AudioSourceIn::System => AudioSourceSpec::System,
        AudioSourceIn::Mic { device_id } => AudioSourceSpec::Mic {
            device: device_id.clone(),
        },
        AudioSourceIn::RenderEndpoint { device_id } => AudioSourceSpec::RenderEndpoint {
            device_id: device_id.clone(),
        },
    }
}

fn route_target(target: &TargetIn) -> Result<route::RouteTarget, String> {
    match target {
        TargetIn::Window { hwnd } => {
            if *hwnd == 0 {
                Err(invalid("E_TARGET_LOST", "target.hwnd 不得为 0"))
            } else {
                Ok(route::RouteTarget::Window { hwnd: *hwnd })
            }
        }
        TargetIn::Monitor { index } => Ok(route::RouteTarget::Monitor { index: *index }),
        TargetIn::Region {
            display_device,
            rect,
        } => {
            if rect.w == 0 || rect.h == 0 {
                return Err(invalid(
                    "E_INVALID_REGION",
                    format!("区域尺寸不得为 0（{}x{}）", rect.w, rect.h),
                ));
            }
            let device = display_device.trim();
            if device.is_empty() {
                return Err(invalid(
                    "E_TARGET_LOST",
                    "区域录制缺少 display_device（设备路径与 GDI szDevice 均不可得；拒绝用友好名代替）",
                ));
            }
            Ok(route::RouteTarget::Region {
                display_device: device.to_string(),
                rect: route::RegionRect {
                    x: rect.x,
                    y: rect.y,
                    w: rect.w,
                    h: rect.h,
                },
            })
        }
    }
}

fn shot_target(target: &TargetIn) -> Result<ShotTarget, String> {
    match target {
        TargetIn::Window { hwnd } => {
            if *hwnd == 0 {
                Err(invalid("E_TARGET_LOST", "target.hwnd 不得为 0"))
            } else {
                Ok(ShotTarget::Window { hwnd: *hwnd })
            }
        }
        TargetIn::Monitor { index } => Ok(ShotTarget::Monitor { index: *index }),
        TargetIn::Region { rect, .. } => {
            if rect.w == 0 || rect.h == 0 {
                return Err(invalid(
                    "E_INVALID_REGION",
                    format!("区域尺寸不得为 0（{}x{}）", rect.w, rect.h),
                ));
            }
            Ok(ShotTarget::Region {
                rect: ScreenRect {
                    x: rect.x,
                    y: rect.y,
                    width_px: rect.w,
                    height_px: rect.h,
                },
            })
        }
    }
}

/// 路由错误 → 规格错误码（§3.4/§3.5）。
fn route_error_code(error: &RouteError) -> &'static str {
    match error {
        RouteError::EncoderUnavailable => "E_ENCODER_UNAVAILABLE",
        RouteError::RegionSizeUnsupported { .. } => "E_ENCODER_UNAVAILABLE",
        RouteError::ConsentDenied => "E_CONSENT_DENIED",
        RouteError::SidecarExit { .. } => "E_SIDECAR_EXIT",
        RouteError::WaitTimeout => "E_WAIT_TIMEOUT",
        RouteError::LoopbackUnavailable { .. } => "E_LOOPBACK_UNAVAILABLE",
        RouteError::TargetInvalid { .. } => "E_TARGET_LOST",
        RouteError::CaptureFailed { .. } => "E_CAPTURE_FAILED",
        RouteError::StartFailed { .. } => "E_START_FAILED",
        RouteError::FinalizeFailed { .. } => "E_FINALIZE_FAILED",
        RouteError::DiskFull { .. } => "E_DISK_FULL",
        RouteError::Internal { .. } => "E_INTERNAL",
    }
}

/// 去掉 Display 文本里的 `CODE: ` 前缀（错误码在事件/错误串里已单列）。
fn strip_code_prefix(text: &str) -> String {
    match text.split_once(": ") {
        Some((prefix, rest)) if prefix.starts_with("E_") => rest.to_string(),
        _ => text.to_string(),
    }
}

/// 路由错误消息（去前缀；事件里 code 已单列）。
fn route_error_message(error: &RouteError) -> String {
    strip_code_prefix(&error.to_string())
}

pub(crate) fn shot_error_string(error: ShotError) -> String {
    let code = match &error {
        ShotError::TargetMinimized => "E_TARGET_MINIMIZED",
        ShotError::TargetInvalid { .. } | ShotError::MonitorIndexOutOfRange { .. } => {
            "E_TARGET_LOST"
        }
        ShotError::EmptyRegion { .. } => "E_INVALID_REGION",
        ShotError::CaptureUnavailable { .. } => "E_CAPTURE_UNAVAILABLE",
        ShotError::CaptureTimeout { .. } => "E_CAPTURE_TIMEOUT",
        ShotError::OutputWriteFailed { .. } => "E_OUTPUT_UNAVAILABLE",
        ShotError::Internal { .. } => "E_INTERNAL",
    };
    invalid(code, strip_code_prefix(&error.to_string()))
}

fn merge_error_string(error: MergeError) -> String {
    match error {
        MergeError::InvalidInput { input, detail } => invalid(
            "E_MERGE_INCOMPATIBLE",
            format!("输入不可用（{}）：{detail}", input.display()),
        ),
        MergeError::OutputUnavailable { out, detail } => invalid(
            "E_OUTPUT_UNAVAILABLE",
            format!("输出不可用（{}）：{detail}", out.display()),
        ),
        MergeError::SidecarFailed { detail } => {
            invalid("E_SIDECAR_EXIT", format!("合并失败：{detail}"))
        }
        MergeError::Cancelled { detail } => invalid("E_CANCELLED", detail),
    }
}

fn recovery_error_string(error: &recovery::RecoveryError) -> String {
    match error {
        recovery::RecoveryError::NotFound { dir } => invalid(
            "E_SESSION_NOT_FOUND",
            format!("会话不存在：{}", dir.display()),
        ),
        recovery::RecoveryError::Active { session_id } => invalid(
            "E_SESSION_ACTIVE",
            format!("会话仍在写入（拒绝收尾）：{session_id}"),
        ),
        recovery::RecoveryError::NoUsableSegments { detail } => {
            invalid("E_RECOVERY_FAILED", detail)
        }
        recovery::RecoveryError::RecoveryFailed { detail } => invalid("E_RECOVERY_FAILED", detail),
    }
}

/// 会话 id 合法性（只允许单一路径分量；收尾/丢弃会删改磁盘，禁止穿越）。
fn validate_session_id(session_id: &str) -> Result<(), String> {
    let bad = session_id.is_empty()
        || session_id == "."
        || session_id.contains("..")
        || session_id.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        });
    if bad {
        return Err(invalid(
            "E_INVALID_SESSION_ID",
            format!("非法会话 id：{session_id:?}"),
        ));
    }
    Ok(())
}

/// 会话目录（`<输出目录>/.screenrec_tmp/<session_id>`，REQ-24/D12）。
fn session_dir_for(app: &AppHandle, session_id: &str) -> Result<PathBuf, String> {
    let output_dir = current_settings(app).output_dir;
    session_dir_for_output(&output_dir, session_id)
}

/// 会话目录（显式输出目录快照；M17/REQ-18：恢复收尾与 output_dir 同一次读取）。
fn session_dir_for_output(output_dir: &str, session_id: &str) -> Result<PathBuf, String> {
    validate_session_id(session_id)?;
    Ok(naming::session_tmp_dir(Path::new(output_dir), session_id))
}

fn is_terminal(state: &str) -> bool {
    state == STATE_FINALIZED || state == STATE_FAILED
}

fn session_view(session: recovery::RecoverableSession) -> SessionOut {
    let recovery::RecoverableSession {
        session_id,
        dir,
        manifest,
        sealed_segments,
        tail_segment,
        active,
    } = session;
    let routes = manifest
        .as_ref()
        .map(|m| {
            m.routes
                .iter()
                .map(|r| SessionRouteOut {
                    route_id: r.route_id.clone(),
                    name: r.identity.name.clone(),
                    streams: r.streams.clone(),
                    state: r.state.clone(),
                    segments: r.segments.len(),
                    final_path: r
                        .final_path
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned()),
                })
                .collect()
        })
        .unwrap_or_default();
    SessionOut {
        session_id,
        dir: dir.to_string_lossy().into_owned(),
        created_at: manifest.as_ref().map(|m| m.created_at_unix_ms).unwrap_or(0),
        active,
        manifest_ok: manifest.is_some(),
        routes,
        sealed_segments: sealed_segments
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        tail_segment: tail_segment
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
    }
}

fn probe_view(caps: ProbeCapabilities) -> ProbeOut {
    let ProbeCapabilities {
        os_build,
        os_supported,
        encoders,
        selected_encoder,
        loopback,
        mic,
        wgc,
        canvas_limits,
    } = caps;
    ProbeOut {
        os_build,
        os_supported,
        encoders: encoders
            .into_iter()
            .map(|e| EncoderOut {
                name: e.name,
                init_ok: e.init_ok,
                detail: e.detail,
            })
            .collect(),
        selected_encoder,
        loopback_ok: loopback.status == LayerStatus::Ok,
        mic_privacy: mic.status == LayerStatus::Ok,
        wgc_ok: wgc.status == LayerStatus::Ok,
        canvas_limits: CanvasLimitsOut {
            max_width: canvas_limits.max_width,
            max_height: canvas_limits.max_height,
        },
        layers: LayersOut {
            loopback: LayerOut {
                status: layer_status(loopback.status).to_string(),
                detail: loopback.detail,
            },
            mic: LayerOut {
                status: layer_status(mic.status).to_string(),
                detail: mic.detail,
            },
            wgc: LayerOut {
                status: layer_status(wgc.status).to_string(),
                detail: wgc.detail,
            },
        },
    }
}

fn layer_status(status: LayerStatus) -> &'static str {
    match status {
        LayerStatus::Ok => "ok",
        LayerStatus::Denied => "denied",
        LayerStatus::Timeout => "timeout",
        LayerStatus::Canceled => "canceled",
        LayerStatus::Unavailable => "unavailable",
        LayerStatus::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P2：设置编码器 → 管线覆盖值的映射（Auto → None；Named 原样透传）。
    #[test]
    fn encoder_setting_maps_to_pipeline_override() {
        let mut s = Settings::default();
        assert_eq!(encoder_override(&s), None, "Auto 应走 D4 探测链");
        s.encoder = EncoderSetting::Named("h264_mf".to_string());
        assert_eq!(
            encoder_override(&s).as_deref(),
            Some("h264_mf"),
            "显式编码器应透传"
        );
    }

    /// P2：托盘「开始」在无最近配置时保持显式空态（命令层据此给出明确提示）。
    #[test]
    fn tray_start_without_selection_is_explicitly_empty() {
        let state = CommandsState::new();
        assert!(state.last_routes().is_none(), "无历史启动 → 无选中目标");
        assert!(!state.has_active_routes(), "初始无活动路由");
    }

    /// M17/REQ-18：恢复收尾执行时只读取一次设置快照；三字段必须来自同一快照。
    #[test]
    fn unit_commands_recovery_settings_snapshot_once() {
        use sr_pipeline::naming::OutputContainer;
        use std::cell::Cell;

        let calls = Cell::new(0usize);
        let mut first = Settings::default();
        first.output_dir = r"D:\first".to_string();
        first.keep_segments = true;
        first.output_container = OutputContainer::Mp4;
        first.include_window_title_in_filename = true;
        let mut second = Settings::default();
        second.output_dir = r"E:\second".to_string();
        second.keep_segments = false;
        second.output_container = OutputContainer::Mkv;
        second.include_window_title_in_filename = false;

        let snap = finalize_session_settings_snapshot(|| {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                first.clone()
            } else {
                second.clone()
            }
        });
        assert_eq!(
            calls.get(),
            1,
            "REQ-18：Settings 快照提供者总共只能调用一次（扫描/执行不得各读一次）"
        );
        assert_eq!(
            snap,
            FinalizeSessionSettings {
                output_dir: r"D:\first".to_string(),
                keep_segments: true,
                output_container: OutputContainer::Mp4,
                include_window_title_in_filename: true,
            },
            "REQ-18：output_dir/keep_segments/output_container（及 M19a 隐私开关）必须来自同一快照"
        );
    }

    #[test]
    fn route_config_in_parses_window_isolated_json() {
        let json = r#"{
            "target": { "kind": "window", "hwnd": 66052 },
            "video": { "canvas_w": 1920, "canvas_h": 1080, "fps": 30, "cursor": true },
            "audio": {
                "mode": "isolated",
                "sources": [
                    { "kind": "app", "pid": 1234, "created_at_unix_ms": 1700000000000 }
                ]
            },
            "output_dir": "C:\\Rec"
        }"#;
        let cfg: RouteConfigIn = serde_json::from_str(json).expect("window/isolated JSON 应可解析");
        assert_eq!(cfg.target, TargetIn::Window { hwnd: 66052 });
        assert_eq!(cfg.audio.mode, AudioMode::Isolated);
        let planned = plan_routes(vec![cfg]).expect("计划应合法");
        assert_eq!(planned.len(), 1);
        assert_eq!(
            planned[0].target,
            route::RouteTarget::Window { hwnd: 66052 }
        );
        assert_eq!(planned[0].audio.len(), 1, "isolated 应恰 1 条应用轨");
        match &planned[0].audio[0] {
            AudioSourceSpec::App {
                pid,
                created_at_unix_ms,
            } => {
                assert_eq!(*pid, 1234);
                assert_eq!(*created_at_unix_ms, 1_700_000_000_000);
            }
            other => panic!("应映射为 App 音源（得到 {other:?}）"),
        }
        assert_eq!(planned[0].video.canvas_w, 1920);
        assert_eq!(planned[0].video.fps, 30);
    }

    #[test]
    fn route_config_in_parses_monitor_all_tracks_json() {
        let json = r#"{
            "target": { "kind": "monitor", "index": 0 },
            "video": { "canvas_w": 1280, "canvas_h": 720, "fps": 60 },
            "audio": {
                "mode": "all_tracks",
                "sources": [
                    { "kind": "system" },
                    { "kind": "mic", "device_id": "mic-1" }
                ]
            },
            "output_dir": "D:\\out"
        }"#;
        let cfg: RouteConfigIn =
            serde_json::from_str(json).expect("monitor/all_tracks JSON 应可解析");
        assert!(cfg.video.cursor, "cursor 缺省应为 true（REQ-11 默认开）");
        let planned = plan_routes(vec![cfg]).expect("计划应合法");
        assert_eq!(planned[0].target, route::RouteTarget::Monitor { index: 0 });
        assert_eq!(planned[0].audio.len(), 2, "all_tracks 应保留 2 条独立轨");
        assert!(
            matches!(&planned[0].audio[0], AudioSourceSpec::System),
            "第 1 轨应为系统音源"
        );
        match &planned[0].audio[1] {
            AudioSourceSpec::Mic { device } => assert_eq!(device, "mic-1"),
            other => panic!("第 2 轨应为麦克风（得到 {other:?}）"),
        }
    }

    #[test]
    fn route_config_in_parses_render_endpoint_json() {
        let json = r#"{
            "target": { "kind": "monitor", "index": 0 },
            "video": { "canvas_w": 1280, "canvas_h": 720, "fps": 30 },
            "audio": {
                "mode": "all_tracks",
                "sources": [
                    { "kind": "render_endpoint", "device_id": "ep-display" },
                    { "kind": "system" }
                ]
            },
            "output_dir": "C:\\out"
        }"#;
        let cfg: RouteConfigIn = serde_json::from_str(json).expect("render_endpoint JSON 可解析");
        let planned = plan_routes(vec![cfg]).expect("显示器 + 端点计划应合法");
        assert_eq!(
            planned[0].audio[0],
            AudioSourceSpec::RenderEndpoint {
                device_id: "ep-display".into()
            },
            "REQ-11：JSON kind=render_endpoint → AudioSourceSpec::RenderEndpoint"
        );
        assert_eq!(planned[0].audio.len(), 2);
    }

    #[test]
    fn invalid_plans_are_rejected_with_codes() {
        let base = RouteConfigIn {
            target: TargetIn::Monitor { index: 0 },
            video: VideoCfgIn {
                canvas_w: 640,
                canvas_h: 360,
                fps: 30,
                cursor: false,
            },
            audio: AudioPlanIn {
                mode: AudioMode::Isolated,
                sources: vec![AudioSourceIn::System],
            },
            output_dir: "C:\\out".to_string(),
        };
        let err = plan_routes(vec![base.clone()]).expect_err("isolated + system 应被拒绝");
        assert!(err.contains("E_INVALID_AUDIO_PLAN"), "得到 {err}");

        let err = plan_routes(Vec::new()).expect_err("空批次应被拒绝");
        assert!(err.contains("E_INVALID_CONFIG"), "得到 {err}");

        let mut zero_fps = base.clone();
        zero_fps.audio.sources.clear();
        zero_fps.video.fps = 0;
        let err = plan_routes(vec![zero_fps]).expect_err("fps=0 应被拒绝");
        assert!(err.contains("E_INVALID_CONFIG"), "得到 {err}");

        let mut zero_hwnd = base;
        zero_hwnd.audio.sources.clear();
        zero_hwnd.target = TargetIn::Window { hwnd: 0 };
        let err = plan_routes(vec![zero_hwnd]).expect_err("hwnd=0 应被拒绝");
        assert!(err.contains("E_TARGET_LOST"), "得到 {err}");

        let err = plan_audio_only(Vec::new()).expect_err("纯录音空来源应被拒绝");
        assert!(err.contains("E_INVALID_CONFIG"), "得到 {err}");
    }

    #[test]
    fn event_payload_shapes_match_frontend_contract() {
        let state = EventOut::RouteState {
            id: "route-1".to_string(),
            state: "recording".to_string(),
            stop_notice: None,
        };
        assert_eq!(
            serde_json::to_value(&state).expect("serialize"),
            serde_json::json!({ "event": "route_state", "id": "route-1", "state": "recording" })
        );

        let level = EventOut::RouteLevel {
            id: "route-1".to_string(),
            track: "a0".to_string(),
            rms: 0.25,
        };
        assert_eq!(
            serde_json::to_value(&level).expect("serialize"),
            serde_json::json!({ "event": "route_level", "id": "route-1", "track": "a0", "rms": 0.25 })
        );

        let error = EventOut::RouteError {
            id: "route-2".to_string(),
            code: "E_TARGET_LOST".to_string(),
            message: "窗口已关闭".to_string(),
            severity: "error".to_string(),
            stop_notice: None,
        };
        assert_eq!(
            serde_json::to_value(&error).expect("serialize"),
            serde_json::json!({
                "event": "route_error",
                "id": "route-2",
                "code": "E_TARGET_LOST",
                "message": "窗口已关闭",
                "severity": "error"
            })
        );

        let finalized = EventOut::RouteFinalized {
            id: "route-3".to_string(),
            path: "C:\\out\\a.mkv".to_string(),
            stop_notice: None,
        };
        assert_eq!(
            serde_json::to_value(&finalized).expect("serialize"),
            serde_json::json!({ "event": "route_finalized", "id": "route-3", "path": "C:\\out\\a.mkv" })
        );

        let session = SessionOut {
            session_id: "sess-1".to_string(),
            dir: "C:\\out\\.screenrec_tmp\\sess-1".to_string(),
            created_at: 1,
            active: false,
            manifest_ok: true,
            routes: Vec::new(),
            sealed_segments: Vec::new(),
            tail_segment: None,
        };
        let found = EventOut::RecoveryFound { session };
        let value = serde_json::to_value(&found).expect("serialize");
        assert_eq!(value["event"], "recovery_found");
        assert_eq!(value["session"]["session_id"], "sess-1");
    }

    #[test]
    fn route_error_code_and_message_are_separated() {
        let e = RouteError::TargetInvalid {
            detail: "窗口已关闭".to_string(),
        };
        assert_eq!(route_error_code(&e), "E_TARGET_LOST");
        assert_eq!(route_error_message(&e), "窗口已关闭");

        let e = RouteError::EncoderUnavailable;
        assert_eq!(route_error_code(&e), "E_ENCODER_UNAVAILABLE");
        assert_eq!(route_error_message(&e), "无可用编码器");
    }

    #[test]
    fn session_id_validation_rejects_traversal_and_separators() {
        assert!(validate_session_id("20261017_101500_0").is_ok());
        for bad in [
            "", ".", "..", "../evil", "a/b", "a\\b", "C:evil", "a*b", "a\nb",
        ] {
            assert!(validate_session_id(bad).is_err(), "{bad:?} 应被拒绝");
        }
    }

    #[test]
    fn snapshot_round_trips_route_state() {
        let snapshot = StateSnapshot {
            routes: vec![RouteStateOut {
                id: "route-1".to_string(),
                state: "finalized".to_string(),
                final_path: Some("C:\\out\\x.mkv".to_string()),
                stop_notice: None,
            }],
            recovery: Vec::new(),
        };
        let value = serde_json::to_value(&snapshot).expect("serialize");
        assert_eq!(value["routes"][0]["id"], "route-1");
        assert_eq!(value["routes"][0]["state"], "finalized");
        assert_eq!(value["routes"][0]["final_path"], "C:\\out\\x.mkv");
        assert_eq!(value["recovery"], serde_json::json!([]));
    }

    /// REQ-31：订阅入口返回快照；告警与电平经同一 Channel 送达且互不阻塞。
    #[test]
    fn channel_forwarder_delivers_alerts_and_levels() {
        let hub: ChannelHub<StateSnapshot, EventOut, EventOut> = ChannelHub::new(StateSnapshot {
            routes: vec![RouteStateOut {
                id: "route-0".to_string(),
                state: STATE_RECORDING.to_string(),
                final_path: None,
                stop_notice: None,
            }],
            recovery: Vec::new(),
        });
        let subscription = hub.subscribe();
        assert_eq!(subscription.snapshot.routes.len(), 1, "订阅应携带当前快照");

        let received: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = received.clone();
        let channel = Channel::<EventOut>::new(move |body| {
            let json = match body {
                tauri::ipc::InvokeResponseBody::Json(json) => json,
                _ => return Ok(()),
            };
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) {
                sink.lock().expect("接收端锁").push(value);
            }
            Ok(())
        });
        spawn_forwarder(channel, subscription.alerts, subscription.levels).expect("转发线程应启动");

        hub.publish_level(EventOut::RouteLevel {
            id: "route-1".to_string(),
            track: "a0".to_string(),
            rms: 0.5,
        });
        hub.publish_alert(EventOut::RouteState {
            id: "route-1".to_string(),
            state: STATE_RECORDING.to_string(),
            stop_notice: None,
        });

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let events = received.lock().expect("接收端锁").clone();
            let has_level = events.iter().any(|e| e["event"] == "route_level");
            let has_state = events.iter().any(|e| e["event"] == "route_state");
            if has_level && has_state {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "告警与电平都应经 Channel 送达（得到 {events:?}）"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    // ── M16/REQ-11/13：音源校验与端点拒绝矩阵 ──

    fn base_route(target: TargetIn, mode: AudioMode, sources: Vec<AudioSourceIn>) -> RouteConfigIn {
        RouteConfigIn {
            target,
            video: VideoCfgIn {
                canvas_w: 640,
                canvas_h: 360,
                fps: 30,
                cursor: false,
            },
            audio: AudioPlanIn { mode, sources },
            output_dir: "C:\\out".to_string(),
        }
    }

    #[test]
    fn unit_commands_audio_source_validation_render_endpoint() {
        let endpoint = AudioSourceIn::RenderEndpoint {
            device_id: "ep-display".to_string(),
        };
        // 显示器 + endpoint：接受并映射为 RenderEndpoint 音源（实际勾选）。
        let planned = plan_routes(vec![base_route(
            TargetIn::Monitor { index: 0 },
            AudioMode::AllTracks,
            vec![endpoint.clone()],
        )])
        .expect("显示器 + 端点应被接受");
        assert_eq!(
            planned[0].audio,
            vec![AudioSourceSpec::RenderEndpoint {
                device_id: "ep-display".into()
            }]
        );

        // 窗口目标 + endpoint：拒绝（REQ-13：窗口目标的隔离限制不变）。
        let err = plan_routes(vec![base_route(
            TargetIn::Window { hwnd: 42 },
            AudioMode::AllTracks,
            vec![endpoint.clone()],
        )])
        .expect_err("窗口 + 端点必须拒绝");
        assert!(err.contains("E_INVALID_AUDIO_PLAN"), "得到 {err}");

        // 纯录音 + endpoint：拒绝（REQ-13）。
        let err = plan_audio_only(vec![endpoint]).expect_err("纯录音不接受端点音源");
        assert!(err.contains("E_INVALID_AUDIO_PLAN"), "得到 {err}");

        // 显示器零音源：拒绝 + 端点选项引导（REQ-14）。
        let err = plan_routes(vec![base_route(
            TargetIn::Monitor { index: 0 },
            AudioMode::AllTracks,
            Vec::new(),
        )])
        .expect_err("显示器零音源必须拒绝");
        assert!(err.contains("E_INVALID_AUDIO_PLAN"), "得到 {err}");
        assert!(
            err.contains("端点") && err.contains("系统"),
            "提示应包含端点/系统音源选项引导（得到 {err}）"
        );

        // 命令层保留 endpoint + system 两条（去重发生在管线启动、按实际 ID）。
        let planned = plan_routes(vec![base_route(
            TargetIn::Monitor { index: 0 },
            AudioMode::AllTracks,
            vec![
                AudioSourceIn::RenderEndpoint {
                    device_id: "ep-display".into(),
                },
                AudioSourceIn::System,
            ],
        )])
        .expect("端点 + 系统应被接受");
        assert_eq!(
            planned[0].audio.len(),
            2,
            "REQ-13：去重时机在管线启动（按实际 device_id），命令层不得提前丢弃"
        );
    }

    #[test]
    fn unit_commands_isolated_system_still_rejected() {
        let err = plan_routes(vec![base_route(
            TargetIn::Window { hwnd: 42 },
            AudioMode::Isolated,
            vec![AudioSourceIn::System],
        )])
        .expect_err("isolated + system 拒绝行为不变");
        assert!(err.contains("E_INVALID_AUDIO_PLAN"), "得到 {err}");
    }

    // ── M16/REQ-12：快照/终态串行化与 stop_notice 载荷 ──

    fn video_finalized(path: &str, notice: Option<StopNotice>) -> RouteOutcome {
        RouteOutcome::Video(route::Finalized {
            final_path: PathBuf::from(path),
            segment_list: PathBuf::from("C:\\out\\segments.ffconcat"),
            segments: Vec::new(),
            stop_notice: notice,
        })
    }

    fn insert_recording(state: &CommandsState, id: &str) {
        state.routes.lock().expect("路由表锁").insert(
            id.to_string(),
            RouteSlot {
                handle: None,
                state: STATE_RECORDING.to_string(),
                final_path: None,
                stop_notice: None,
            },
        );
    }

    #[test]
    fn unit_commands_terminal_snapshot_and_event_carry_stop_notice() {
        let state = CommandsState::new();
        insert_recording(&state, "route-1");
        state.publish_snapshot();
        let sub = state.hub.subscribe();

        let notice = StopNotice {
            code: "E_TARGET_LOST".to_string(),
            endpoint: Some("DELL U2720Q".to_string()),
            detail: "设备已移除".to_string(),
        };
        state.mark_finalized(
            &["route-1".to_string()],
            &video_finalized("C:\\out\\a.mkv", Some(notice.clone())),
        );

        let snapshot = state.hub.subscribe().snapshot;
        let route = snapshot
            .routes
            .iter()
            .find(|r| r.id == "route-1")
            .expect("快照应含 route-1");
        assert_eq!(route.state, STATE_FINALIZED);
        assert_eq!(
            route.stop_notice.as_ref().map(|n| n.code.as_str()),
            Some("E_TARGET_LOST"),
            "REQ-12：stop_notice 必须进入快照（成功收尾分支）"
        );
        assert_eq!(
            route
                .stop_notice
                .as_ref()
                .and_then(|n| n.endpoint.as_deref()),
            Some("DELL U2720Q")
        );
        match sub
            .alerts
            .recv_timeout(Duration::from_secs(1))
            .expect("应收到终态事件")
        {
            EventOut::RouteFinalized {
                id, stop_notice, ..
            } => {
                assert_eq!(id, "route-1");
                assert_eq!(
                    stop_notice.as_ref().map(|n| n.code.as_str()),
                    Some("E_TARGET_LOST"),
                    "REQ-12：终态事件载荷应携带 stop_notice"
                );
            }
            other => panic!("应 route_finalized（得到 {other:?}）"),
        }
    }

    #[test]
    fn unit_commands_snapshot_commit_is_serialized_with_terminal_transition() {
        let state = Arc::new(CommandsState::new());
        insert_recording(&state, "route-1");
        state.publish_snapshot();

        // A：取得快照生成权（旧快照），尚未提交。
        let guard = state.snapshot_lock.lock().expect("快照串行锁");
        let stale = state.build_snapshot();
        assert_eq!(stale.routes[0].state, STATE_RECORDING);

        // B：终态提交必须等待 A 完成（同一锁内完成快照生成 + Hub 提交）。
        let (tx, rx) = std::sync::mpsc::channel();
        let state_b = state.clone();
        let worker = std::thread::spawn(move || {
            state_b.mark_finalized(
                &["route-1".to_string()],
                &video_finalized("C:\\out\\b.mkv", None),
            );
            let _ = tx.send(());
        });
        assert!(
            rx.recv_timeout(Duration::from_millis(150)).is_err(),
            "REQ-12：A 持有快照生成权时，B 的终态提交不得交错完成"
        );

        drop(guard); // A 放弃旧快照（不得再提交）
        rx.recv_timeout(Duration::from_secs(2))
            .expect("B 应在 A 释放后完成终态提交");
        worker.join().expect("B 线程");
        assert_eq!(
            state.hub.subscribe().snapshot.routes[0].state,
            STATE_FINALIZED,
            "REQ-12：旧快照不得回退缓存；新订阅必须取得终态"
        );
    }

    // ── M18/REQ-22：区域 IPC DTO / 坐标 / 拒绝矩阵 ──

    #[test]
    fn unit_region_ipc_dto_old_screenshot_json_compatible() {
        // 旧截图 JSON（只有 rect）必须继续可反序列化并走截图映射。
        let old: TargetIn =
            serde_json::from_str(r#"{"kind":"region","rect":{"x":-100,"y":20,"w":300,"h":200}}"#)
                .expect("旧截图 JSON 应兼容");
        assert_eq!(
            old,
            TargetIn::Region {
                display_device: String::new(),
                rect: RectIn {
                    x: -100,
                    y: 20,
                    w: 300,
                    h: 200
                }
            }
        );
        assert!(
            shot_target(&old).is_ok(),
            "旧截图 JSON 的截图入口不得要求 display_device"
        );
        let err = route_target(&old).expect_err("录制入口必须强制校验 display_device");
        assert!(err.contains("E_TARGET_LOST"), "得到 {err}");

        // 新录制 JSON：display_device 进入管线目标，rect 不再丢弃。
        let new: TargetIn = serde_json::from_str(
            r#"{"kind":"region","display_device":"\\\\?\\DISPLAY#A#1","rect":{"x":10,"y":20,"w":450,"h":300}}"#,
        )
        .expect("区域录制 JSON 应解析");
        let mapped = route_target(&new).expect("有效 display_device 应映射");
        assert_eq!(
            mapped,
            route::RouteTarget::Region {
                display_device: r"\\?\DISPLAY#A#1".to_string(),
                rect: route::RegionRect {
                    x: 10,
                    y: 20,
                    w: 450,
                    h: 300
                }
            }
        );
    }

    #[test]
    fn unit_region_desktop_to_frame_crop_formula_and_bounds() {
        // 桌面 rect − 显示器原点（左上）；非零偏移/负原点。
        let crop = route::region_crop_rect(
            route::RegionRect {
                x: -1200,
                y: 300,
                w: 401,
                h: 301,
            },
            -1280,
            100,
            1280,
            1024,
        )
        .expect("单屏内矩形应映射");
        assert_eq!(
            (crop.start_x, crop.start_y, crop.end_x, crop.end_y),
            (80, 200, 480, 500)
        );
        let err = route::region_crop_rect(
            route::RegionRect {
                x: -1200,
                y: 300,
                w: 200,
                h: 900,
            },
            -1280,
            100,
            1280,
            1024,
        )
        .expect_err("终点越界必须拒绝");
        assert!(err.contains("越界"), "得到 {err}");
        let err = route::region_crop_rect(
            route::RegionRect {
                x: 10,
                y: 10,
                w: 63,
                h: 100,
            },
            0,
            0,
            1920,
            1080,
        )
        .expect_err("<64×64 必须拒绝");
        assert!(err.contains("过小"), "得到 {err}");
    }

    #[test]
    fn unit_commands_source_rejection_region_matrix() {
        let app = AudioSourceIn::App {
            pid: 1234,
            created_at_unix_ms: 1_700_000_000_000,
        };
        let endpoint = AudioSourceIn::RenderEndpoint {
            device_id: "ep-display".to_string(),
        };
        let region = || TargetIn::Region {
            display_device: r"\\.\DISPLAY1".to_string(),
            rect: RectIn {
                x: 0,
                y: 0,
                w: 400,
                h: 300,
            },
        };

        // 隔离：仅 app 接受。
        let planned = plan_routes(vec![base_route(
            region(),
            AudioMode::Isolated,
            vec![app.clone()],
        )])
        .expect("区域隔离 + app 应接受");
        assert_eq!(planned[0].audio.len(), 1);

        // 隔离：system/mic/endpoint 一律拒绝（防设备混音泄漏）。
        for (src, needle) in [
            (AudioSourceIn::System, "system"),
            (
                AudioSourceIn::Mic {
                    device_id: "mic-1".to_string(),
                },
                "mic",
            ),
            (endpoint.clone(), "render_endpoint"),
        ] {
            let err = plan_routes(vec![base_route(region(), AudioMode::Isolated, vec![src])])
                .expect_err("区域隔离只接受 app");
            assert!(err.contains("E_INVALID_AUDIO_PLAN"), "得到 {err}");
            assert!(err.contains(needle), "提示应指出 {needle}（得到 {err}）");
        }

        // 全音轨：endpoint 可选（与系统/应用并列）。
        let planned = plan_routes(vec![base_route(
            region(),
            AudioMode::AllTracks,
            vec![endpoint.clone(), AudioSourceIn::System, app],
        )])
        .expect("区域全音轨 + 端点应接受");
        assert_eq!(planned[0].audio.len(), 3);

        // 区域零音源拒绝。
        let err = plan_routes(vec![base_route(region(), AudioMode::Isolated, Vec::new())])
            .expect_err("区域零音源必须拒绝");
        assert!(err.contains("E_INVALID_AUDIO_PLAN"), "得到 {err}");

        // 窗口/纯录音端点拒绝矩阵保持（REQ-13）。
        let err = plan_routes(vec![base_route(
            TargetIn::Window { hwnd: 42 },
            AudioMode::AllTracks,
            vec![endpoint.clone()],
        )])
        .expect_err("窗口 + 端点必须拒绝");
        assert!(err.contains("E_INVALID_AUDIO_PLAN"), "得到 {err}");
        let err = plan_audio_only(vec![endpoint]).expect_err("纯录音不接受端点");
        assert!(err.contains("E_INVALID_AUDIO_PLAN"), "得到 {err}");
    }
}
