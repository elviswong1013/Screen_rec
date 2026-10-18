//! L1（M16/REQ-10）：屏幕 ↔ 渲染端点匹配纯函数。
//!
//! 运行：`cargo test -p sr-enum --test render_endpoint_match`（默认 L1 全量执行）。
//!
//! 契约（specs/002-recording-modes.md §3.2 REQ-10 / §4）：
//! - [`sr_enum::render_endpoints::match_endpoint`] 调用**生产**匹配函数；
//! - ContainerId 规范化（去花括号/小写；全零 GUID = 缺失）；
//! - ① 唯一 active ContainerId 命中 → `Matched`；
//! - ② 同 ContainerId 多 active 端点 → 名称最接近者，否则 `Ambiguous`（**不得取首项**）；
//! - ③ 无 ContainerId → 仅 active 端点参与名称 hint（空串/回退名不作证据）；
//! - ④ 无关联 → `Unmatched`；
//! - ⑥ 全 inactive / 顺序无关 / 同分歧义。

use sr_enum::render_endpoints::{
    match_endpoint, normalize_guid, EndpointMatch, MatchBasis, MonitorMeta, RenderEndpoint,
};

const CONTAINER_A: &str = "11111111-2222-3333-4444-555555555555";
const CONTAINER_B: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
const ZERO_GUID: &str = "00000000-0000-0000-0000-000000000000";

fn endpoint(id: &str, name: &str, active: bool, container: Option<&str>) -> RenderEndpoint {
    RenderEndpoint {
        device_id: id.to_string(),
        friendly_name: name.to_string(),
        mix_rate: Some(48_000),
        is_active: active,
        container_id: container.map(str::to_string),
    }
}

fn matched_id(result: EndpointMatch) -> String {
    match result {
        EndpointMatch::Matched { device_id, .. } => device_id,
        other => panic!("应 Matched，得到 {other:?}"),
    }
}

fn ambiguous_ids(result: EndpointMatch) -> Vec<String> {
    match result {
        EndpointMatch::Ambiguous { candidates } => {
            candidates.into_iter().map(|c| c.device_id).collect()
        }
        other => panic!("应 Ambiguous，得到 {other:?}"),
    }
}

#[test]
fn unit_render_endpoint_match_guid_normalizes_braces_and_case() {
    assert_eq!(
        normalize_guid("{AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE}").as_deref(),
        Some(CONTAINER_B),
        "GUID 规范化应去花括号并小写"
    );
    assert_eq!(
        normalize_guid("  {11111111-2222-3333-4444-555555555555} ").as_deref(),
        Some(CONTAINER_A)
    );
    assert_eq!(
        normalize_guid(ZERO_GUID),
        None,
        "全零 GUID 必须视为缺失（不得当作可匹配值）"
    );
    assert_eq!(
        normalize_guid("{00000000-0000-0000-0000-000000000000}"),
        None,
        "带花括号的全零 GUID 同样视为缺失"
    );
    assert_eq!(normalize_guid(""), None, "空串 = 缺失");
    assert_eq!(normalize_guid("   "), None, "空白 = 缺失");
}

#[test]
fn unit_render_endpoint_match_container_unique_hit() {
    let monitor = MonitorMeta {
        friendly_name: Some("LS24A600N".into()),
        container_id: Some(format!("{{{}}}", CONTAINER_A.to_uppercase())),
    };
    let endpoints = vec![
        endpoint("ep-default", "Speakers", true, Some(CONTAINER_B)),
        endpoint(
            "ep-display",
            "LS24A600N (NVIDIA HD Audio)",
            true,
            Some(CONTAINER_A),
        ),
    ];
    let result = match_endpoint(&monitor, &endpoints);
    assert_eq!(matched_id(result.clone()), "ep-display");
    match result {
        EndpointMatch::Matched { basis, .. } => {
            assert_eq!(basis, MatchBasis::ContainerId, "唯一 ContainerId 命中");
        }
        other => panic!("应 Matched，得到 {other:?}"),
    }
}

#[test]
fn unit_render_endpoint_match_container_multi_prefers_name() {
    let monitor = MonitorMeta {
        friendly_name: Some("U27G4WS".into()),
        container_id: Some(CONTAINER_A.into()),
    };
    let endpoints = vec![
        endpoint("ep-other", "Speakers (USB Audio)", true, Some(CONTAINER_A)),
        endpoint(
            "ep-display",
            "U27G4WS (NVIDIA High Definition Audio)",
            true,
            Some(CONTAINER_A),
        ),
        endpoint("ep-inactive", "U27G4WS", false, Some(CONTAINER_A)),
    ];
    let result = match_endpoint(&monitor, &endpoints);
    assert_eq!(
        matched_id(result),
        "ep-display",
        "同 ContainerId 多 active 端点应取名称最接近者（不得取枚举首项）"
    );
}

