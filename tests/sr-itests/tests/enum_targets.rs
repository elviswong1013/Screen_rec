//! L2 集成测试：目标枚举（REQ-1..3，specs/001-screen-recorder.md §2.1 / §4）。
//!
//! 运行方式（L2 门：`#[ignore]`，默认 `cargo test` 不执行本文件用例）：
//! ```text
//! cargo test -p sr-itests --test enum_targets -- --ignored --nocapture
//! cargo test -p sr-itests --test enum_targets it_enum_monitors_physical -- --ignored --nocapture   # 单用例
//! ```
//! 前置条件：交互式用户会话（Session 0 不合格）；缺失 → BLOCKED（fail），不静默 skip（§4）。
//! 本机环境（红测试编写时）：Windows 11 25H2，build 26200.9457，SESSIONNAME=Console。
//!
//! 接口契约（测试即契约，详见 docs/milestones/m1-red-report.md）：
//! - `sr_enum::windows::list_windows() -> Result<Vec<WindowEntry>, E>`
//!   `WindowEntry { hwnd: u64, pid: u32, process_created_at: u64(UNIX ms),
//!                  title: String, app_name: String, process_name: String }`
//! - `sr_enum::monitors::list_monitors() -> Result<Vec<MonitorEntry>, E>`
//!   `MonitorEntry { index: u32, is_primary: bool, x: i32, y: i32, width_px: u32, height_px: u32 }`
//! 其中 E 仅要求实现 std::error::Error 语义（测试经 `.expect` 触发），具体错误枚举
//! 契约见规格 §3.4，由后续任务钉死。字段命名/单位为「测试假定」。

mod common;

use common::*;
use std::time::{Duration, Instant};

use sr_enum::monitors::list_monitors;
use sr_enum::windows::list_windows;

