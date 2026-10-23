//! Screen_rec Tauri 主进程（M1-D：REQ-30 外壳功能；M9：§3.3 命令接线）。
//!
//! 本轮接线（specs/001-screen-recorder.md §2.8 REQ-30/REQ-31、§3.3 commands）：
//! - 设置持久化：`get_settings`/`set_settings`（自管 JSON 原子写，见 [`settings`]）；
//! - 命令面：枚举/录制/停止/纯录音/截图/合并/恢复/探测与 Channel 订阅见 [`commands`]
//!   （REQ-31：订阅返回状态快照，事件与电平分通道；长任务不占用 UI 线程）；
//! - 单实例：tauri-plugin-single-instance，二次启动聚焦已有主窗口；
//! - 系统托盘：占位图标 + 菜单（显示/开始/停止/退出）；「开始」复用最近一次成功配置
//!   （无目标选择时以原生提示明确要求先在主窗口选择目标），「停止」= stop_all 语义；
//! - 关闭主窗口：默认不退出进程（隐藏到托盘）。REQ-30 的「录制中关闭不中断录制」
//!   语义由录制会话在 [`commands`] 的后台线程持有（UI 线程只发命令，不持有时序）。
//!
//! 说明：HWND/PID 不作为长期身份的约束由会话清单（PID+创建时间+路径，sr-pipeline
//! recovery 维护）落实。

pub mod commands;
pub mod region;
pub mod settings;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{TrayIcon, TrayIconBuilder},
    AppHandle, Manager, State,
};

/// 全局设置状态（命令读写；写入时先落盘后更新内存）。
pub(crate) struct SettingsState(pub(crate) Mutex<settings::Settings>);

impl SettingsState {
    /// 当前设置快照（命令层只读访问；锁内拷贝，避免跨线程持有）。
    pub(crate) fn snapshot(&self) -> settings::Settings {
        self.0.lock().expect("settings lock").clone()
    }
}

/// §3.3：`get_settings()`——返回当前设置快照。
#[tauri::command]
fn get_settings(state: State<'_, SettingsState>) -> settings::Settings {
    state.snapshot()
}

/// §3.3：`set_settings(Settings)`——归一化 → 校验 → 原子落盘 → 更新内存。
/// 失败（校验/写盘）时内存保持不变，返回错误消息给前端。
#[tauri::command]
fn set_settings(
    app: AppHandle,
    state: State<'_, SettingsState>,
    settings: settings::Settings,
) -> Result<(), String> {
    let dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("解析设置目录失败: {e}"))?;
    persist_settings(&dir, &state, settings)
}

/// `set_settings` 同步实现（归一化 → 校验 → 落盘 → 更新内存）。
///
/// REQ-1：`audio_sample_rate` 与磁盘加载复用同一归一化（`Settings::normalize`）；
/// 写入的新值只影响后续启动快照，已取得的快照（`SettingsState::snapshot`）不变。
pub(crate) fn persist_settings(
    dir: &std::path::Path,
    state: &SettingsState,
    mut settings: settings::Settings,
) -> Result<(), String> {
    settings.normalize();
    settings.validate()?;
    settings::save(dir, &settings).map_err(|e| format!("写入设置失败: {e}"))?;
    *state.0.lock().expect("settings lock") = settings;
    Ok(())
}

/// 显示并聚焦主窗口（单实例二次启动 / 托盘「显示」共用）。
fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

// ─────────────────────── M19b1/REQ-1：托盘/主窗状态（纯函数 + 1s 轮询） ───────────────────────

/// 托盘/主窗状态映射结果（纯函数输出；不包含任何 Tauri 句柄）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayStatus {
    pub tooltip: String,
    pub start_enabled: bool,
    pub stop_enabled: bool,
    pub title: String,
}