#[test]
fn unit_render_endpoint_match_container_multi_tie_is_ambiguous() {
    let monitor = MonitorMeta {
        friendly_name: Some("Display".into()),
        container_id: Some(CONTAINER_A.into()),
    };
    let endpoints = vec![
        endpoint("ep-b", "Display Audio B", true, Some(CONTAINER_A)),
        endpoint("ep-a", "Display Audio A", true, Some(CONTAINER_A)),
    ];
    let result = match_endpoint(&monitor, &endpoints);
    let ids = ambiguous_ids(result);
    assert_eq!(
        ids,
        vec!["ep-a".to_string(), "ep-b".to_string()],
        "同分候选应 Ambiguous 且按 device_id 排序（确定性，不取首项）"
    );
}

#[test]
fn unit_render_endpoint_match_name_hint_only_active() {
    let monitor = MonitorMeta {
        friendly_name: Some("DELL U2720Q".into()),
        container_id: None,
    };
    let endpoints = vec![
        endpoint("ep-inactive", "DELL U2720Q", false, None),
        endpoint(
            "ep-active",
            "DELL U2720Q (NVIDIA High Definition Audio)",
            true,
            None,
        ),
    ];
    let result = match_endpoint(&monitor, &endpoints);
    match result {
        EndpointMatch::MatchedByHint { device_id } => {
            assert_eq!(device_id, "ep-active", "名称 hint 只能命中 active 端点");
        }
        other => panic!("应 MatchedByHint，得到 {other:?}"),
    }
}

#[test]
fn unit_render_endpoint_match_hint_tie_is_ambiguous() {
    let monitor = MonitorMeta {
        friendly_name: Some("DELL U2720Q".into()),
        container_id: None,
    };
    let endpoints = vec![
        endpoint("ep-b", "DELL U2720Q Speaker B", true, None),
        endpoint("ep-a", "DELL U2720Q Speaker A", true, None),
    ];
    let ids = ambiguous_ids(match_endpoint(&monitor, &endpoints));
    assert_eq!(ids, vec!["ep-a".to_string(), "ep-b".to_string()]);
}

#[test]
fn unit_render_endpoint_match_zero_hits_unmatched() {
    let monitor = MonitorMeta {
        friendly_name: Some("LS24A600N".into()),
        container_id: Some(CONTAINER_A.into()),
    };
    let endpoints = vec![
        endpoint("ep-other", "Speakers", true, Some(CONTAINER_B)),
        endpoint("ep-nocontainer", "LS24A600N", true, None),
    ];
    assert_eq!(
        match_endpoint(&monitor, &endpoints),
        EndpointMatch::Unmatched,
        "有 ContainerId 但无命中 → Unmatched（不得退化为按名称猜）"
    );
}

#[test]
fn unit_render_endpoint_match_zero_guid_treated_missing() {
    let monitor = MonitorMeta {
        friendly_name: Some("LS24A600N".into()),
        container_id: Some(ZERO_GUID.into()),
    };
    let endpoints = vec![
        endpoint("ep-zero", "Spare", true, Some(ZERO_GUID)),
        endpoint("ep-name", "LS24A600N Monitor", true, None),
    ];
    let result = match_endpoint(&monitor, &endpoints);
    match result {
        EndpointMatch::MatchedByHint { device_id } => {
            assert_eq!(
                device_id, "ep-name",
                "全零 GUID 视为缺失 → 转名称 hint（零 GUID 本身不得命中）"
            );
        }
        other => panic!("应 MatchedByHint（全零 GUID 视为缺失），得到 {other:?}"),
    }
}

#[test]
fn unit_render_endpoint_match_missing_name_never_hint() {
    for name in [
        None,
        Some(String::new()),
        Some("   ".into()),
        Some("Monitor 2".into()),
    ] {
        let monitor = MonitorMeta {
            friendly_name: name,
            container_id: None,
        };
        let endpoints = vec![endpoint("ep", "Monitor 2", true, None)];
        assert_eq!(
            match_endpoint(&monitor, &endpoints),
            EndpointMatch::Unmatched,
            "空串/回退名不得作为名称证据（friendly_name={monitor:?}）"
        );
    }
}

#[test]
fn unit_render_endpoint_match_all_inactive_unmatched() {
    let monitor = MonitorMeta {
        friendly_name: Some("U27G4WS".into()),
        container_id: Some(CONTAINER_A.into()),
    };
    let endpoints = vec![
        endpoint("ep-1", "U27G4WS", false, Some(CONTAINER_A)),
        endpoint("ep-2", "U27G4WS (HDMI)", false, None),
    ];
    assert_eq!(
        match_endpoint(&monitor, &endpoints),
        EndpointMatch::Unmatched,
        "全 inactive → Unmatched（不得匹配到非活动端点）"
    );
}

#[test]
fn unit_render_endpoint_match_order_independent() {
    let monitor = MonitorMeta {
        friendly_name: Some("U27G4WS".into()),
        container_id: Some(CONTAINER_A.into()),
    };
    let a = endpoint("ep-b", "U27G4WS Audio", true, Some(CONTAINER_A));
    let b = endpoint(
        "ep-a",
        "U27G4WS (NVIDIA High Definition Audio)",
        true,
        Some(CONTAINER_A),
    );
    let first = match_endpoint(&monitor, &[a.clone(), b.clone()]);
    let second = match_endpoint(&monitor, &[b, a]);
    assert_eq!(first, second, "匹配结果不得依赖端点枚举顺序");
}