/// REQ-1：窗口枚举列出可见顶层窗口（标题/应用名/进程名/PID/身份字段）+ cloaked 过滤语义。
///
/// 策略：测试自建可见顶层窗口（PID=测试进程，标题=唯一标记，字段可全量断言），
/// 并 spawn notepad 打开唯一命名文件（跨应用验证应用名/进程名解析；本机实测
/// Win11 记事本合并标签页，故以标题含文件名定位其窗口属主）。
#[test]
#[ignore = "L2：需交互式会话/真实窗口（specs §4）。显式运行：cargo test -p sr-itests --test enum_targets -- --ignored --nocapture"]
fn it_enum_windows_lists_visible_app_name() {
    require_interactive_session();
    let _os = record_os_build();

    let marker = format!("SR-M1-RED-targets-{}", std::process::id());
    let probe = ProbeWindow::new(&marker);

    // spawn notepad 打开唯一命名文件（标题将以该文件名开头/包含该文件名）
    let tmp = std::env::temp_dir().join(format!("sr_m1_red_{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("创建临时目录");
    let note_file = tmp.join(format!("sr_m1_red_note_{}.txt", std::process::id()));
    std::fs::write(&note_file, b"sr m1 red\n").expect("写入临时文件");
    let mut notepad_child = std::process::Command::new("notepad.exe")
        .arg(&note_file)
        .spawn()
        .unwrap_or_else(|e| blocked(&format!("无法 spawn notepad.exe：{e}")));
    let note_stem = note_file.file_stem().unwrap().to_string_lossy().to_string();

    // 等 notepad 出现可识别窗口（标题含唯一文件名；标签页合并场景下现有窗口标题会切换到该文件名）
    let note_win = wait_until(Duration::from_secs(15), Duration::from_millis(250), || {
        enum_sys_windows()
            .into_iter()
            .find(|w| w.visible && w.title.contains(&note_stem) && w.pid != std::process::id())
    })
    .unwrap_or_else(|| {
        blocked(&format!(
            "15s 内未出现标题含 {note_stem} 的 notepad 窗口（无法取得可识别外部应用窗口）"
        ))
    });
    let note_created_true = process_created_unix_ms(note_win.pid).unwrap_or_else(|| {
        blocked(&format!(
            "无法读取 notepad 属主进程 {} 的创建时间",
            note_win.pid
        ))
    });
    let self_created_true = self_created_unix_ms();
    let self_exe_name = std::env::current_exe()
        .expect("current_exe")
        .file_name()
        .expect("exe file_name")
        .to_string_lossy()
        .to_string();

    // ── 被测 API（REQ-1）──
    let entries = list_windows().expect("sr_enum::windows::list_windows 应成功（REQ-1）");

    // 断言 1：自建可见窗口被列出，且全部字段正确
    let probe_entry = entries
        .iter()
        .find(|e| e.hwnd == probe.hwnd_u64)
        .unwrap_or_else(|| {
            panic!(
                "REQ-1：枚举结果应包含测试自建的可见顶层窗口 hwnd={:#x}（标题 {marker:?}）",
                probe.hwnd_u64
            )
        });
    assert_eq!(
        probe_entry.title, marker,
        "REQ-1：窗口标题应为 SetWindowText 地面真值"
    );
    assert_eq!(
        probe_entry.pid,
        std::process::id(),
        "REQ-1：PID 应为窗口属主进程（= 测试进程）"
    );
    assert_eq!(
        probe_entry.process_name.to_lowercase(),
        self_exe_name.to_lowercase(),
        "REQ-1：进程名应为属主进程映像名（= 测试二进制 {self_exe_name}）"
    );
    assert!(
        !probe_entry.app_name.trim().is_empty(),
        "REQ-1：应用名不得为空"
    );
    let created_delta = probe_entry.process_created_at.abs_diff(self_created_true);
    assert!(
        created_delta <= 2_000,
        "REQ-1：身份字段 process_created_at 应为属主进程创建时间（UNIX ms），偏差 {created_delta}ms > 2000ms"
    );

    // 断言 2：外部真实应用（notepad）被列出，PID/标题/进程名/应用名与地面真值一致
    let note_entry = entries
        .iter()
        .find(|e| e.hwnd == note_win.hwnd)
        .unwrap_or_else(|| {
            panic!(
                "REQ-1：枚举结果应包含 notepad 窗口 hwnd={:#x}（标题 {:?}）",
                note_win.hwnd, note_win.title
            )
        });
    assert_eq!(
        note_entry.pid, note_win.pid,
        "REQ-1：PID 应为窗口属主（启动器 PID 无关）"
    );
    assert_eq!(
        note_entry.title, note_win.title,
        "REQ-1：标题应与 GetWindowTextW 地面真值一致"
    );
    assert!(
        note_entry.process_name.to_lowercase().contains("notepad"),
        "REQ-1：进程名应解析为 notepad（得到 {:?}）",
        note_entry.process_name
    );
    assert!(
        !note_entry.app_name.trim().is_empty(),
        "REQ-1：应用名不得为空（真实应用）"
    );
    assert!(
        note_entry.process_created_at.abs_diff(note_created_true) <= 2_000,
        "REQ-1：notepad 身份创建时间偏差 > 2s"
    );

    // 断言 3：cloaked 过滤语义——结果中任何仍存活的窗口都不得处于 cloaked 状态
    let mut cloaked_leaks = Vec::new();
    for e in &entries {
        if !is_window_alive_u64(e.hwnd) {
            continue; // 枚举后窗口已销毁（竞态），不计
        }
        if let Some(c) = cloak_of_u64(e.hwnd) {
            if c != 0 {
                cloaked_leaks.push(format!(
                    "hwnd={:#x} cloak={} title={:?}",
                    e.hwnd, c, e.title
                ));
            }
        }
    }
    assert!(
        cloaked_leaks.is_empty(),
        "REQ-1：cloaked 窗口不得出现在枚举结果中（泄露 {} 个：{:?}）",
        cloaked_leaks.len(),
        cloaked_leaks
    );

    // 断言 4：条目以 HWND 去重（同一 HWND 只出现一次）
    let mut hwnds: Vec<u64> = entries.iter().map(|e| e.hwnd).collect();
    let before = hwnds.len();
    hwnds.sort_unstable();
    hwnds.dedup();
    assert_eq!(before, hwnds.len(), "REQ-1：同一 HWND 不得重复出现");

    // 清理
    drop(probe);
    close_window(note_win.hwnd);
    let _ = notepad_child.wait();
}

/// REQ-2：显示器枚举（数量/物理分辨率/位置/主显示器标识）与系统状态一致。
/// 交叉验证：Win32 EnumDisplayMonitors + EnumDisplaySettingsW(ENUM_CURRENT_SETTINGS)（物理像素，
/// 不受测试进程 DPI 感知影响 → 钉死「物理分辨率」契约）+ GetMonitorInfoW（主显示器标识）。
#[test]
#[ignore = "L2：需交互式会话/真实显示器（specs §4）。显式运行：cargo test -p sr-itests --test enum_targets -- --ignored --nocapture"]
fn it_enum_monitors_physical() {
    require_interactive_session();
    let _os = record_os_build();

    let sys = enum_sys_monitors();
    assert!(
        !sys.is_empty(),
        "交叉验证前提：EnumDisplayMonitors 至少返回 1 台显示器"
    );
    let sys_primary_count = sys.iter().filter(|m| m.primary).count();
    assert_eq!(sys_primary_count, 1, "系统应有且仅有一台主显示器");

    let mons = list_monitors().expect("sr_enum::monitors::list_monitors 应成功（REQ-2）");

    // 数量一致
    assert_eq!(
        mons.len(),
        sys.len(),
        "REQ-2：显示器数量应与 EnumDisplayMonitors 一致"
    );

    // 物理分辨率 + 位置一致（按 (x,y,w,h) 匹配集合）
    let mut sys_set: Vec<(i32, i32, u32, u32)> = sys
        .iter()
        .map(|m| (m.x, m.y, m.width_px, m.height_px))
        .collect();
    sys_set.sort_unstable();
    let mut got_set: Vec<(i32, i32, u32, u32)> = mons
        .iter()
        .map(|m| (m.x, m.y, m.width_px, m.height_px))
        .collect();
    got_set.sort_unstable();
    assert_eq!(
        got_set, sys_set,
        "REQ-2：各显示器物理分辨率与位置（DEVMODE 地面真值）应一致"
    );

    // 主显示器标识一致
    let got_primary: Vec<&_> = mons.iter().filter(|m| m.is_primary).collect();
    assert_eq!(
        got_primary.len(),
        1,
        "REQ-2：应有且仅有一台 is_primary=true"
    );
    let sys_primary = sys.iter().find(|m| m.primary).unwrap();
    let got_p = got_primary[0];
    assert_eq!(
        (got_p.x, got_p.y),
        (sys_primary.x, sys_primary.y),
        "REQ-2：is_primary 显示器的位置应与系统主显示器一致"
    );

    // index 唯一且稠密（RouteConfig::Monitor(idx) 的引用基础——测试假定）
    let mut idx: Vec<u32> = mons.iter().map(|m| m.index).collect();
    idx.sort_unstable();
    let dense: Vec<u32> = (0..mons.len() as u32).collect();
    assert_eq!(
        idx, dense,
        "测试假定：index 应唯一且为 0..n-1（供 Monitor(idx) 引用）"
    );
}

/// REQ-3：刷新——窗口打开→刷新可见；关闭→刷新消失；单次刷新 ≤2s（≤100 窗口规模）。
/// 用自建窗口保证「打开/关闭」的确定性（不受外部应用标签页合并等行为影响）。
#[test]
#[ignore = "L2：需交互式会话/真实窗口（specs §4）。显式运行：cargo test -p sr-itests --test enum_targets -- --ignored --nocapture"]
fn it_enum_refresh_reflects_open_close() {
    require_interactive_session();
    let _os = record_os_build();

    // 基线：窗口尚未创建
    let base = list_windows().expect("REQ-3：基线 list_windows 应成功");
    let marker = format!("SR-M1-RED-refresh-{}", std::process::id());
    let probe_hwnd = {
        // 打开
        let probe = ProbeWindow::new(&marker);
        wait_until(Duration::from_secs(5), Duration::from_millis(50), || {
            if probe.is_alive() {
                Some(())
            } else {
                None
            }
        })
        .expect("自建窗口应立即可用");
        let probe_hwnd = probe.hwnd_u64;

        // 刷新 → 可见
        let t0 = Instant::now();
        let entries = list_windows().expect("REQ-3：打开后 list_windows 应成功");
        let dt_open = t0.elapsed();
        assert!(
            entries.iter().any(|e| e.hwnd == probe_hwnd),
            "REQ-3：窗口打开后一次刷新应反映（hwnd={probe_hwnd:#x}）"
        );
        check_refresh_budget(&entries, dt_open, "打开后刷新");
        probe_hwnd
        // probe 在此 drop → DestroyWindow（关闭）
    };

    // 等窗口真正销毁（地面真值），再刷新
    wait_until(Duration::from_secs(10), Duration::from_millis(100), || {
        if is_window_alive_u64(probe_hwnd) {
            None
        } else {
            Some(())
        }
    })
    .expect("自建窗口销毁应生效（IsWindow=false）");

    // 刷新 → 消失
    let t0 = Instant::now();
    let entries2 = list_windows().expect("REQ-3：关闭后 list_windows 应成功");
    let dt_close = t0.elapsed();
    assert!(
        !entries2.iter().any(|e| e.hwnd == probe_hwnd),
        "REQ-3：窗口关闭后一次刷新应不再列出（hwnd={probe_hwnd:#x}）"
    );
    check_refresh_budget(&entries2, dt_close, "关闭后刷新");
    let _ = base; // 基线仅证明 API 可调用；可见性断言如上
}

/// REQ-3/REQ-1：cloaked（UWP 幽灵）窗口不出现在枚举结果中。
///
/// 前置条件构造：对自建可见窗口设置 DWMWA_CLOAK（与被挂起的 UWP 幽灵窗口同类的
/// cloaked 状态，DWMWA_CLOAKED 报告 DWM_CLOAKED_APP），使测试确定性成立；
/// 同时扫描系统自然存在的 cloaked 窗口一并断言不被列出。
/// 若无法构造 cloaked 前置条件 → BLOCKED（fail），不静默 skip（§4）。
#[test]
#[ignore = "L2：需交互式会话/DWM（specs §4）。显式运行：cargo test -p sr-itests --test enum_targets -- --ignored --nocapture"]
fn it_enum_uwp_cloaked_filter() {
    require_interactive_session();
    let _os = record_os_build();

    let marker = format!("SR-M1-RED-cloak-{}", std::process::id());
    let probe = ProbeWindow::new(&marker);

    // 前提：未 cloaked 时窗口可被枚举（避免「空列表」造成的空洞通过）
    let entries0 = list_windows().expect("REQ-1：cloaked 前置 list_windows 应成功");
    assert!(
        entries0.iter().any(|e| e.hwnd == probe.hwnd_u64),
        "前置条件：可见的自建窗口应先被枚举到，后续 cloaked 过滤断言才有意义"
    );

    // 构造 cloaked 前置条件
    probe.set_cloak(true).unwrap_or_else(|e| {
        blocked(&format!(
            "无法构造 cloaked 前置条件（DwmSetWindowAttribute(DWMWA_CLOAK) 失败：{e}）"
        ))
    });
    match probe.cloaked_state() {
        Some(c) if c != 0 => { /* 前置条件就绪 */ }
        Some(_) => blocked("DWMWA_CLOAK 置位后 DWMWA_CLOAKED 仍报告 0，无法构造 cloaked 前置条件"),
        None => blocked("DWMWA_CLOAKED 查询失败，无法确认 cloaked 前置条件"),
    }

    let entries1 = list_windows().expect("REQ-1：cloaked 后 list_windows 应成功");
    assert!(
        !entries1.is_empty(),
        "REQ-1：过滤 cloaked 后不应返回空列表（桌面总有可见顶层窗口）"
    );
    assert!(
        !entries1.iter().any(|e| e.hwnd == probe.hwnd_u64),
        "REQ-1：cloaked（幽灵）窗口不得出现在枚举结果中"
    );

    // 系统自然存在的 cloaked 窗口（UWP 挂起幽灵等）同样不得泄露
    let natural_cloaked: Vec<u64> = enum_sys_windows()
        .into_iter()
        .filter(|w| w.visible && w.cloaked.map_or(false, |c| c != 0) && w.hwnd != probe.hwnd_u64)
        .map(|w| w.hwnd)
        .collect();
    let leaked: Vec<u64> = natural_cloaked
        .iter()
        .filter(|h| entries1.iter().any(|e| &e.hwnd == *h))
        .copied()
        .collect();
    if natural_cloaked.is_empty() {
        println!("[note] 当前系统无自然 cloaked 窗口（已用 DWMWA_CLOAK 自造前置条件覆盖 REQ-1 过滤断言）");
    }
    assert!(
        leaked.is_empty(),
        "REQ-1：系统自然 cloaked 窗口 {natural_cloaked:?} 中的 {leaked:?} 泄露进枚举结果"
    );

    // 对称性：解除 cloak → 重新可见（排除 impl 以其它方式偶发滤掉该窗口的假阳性）
    probe.set_cloak(false).expect("解除 DWMWA_CLOAK");
    wait_until(Duration::from_secs(5), Duration::from_millis(100), || {
        if probe.cloaked_state() == Some(0) {
            Some(())
        } else {
            None
        }
    })
    .expect("DWMWA_CLOAKED 应回到 0");
    let entries2 = list_windows().expect("REQ-1：解除 cloak 后 list_windows 应成功");
    assert!(
        entries2.iter().any(|e| e.hwnd == probe.hwnd_u64),
        "对称性：解除 cloak 后窗口应重新被枚举到（排除假阳性过滤）"
    );

    drop(probe);
}

// ── 本文件内部小工具 ──

/// REQ-3 的 ≤2s 刷新预算仅以 ≤100 窗口规模为前提；超出规模时记录说明而非失败。
fn check_refresh_budget(
    entries: &[sr_enum::windows::WindowEntry],
    dt: std::time::Duration,
    phase: &str,
) {
    if entries.len() <= 100 {
        assert!(
            dt <= Duration::from_secs(2),
            "REQ-3：{phase}耗时 {dt:?} > 2s（规模 {} ≤ 100 窗口）",
            entries.len()
        );
    } else {
        println!(
            "[note] 当前可见顶层窗口 {} > 100，REQ-3 的 ≤2s 预算以 ≤100 窗口为前提，本次不判定（实际 {dt:?}）",
            entries.len()
        );
    }
}