/// M19b1/REQ-1：按 `recording`/`stopping` 路数生成托盘 tooltip、菜单 enabled 与主窗标题。
///
/// 四态契约见 SDD 004 REQ-1；不新增图标资源，不使用 Windows 不支持的
/// `TrayIcon::set_title`（主窗标题走 `Window::set_title`）。
pub fn tray_status(recording: usize, stopping: usize) -> TrayStatus {
    match (recording, stopping) {
        (0, 0) => TrayStatus {
            tooltip: "Screen_rec".to_string(),
            start_enabled: true,
            stop_enabled: false,
            title: "Screen_rec".to_string(),
        },
        (r, 0) => TrayStatus {
            tooltip: format!("Screen_rec — 录制中 ({r} 路)"),
            start_enabled: false,
            stop_enabled: true,
            title: format!("● 录制中 ({r}) — Screen_rec"),
        },
        (0, s) => TrayStatus {
            tooltip: format!("Screen_rec — 正在收尾 ({s} 路)"),
            start_enabled: false,
            stop_enabled: false,
            title: format!("收尾中 ({s}) — Screen_rec"),
        },
        (r, s) => TrayStatus {
            tooltip: format!("Screen_rec — 录制中 ({r} 路) · 收尾中 ({s} 路)"),
            start_enabled: false,
            stop_enabled: true,
            title: format!("● 录制中 ({r}) · 收尾中 ({s}) — Screen_rec"),
        },
    }
}

/// M19b1/REQ-2：主窗关闭动作（纯函数输出）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseMainAction {
    /// 空闲 / 已提示过：静默隐藏。
    HideSilent,
    /// 录制中首次关闭：后台弹一次提示。
    PromptOnce,
    /// 提示进行中：只 `prevent_close`，不重复弹框。
    Ignore,
}

/// M19b1/REQ-2：关闭主窗四分支。
///
/// `shown`（本进程已成功提示过）优先于 `pending`：一旦提示成功，后续关闭一律静默
/// 隐藏；`active=false`（空闲）始终静默隐藏。
pub fn close_main_action(active: bool, shown: bool, pending: bool) -> CloseMainAction {
    if !active || shown {
        CloseMainAction::HideSilent
    } else if pending {
        CloseMainAction::Ignore
    } else {
        CloseMainAction::PromptOnce
    }
}

/// 进程级关闭提示标志（仅 main 窗口使用）：`pending` = 后台 MessageBox 进行中；
/// `shown` = 本进程已成功提示过。
static CLOSE_PROMPT_PENDING: AtomicBool = AtomicBool::new(false);
static CLOSE_PROMPT_SHOWN: AtomicBool = AtomicBool::new(false);

/// 托盘状态轮询间隔（固定 1s；初始化立即刷新）。
const TRAY_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// 启动托盘状态轮询线程：固定 1s 读取 [`commands::CommandsState::route_counts`]；
/// **只在状态变化时**更新 tooltip/菜单 enabled/主窗标题。状态锁在 `route_counts`
/// 返回前已释放，UI 调用不持任何路由锁。
fn spawn_tray_status_poll(
    app: AppHandle,
    tray: TrayIcon,
    start: MenuItem<tauri::Wry>,
    stop: MenuItem<tauri::Wry>,
) {
    let spawned = std::thread::Builder::new()
        .name("sr-tray-status".to_string())
        .spawn(move || {
            let mut last: Option<TrayStatus> = None;
            loop {
                // 只拷贝计数；`State` 借用在本语句结束即释放，不跨 UI 调用/睡眠持有。
                let counts = match app.try_state::<commands::CommandsState>() {
                    Some(state) => state.route_counts(),
                    None => return, // 应用退出/状态销毁：轮询结束。
                };
                let status = tray_status(counts.recording, counts.stopping);
                if last.as_ref() != Some(&status) {
                    apply_tray_status(&app, &tray, &start, &stop, &status);
                    last = Some(status);
                }
                std::thread::sleep(TRAY_STATUS_POLL_INTERVAL);
            }
        });
    if let Err(e) = spawned {
        eprintln!("[screen-rec] 托盘状态轮询线程启动失败（tooltip/菜单/标题不更新）：{e}");
    }
}

/// 应用一次状态变化（运行在轮询线程；Tauri 菜单/托盘 setter 自行投递主线程）。
fn apply_tray_status(
    app: &AppHandle,
    tray: &TrayIcon,
    start: &MenuItem<tauri::Wry>,
    stop: &MenuItem<tauri::Wry>,
    status: &TrayStatus,
) {
    if let Err(e) = tray.set_tooltip(Some(status.tooltip.as_str())) {
        eprintln!("[screen-rec] 更新托盘 tooltip 失败：{e}");
    }
    if let Err(e) = start.set_enabled(status.start_enabled) {
        eprintln!("[screen-rec] 更新托盘「开始」状态失败：{e}");
    }
    if let Err(e) = stop.set_enabled(status.stop_enabled) {
        eprintln!("[screen-rec] 更新托盘「停止」状态失败：{e}");
    }
    if let Some(window) = app.get_webview_window("main") {
        if let Err(e) = window.set_title(&status.title) {
            eprintln!("[screen-rec] 更新主窗标题失败：{e}");
        }
    }
}

