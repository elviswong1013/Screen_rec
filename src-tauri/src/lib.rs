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

use std::sync::Mutex;

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
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

/// 托盘提示（无选中目标/已在录制）：原生 MessageBox（不依赖前端页面状态）。
fn tray_notice(message: &str) {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND,
    };
    let text = HSTRING::from(message);
    let caption = HSTRING::from("Screen_rec");
    unsafe {
        let _ = MessageBoxW(
            None,
            windows::core::PCWSTR::from_raw(text.as_ptr()),
            windows::core::PCWSTR::from_raw(caption.as_ptr()),
            MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND,
        );
    }
}

/// 隐藏自检入口（M11-C P1-5）：`screen-rec.exe --self-check` 打印工具解析结果并执行
/// `<tool> -version`；退出码 0 = 全部可用。供安装后/模拟安装目录验证。
pub fn self_check() -> i32 {
    sr_core::tools::self_check()
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
            match app.path().resource_dir() {
                Ok(dir) => sr_core::tools::set_resource_dir(dir),
                Err(e) => eprintln!("[screen-rec] resource_dir 解析失败（回退 exe 同目录）：{e}"),
            }
            // 设置：启动即加载（缺失/损坏回退默认值并写回），供命令读写。
            let config_dir = app.path().app_local_data_dir()?;
            let initial = settings::load_or_default(&config_dir);
            app.manage(SettingsState(Mutex::new(initial)));
            // §3.3 命令层运行时状态（ChannelHub 快照 + 活动路由表 + 恢复扫描缓存）。
            app.manage(commands::CommandsState::new());

            // 托盘：占位图标（icons/icon.ico）+ 菜单（显示/开始/停止/退出）。
            // P2：开始/停止复用命令层语义（最近一次成功配置 = 托盘「开始」的目标；
            // 尚无配置时以原生提示明确要求先在主窗口选择目标）。
            let show = MenuItem::with_id(app, "show", "显示", true, None::<&str>)?;
            let start = MenuItem::with_id(app, "start", "开始", true, None::<&str>)?;
            let stop = MenuItem::with_id(app, "stop", "停止", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &start, &stop, &quit])?;
            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/icon.ico"))?;
            TrayIconBuilder::with_id("main-tray")
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
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // 关闭主窗口不退出进程：隐藏到托盘（REQ-30）。
                // 录制/收尾在命令层后台线程与 sidecar 中独立推进，隐藏窗口不中断录制；
                // 「录制中隐藏」的 UI 提示由前端按快照呈现。
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
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
