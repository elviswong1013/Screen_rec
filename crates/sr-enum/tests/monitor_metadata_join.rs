//! L1（M16/REQ-10）：GDI 显示器 ↔ DisplayConfig 元数据按**设备名**关联。
//!
//! 运行：`cargo test -p sr-enum --test monitor_metadata_join`。
//!
//! 规格明确禁止两个枚举列表按位置配对（specs/002-recording-modes.md §3.2 REQ-10）：
//! GDI 的 `szDevice`（如 `\\.\DISPLAY2`）必须与 DisplayConfig 的
//! `viewGdiDeviceName` 按字符串关联；关联失败只降级字段、不得丢显示器/改几何。

use sr_enum::monitors::{join_display_metadata, DisplayTargetMeta};

fn target(
    gdi: &str,
    friendly: Option<&str>,
    path: Option<&str>,
    container: Option<&str>,
) -> DisplayTargetMeta {
    DisplayTargetMeta {
        view_gdi_device_name: gdi.to_string(),
        friendly_name: friendly.map(str::to_string),
        monitor_device_path: path.map(str::to_string),
        container_id: container.map(str::to_string),
    }
}

#[test]
fn unit_monitor_metadata_join_by_device_name_not_position() {
    let gdi_names = vec![r"\\.\DISPLAY1".to_string(), r"\\.\DISPLAY2".to_string()];
    // DisplayConfig 列表顺序与 GDI 相反：位置配对会串屏。
    let targets = vec![
        target(
            r"\\.\DISPLAY2",
            Some("U27G4WS"),
            Some(r"\\?\DISPLAY#DELA1B2#5&abc&0&UID4357#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}"),
            Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"),
        ),
        target(
            r"\\.\DISPLAY1",
            Some("LS24A600N"),
            Some(r"\\?\DISPLAY#SAM0F9E#5&def&0&UID4352#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}"),
            Some("11111111-2222-3333-4444-555555555555"),
        ),
    ];
    let metas = join_display_metadata(&gdi_names, &targets);
    assert_eq!(metas.len(), 2, "必须保持 GDI 稠密顺序/长度");
    assert_eq!(metas[0].friendly_name.as_deref(), Some("LS24A600N"));
    assert_eq!(
        metas[0].container_id.as_deref(),
        Some("11111111-2222-3333-4444-555555555555"),
        "DISPLAY1 必须按名称关联到自己的元数据（不得按位置取 DISPLAY2 的）"
    );
    assert_eq!(metas[1].friendly_name.as_deref(), Some("U27G4WS"));
    assert_eq!(
        metas[1].container_id.as_deref(),
        Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")
    );
}

#[test]
fn unit_monitor_metadata_join_missing_target_degrades_fields_only() {
    let gdi_names = vec![r"\\.\DISPLAY1".to_string(), r"\\.\DISPLAY2".to_string()];
    let targets = vec![target(
        r"\\.\DISPLAY2",
        Some("U27G4WS"),
        Some(r"\\?\DISPLAY#DELA1B2#5&abc&0&UID4357#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}"),
        None,
    )];
    let metas = join_display_metadata(&gdi_names, &targets);
    assert_eq!(metas.len(), 2, "元数据缺失不得丢失显示器");
    assert!(
        metas[0].friendly_name.is_none(),
        "未关联显示器字段降级为 None"
    );
    assert!(metas[0].device_path.is_none());
    assert!(metas[0].container_id.is_none());
    assert_eq!(metas[1].friendly_name.as_deref(), Some("U27G4WS"));
}

#[test]
fn unit_monitor_metadata_join_empty_names_never_match() {
    let gdi_names = vec![r"\\.\DISPLAY1".to_string()];
    let targets = vec![target("", Some("Bogus"), None, None)];
    let metas = join_display_metadata(&gdi_names, &targets);
    assert!(
        metas[0].friendly_name.is_none(),
        "空设备名不得按位置兜底关联"
    );
}

#[test]
fn unit_monitor_metadata_join_duplicate_names_first_target_wins() {
    let gdi_names = vec![r"\\.\DISPLAY1".to_string()];
    let targets = vec![
        target(r"\\.\DISPLAY1", Some("First"), None, Some("a")),
        target(r"\\.\DISPLAY1", Some("Second"), None, Some("b")),
    ];
    let metas = join_display_metadata(&gdi_names, &targets);
    assert_eq!(metas[0].friendly_name.as_deref(), Some("First"));
    assert_eq!(metas[0].container_id.as_deref(), Some("a"));
}