/// M19b1/REQ-2：后台显示一次「录制仍在继续」原生提示；成功才隐藏主窗。
///
/// 调用方已同步 `prevent_close` 并原子认领 `CLOSE_PROMPT_PENDING`。MessageBox 失败/
/// 线程启动失败/隐藏失败 → 重置 pending（隐藏失败同时保持 shown=false）、保留窗口，
/// 下次关闭可重试；只有 MessageBox 确认且窗口隐藏成功后才置 shown。
fn spawn_close_prompt(app: AppHandle) {
    let spawned = std::thread::Builder::new()
        .name("sr-close-prompt".to_string())
        .spawn(move || {
            let confirmed = tray_notice("录制仍在继续；可从托盘停止或重新打开窗口");
            if !confirmed {
                CLOSE_PROMPT_PENDING.store(false, Ordering::Release);
                eprintln!("[screen-rec] 录制中关闭提示失败（窗口保留，可再次关闭重试）");
                return;
            }
            let hide_app = app.clone();
            if let Err(e) = app.run_on_main_thread(move || {
                let hidden = match hide_app.get_webview_window("main") {
                    Some(window) => match window.hide() {
                        Ok(()) => true,
                        Err(e) => {
                            eprintln!("[screen-rec] 关闭提示确认后隐藏主窗失败：{e}");
                            false
                        }
                    },
                    None => true, // 窗口已不存在：视作已隐藏
                };
                if hidden {
                    CLOSE_PROMPT_SHOWN.store(true, Ordering::Release);
                } else {
                    CLOSE_PROMPT_SHOWN.store(false, Ordering::Release);
                }
                CLOSE_PROMPT_PENDING.store(false, Ordering::Release);
            }) {
                CLOSE_PROMPT_SHOWN.store(false, Ordering::Release);
                CLOSE_PROMPT_PENDING.store(false, Ordering::Release);
                eprintln!("[screen-rec] 关闭提示确认后投递隐藏失败：{e}");
            }
        });
    if let Err(e) = spawned {
        CLOSE_PROMPT_PENDING.store(false, Ordering::Release);
        eprintln!("[screen-rec] 关闭提示线程启动失败（窗口保留，可重试）：{e}");
    }
}

/// 托盘提示（无选中目标/已在录制/录制中关闭主窗）：原生 MessageBox。
///
/// 返回 `true` = 用户确认（MB_OK）；`false` = MessageBox 创建失败（调用方据此保留
/// 现场/重置提示标志）。既有托盘调用可忽略返回值。
fn tray_notice(message: &str) -> bool {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND,
    };
    let text = HSTRING::from(message);
    let caption = HSTRING::from("Screen_rec");
    unsafe {
        MessageBoxW(
            None,
            windows::core::PCWSTR::from_raw(text.as_ptr()),
            windows::core::PCWSTR::from_raw(caption.as_ptr()),
            MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND,
        )
        .0 != 0
    }
}

/// 隐藏自检入口（M11-C P1-5）：`screen-rec.exe --self-check` 打印工具解析结果并执行
/// `<tool> -version`；退出码 0 = 全部可用。供安装后/模拟安装目录验证。
pub fn self_check() -> i32 {
    sr_core::tools::self_check()
}

/// M19c REQ-2：resource_dir 解析失败的 stderr 文本（纯函数；`verbose` 显式传入便于测试）。
///
/// 默认对错误文本中的路径脱敏（委托 [`commands::scrub_absolute_paths`]：Tauri 路径
/// 解析错误的 Display 可能内嵌安装目录）；`verbose`（`SR_DIAG_PATHS=1`）时原样输出
/// （REQ-4 诊断 opt-in）。托盘/窗口 API 错误不含用户数据，保持原样输出。
fn resource_dir_failure_message(err_text: &str, verbose: bool) -> String {
    if verbose {
        format!("[screen-rec] resource_dir 解析失败（回退 exe 同目录）：{err_text}")
    } else {
        format!(
            "[screen-rec] resource_dir 解析失败（回退 exe 同目录）：{}",
            commands::scrub_absolute_paths(err_text)
        )
    }
}

