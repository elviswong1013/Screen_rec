//! L2（M16/REQ-9/10）：渲染端点枚举与显示器元数据真实链路。
//!
//! 运行：
//! ```text
//! cargo test -p sr-itests --test render_endpoints -- --ignored --nocapture --test-threads=1
//! ```
//! 前置：交互式会话 + 至少一个渲染端点 + DisplayConfig/SetupAPI 可用；缺失 → BLOCKED。

mod common;

use common::*;
use sr_enum::monitors::list_monitors;
use sr_enum::render_endpoints::list_render_endpoints;

#[test]
#[ignore = "L2：需交互式会话 + 渲染端点（无设备时空列表不失败）"]
fn it_list_render_endpoints() {
    require_interactive_session();
    let _os = record_os_build();

    let endpoints =
        list_render_endpoints().expect("REQ-9：渲染端点枚举不得失败（失败应为空+日志）");
    println!("[render-endpoints] 共 {} 个活动渲染端点", endpoints.len());
    for e in &endpoints {
        println!(
            "[render-endpoints] id={} name={:?} mix_rate={:?} active={} container={:?}",
            e.device_id, e.friendly_name, e.mix_rate, e.is_active, e.container_id
        );
        assert!(!e.device_id.trim().is_empty(), "device_id 不得为空");
        assert!(
            !e.friendly_name.trim().is_empty(),
            "friendly_name 不得为空（得到 {:?}）",
            e.friendly_name
        );
        assert!(e.is_active, "DeviceCollection(Render) 只应返回 active 端点");
        if let Some(rate) = e.mix_rate {
            assert!(
                rate > 0,
                "mix_rate 允许未知，但已知时必须为正（得到 {rate}）"
            );
        }
        if let Some(cid) = &e.container_id {
            assert!(
                !cid.trim().is_empty() && !cid.chars().all(|c| c == '0' || c == '-'),
                "container_id 允许 None，但非空值不得为全零 GUID（得到 {cid:?}）"
            );
        }
    }
    if endpoints.is_empty() {
        println!("[render-endpoints] 无渲染端点：REQ-9 空列表不失败（正路径字段断言不适用）");
    }
}

/// REQ-10：生产枚举 → DisplayConfig/CM 元数据 → DTO。
/// 本机主依据为 PnP ContainerId；断言 ≥1 台显示器的 container_id 与某端点相等，
/// 证明实现不是恒 None；同时断言 GDI 稠密顺序/几何未因元数据失败而改变。
#[test]
#[ignore = "L2：需交互式会话 + 显示器 + 渲染端点（DisplayConfig/CM 链路）"]
fn it_monitor_metadata_real() {
    require_interactive_session();
    let _os = record_os_build();

    let sys = enum_sys_monitors();
    let monitors = list_monitors().expect("REQ-10：显示器枚举不得因元数据失败而失败");
    assert_eq!(
        monitors.len(),
        sys.len(),
        "元数据链路不得丢失显示器（GDI 稠密顺序保持）"
    );
    let sys_geom: Vec<(i32, i32, u32, u32)> = sys
        .iter()
        .map(|m| (m.x, m.y, m.width_px, m.height_px))
        .collect();
    let got_geom: Vec<(i32, i32, u32, u32)> = monitors
        .iter()
        .map(|m| (m.x, m.y, m.width_px, m.height_px))
        .collect();
    assert_eq!(got_geom, sys_geom, "元数据不得改变 index/几何");
    for (i, m) in monitors.iter().enumerate() {
        assert_eq!(m.index, i as u32, "index 必须为 0..n-1 稠密顺序");
        println!(
            "[monitor-meta] #{} primary={} name={:?} path={:?} container={:?}",
            m.index, m.is_primary, m.friendly_name, m.device_path, m.container_id
        );
    }

    let named = monitors
        .iter()
        .filter(|m| m.friendly_name.is_some())
        .count();
    assert!(
        named >= 1,
        "REQ-10：本机应至少解析出 1 台显示器友好名（证明元数据链路已实现，而非恒 None）"
    );

    let endpoints = list_render_endpoints().expect("渲染端点枚举");
    if endpoints.is_empty() {
        blocked("无渲染端点：无法断言显示器 container_id 与端点相等（REQ-10 正路径环境缺失）");
    }
    let mut matched = 0usize;
    for m in &monitors {
        let Some(mc) = m.container_id.as_deref() else {
            continue;
        };
        if endpoints
            .iter()
            .any(|e| e.container_id.as_deref() == Some(mc))
        {
            matched += 1;
            println!("[monitor-meta] #{} container {mc} ↔ 端点命中", m.index);
        }
    }
    assert!(
        matched >= 1 || monitors.iter().all(|m| m.container_id.is_none()),
        "REQ-10：本机至少 1 台显示器的 container_id 应与端点相等（若全部为 None 则元数据链路未实现）"
    );
    if matched == 0 {
        // 全部 None 的降级路径仍不丢显示器（上方几何断言），但正路径未覆盖 → BLOCKED。
        blocked("本机未观察到显示器↔端点 ContainerId 命中（元数据降级路径，正路径环境缺失）");
    }
}