/// 启动最小 Tauri 窗口 + REQ-30 外壳（托盘/单实例/设置/关闭语义）。
pub fn run() {
    tauri::Builder::default()
        // 单实例插件必须最先注册（官方要求）：二次启动进程把参数转发给已运行实例后即退出。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .setup(|app| {
            // M11-C P1-5：注册安装后的 Tauri resource_dir（ffmpeg/、licenses/ 资源根）。
            // 失败不阻塞启动：exe 同目录/开发回退仍在解析链中。
            // M19c REQ-2：解析失败默认不携带完整路径（`SR_DIAG_PATHS=1` 恢复完整路径）。
            match app.path().resource_dir() {
                Ok(dir) => sr_core::tools::set_resource_dir(dir),
                Err(e) => eprintln!(
                    "{}",
                    resource_dir_failure_message(&e.to_string(), sr_core::redact::paths_verbose())
                ),
            }
            // 设置：启动即加载（缺失/损坏回退默认值并写回），供命令读写。
            let config_dir = app.path().app_local_data_dir()?;
            let initial = settings::load_or_default(&config_dir);
            app.manage(SettingsState(Mutex::new(initial)));
            // §3.3 命令层运行时状态（ChannelHub 快照 + 活动路由表 + 恢复扫描缓存）。
            app.manage(commands::CommandsState::new());
            // M18/REQ-21：区域覆盖层上下文/代次（record 确认一次性交付）。
            app.manage(region::RegionOverlayState::default());

            // 托盘：占位图标（icons/icon.ico）+ 菜单（显示/开始/停止/退出）。
            // P2：开始/停止复用命令层语义（最近一次成功配置 = 托盘「开始」的目标；
            // 尚无配置时以原生提示明确要求先在主窗口选择目标）。
            let show = MenuItem::with_id(app, "show", "显示", true, None::<&str>)?;
            let start = MenuItem::with_id(app, "start", "开始", true, None::<&str>)?;
            // M19b1/REQ-1：空闲默认「停止」禁用；轮询立即按真实状态刷新。
            let stop = MenuItem::with_id(app, "stop", "停止", false, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &start, &stop, &quit])?;
            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/icon.ico"))?;
            let tray = TrayIconBuilder::with_id("main-tray")
                .icon(icon)
                .tooltip("Screen_rec")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => show_main_window(app),
                    "start" => {
                        // 启动含同步握手（编码器/管道/捕获），放到后台线程避免阻塞托盘事件循环。
                        let app = app.clone();
                        std::thread::spawn(move || {
                            if let Err(e) = commands::start_from_tray(&app) {
                                tray_notice(&e);
                            }
                        });
                    }
                    "stop" => {
                        let _ = commands::stop_all_impl(app);
                    }
                    // 退出=进程退出：录制/收尾由后台线程持有，sidecar 由 Job 对象连带
                    // 终止（D8/REQ-22），残留会话下次启动经 `recover_sessions` 恢复。
                    // 「录制中退出」的交互分级待托盘 UX 轮次。
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            // M19b1/REQ-1：保存 tray/menu 句柄并启动 1s 状态轮询（初始化立即刷新）。
            // TrayIcon 由轮询线程长期持有（引用计数；避免 setup 返回时最后一个实例被丢弃）。
            spawn_tray_status_poll(app.handle().clone(), tray, start.clone(), stop.clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() != "main" {
                    return;
                }
                // M19b1/REQ-2：关闭主窗口不退出进程（REQ-30）。
                // 空闲/已提示过 → 静默隐藏；录制中首次 → 同步 prevent_close + 原子认领，
                // 后台弹一次提示（不得在回调内同步弹框/等线程/持状态锁）。
                api.prevent_close();
                let app = window.app_handle();
                let counts = app
                    .try_state::<commands::CommandsState>()
                    .map(|state| state.route_counts())
                    .unwrap_or_default();
                let active = counts.recording + counts.stopping > 0;
                let shown = CLOSE_PROMPT_SHOWN.load(Ordering::Acquire);
                let pending = CLOSE_PROMPT_PENDING.load(Ordering::Acquire);
                match close_main_action(active, shown, pending) {
                    CloseMainAction::HideSilent => {
                        let _ = window.hide();
                    }
                    CloseMainAction::Ignore => {}
                    CloseMainAction::PromptOnce => {
                        // 双回调竞态：只有成功认领者弹框；其余只 prevent_close。
                        if CLOSE_PROMPT_PENDING
                            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                            .is_err()
                        {
                            return;
                        }
                        spawn_close_prompt(app.clone());
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            set_settings,
            // §3.3 枚举与能力探测
            commands::probe_capabilities,
            commands::list_targets,
            commands::list_audio_sources,
            commands::list_audio_devices,
            // M16/REQ-9：渲染端点枚举（显示器↔端点匹配）
            commands::list_render_endpoints,
            // M15/REQ-2：端点原生混音率（UI 只读展示；失败返回 None）
            commands::get_endpoint_mix_rate,
            // §3.3 录制/停止（全成功或回滚；停止返回「已请求停止」）
            commands::start_routes,
            commands::stop_route,
            commands::stop_all,
            commands::start_audio_only,
            // §3.3 截图 / 合并 / 恢复
            commands::take_screenshot,
            commands::merge_recordings,
            commands::recover_sessions,
            commands::finalize_session,
            commands::discard_session,
            // P1-1 区域框选（虚拟桌面物理几何 + overlay 生命周期）
            region::get_overlay_geometry,
            region::open_region_overlay,
            region::close_region_overlay,
            // M18/REQ-21：区域录制上下文 / 确认（region_target_added）/ 取消
            region::get_region_context,
            region::confirm_region_record,
            region::cancel_region_record,
            // P1-A 区域截图（后端主导：隐藏覆盖层 → 等合成 → 截图 → 关闭）
            region::take_region_screenshot,
            // REQ-31 Channel 订阅（返回当前状态快照）
            commands::subscribe,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ─────────────────── M19b1/REQ-1：托盘/主窗状态纯函数 ───────────────────

    #[test]
    fn tray_status_idle() {
        let s = tray_status(0, 0);
        assert_eq!(s.tooltip, "Screen_rec");
        assert!(s.start_enabled, "空闲时托盘「开始」应可用");
        assert!(!s.stop_enabled, "空闲时托盘「停止」应禁用");
        assert_eq!(s.title, "Screen_rec");
    }

    #[test]
    fn tray_status_recording() {
        let s = tray_status(2, 0);
        assert_eq!(s.tooltip, "Screen_rec — 录制中 (2 路)");
        assert!(!s.start_enabled, "录制中不得再次开始");
        assert!(s.stop_enabled, "录制中应可停止");
        assert_eq!(s.title, "● 录制中 (2) — Screen_rec");
    }

    #[test]
    fn tray_status_stopping_only() {
        let s = tray_status(0, 3);
        assert_eq!(s.tooltip, "Screen_rec — 正在收尾 (3 路)");
        assert!(!s.start_enabled, "收尾中不得开始");
        assert!(!s.stop_enabled, "收尾中停止已无意义");
        assert_eq!(s.title, "收尾中 (3) — Screen_rec");
    }

    #[test]
    fn tray_status_mixed() {
        let s = tray_status(1, 2);
        assert_eq!(s.tooltip, "Screen_rec — 录制中 (1 路) · 收尾中 (2 路)");
        assert!(!s.start_enabled, "混合状态不得开始");
        assert!(s.stop_enabled, "仍有录制路时应可停止");
        assert_eq!(s.title, "● 录制中 (1) · 收尾中 (2) — Screen_rec");
    }

    // ─────────────────── M19b1/REQ-2：关闭主窗一次提示纯函数 ───────────────────

    #[test]
    fn close_main_action_idle() {
        assert_eq!(
            close_main_action(false, false, false),
            CloseMainAction::HideSilent,
            "空闲关闭应静默隐藏"
        );
        assert_eq!(
            close_main_action(false, true, false),
            CloseMainAction::HideSilent,
            "空闲关闭不因历史提示标志改变"
        );
    }

    #[test]
    fn close_main_action_active_once() {
        assert_eq!(
            close_main_action(true, false, false),
            CloseMainAction::PromptOnce,
            "录制中首次关闭应提示一次"
        );
    }

    #[test]
    fn close_main_action_pending() {
        assert_eq!(
            close_main_action(true, false, true),
            CloseMainAction::Ignore,
            "提示进行中重复关闭只 prevent_close"
        );
    }

    #[test]
    fn close_main_action_after_prompt() {
        assert_eq!(
            close_main_action(true, true, false),
            CloseMainAction::HideSilent,
            "已提示过后关闭应静默隐藏"
        );
        assert_eq!(
            close_main_action(true, true, true),
            CloseMainAction::HideSilent,
            "shown 优先后续关闭不重复提示"
        );
    }

    // ─────────────────── M19c/REQ-2：resource_dir 解析失败 stderr 脱敏 ───────────────────

    /// M19c REQ-2：解析失败默认 stderr 不出现完整路径/用户名；`SR_DIAG_PATHS=1`
    /// （verbose=true）恢复完整错误文本（REQ-4）。
    #[test]
    fn unit_lib_resource_dir_failure_message_redacts_path_by_default() {
        let err = r"failed to resolve resource dir C:\Users\elvis\AppData\Local\app\resources: 系统找不到指定的路径。";

        let default = resource_dir_failure_message(err, false);
        assert!(
            default.contains(r"<redacted>\resources"),
            "默认应保留脱敏后 basename（得到 {default}）"
        );
        assert!(
            !default.contains("elvis") && !default.contains(r"C:\Users"),
            "默认不得泄漏父目录/用户名（得到 {default}）"
        );
        assert!(
            default.contains("系统找不到指定的路径"),
            "错误原因文本保留（得到 {default}）"
        );
        assert!(
            default.starts_with("[screen-rec] resource_dir 解析失败"),
            "既有日志前缀与回退提示保留（得到 {default}）"
        );

        let verbose = resource_dir_failure_message(err, true);
        assert!(
            verbose.contains(r"C:\Users\elvis"),
            "REQ-4：SR_DIAG_PATHS=1 恢复完整路径（得到 {verbose}）"
        );

        // 无路径的错误文本（托盘/窗口类）默认原样保留。
        let plain = "tauri runtime unavailable";
        assert_eq!(
            resource_dir_failure_message(plain, false),
            format!("[screen-rec] resource_dir 解析失败（回退 exe 同目录）：{plain}"),
            "无路径错误文本不得被改写"
        );
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "sr_m15_settings_entry_{tag}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// REQ-1：`set_settings` 与磁盘加载复用同一归一化；录制启动快照不随保存变更。
    #[test]
    fn unit_settings_sample_rate_set_settings_normalizes_and_snapshot_is_stable() {
        let dir = temp_dir("entry");
        let state = SettingsState(Mutex::new(settings::Settings::default()));
        let snapshot = state.snapshot();
        assert_eq!(snapshot.audio_sample_rate, 48_000, "默认 48000");

        let mut next = settings::Settings::default();
        next.audio_sample_rate = 12_345; // 非法档位：set_settings 入口应归一化
        next.fps = 60;
        persist_settings(&dir, &state, next).expect("set_settings 入口应成功");
        let loaded = settings::load_or_default(&dir);
        assert_eq!(
            loaded.audio_sample_rate, 48_000,
            "REQ-1：set_settings 入口应与磁盘加载同归一化（落盘值）"
        );
        assert_eq!(loaded.fps, 60, "归一化不得重置其余字段");
        assert_eq!(
            state.snapshot().audio_sample_rate,
            48_000,
            "内存状态应同样归一化"
        );
        assert_eq!(
            snapshot.audio_sample_rate, 48_000,
            "REQ-1：启动快照不随保存变更"
        );

        let mut next = settings::Settings::default();
        next.audio_sample_rate = 96_000;
        persist_settings(&dir, &state, next).expect("第二次保存");
        assert_eq!(
            snapshot.audio_sample_rate, 48_000,
            "REQ-1：再次保存也不得改变已取得的启动快照"
        );
        assert_eq!(
            state.snapshot().audio_sample_rate,
            96_000,
            "新快照反映最新保存值"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
