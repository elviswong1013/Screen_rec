//! 渲染端点枚举与屏幕↔端点匹配（M16/REQ-9/10，specs/002-recording-modes.md §3.2）。
//!
//! - [`list_render_endpoints`]：WASAPI Render DeviceCollection + `as_immdevice()` 属性存储
//!   读 `PKEY_Device_ContainerId`；专用 MTA 线程（wasapi 类型 !Send）；任何失败返回空 +
//!   日志（REQ-9：枚举失败不阻断 UI）。
//! - [`match_endpoint`]：纯函数，ContainerId 优先、名称仅作 hint/多端点取优；全零 GUID
//!   视为缺失；歧义不取枚举首项（REQ-10）。

use wasapi::{DeviceEnumerator, Direction};
use windows::core::GUID;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_ContainerId;
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToGUID};
use windows::Win32::System::Com::STGM_READ;

use sr_core::error::Error;

/// 渲染端点 DTO（REQ-9）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderEndpoint {
    pub device_id: String,
    pub friendly_name: String,
    /// 设备混音率；未知允许 `None`（不得用于推导重采样比率）。
    pub mix_rate: Option<u32>,
    pub is_active: bool,
    /// PnP ContainerId（规范化小写、无花括号；缺失/全零 → None）。
    pub container_id: Option<String>,
}

/// 列出活动渲染端点。失败返回空列表并记录日志（REQ-9）。
pub fn list_render_endpoints() -> Result<Vec<RenderEndpoint>, Error> {
    let handle = std::thread::Builder::new()
        .name("sr-enum-render-endpoints".into())
        .spawn(|| -> Result<Vec<RenderEndpoint>, String> {
            let _ = wasapi::initialize_mta();
            let enumerator =
                DeviceEnumerator::new().map_err(|e| format!("DeviceEnumerator::new: {e:?}"))?;
            let collection = enumerator
                .get_device_collection(&Direction::Render)
                .map_err(|e| format!("枚举渲染端点失败: {e:?}"))?;
            let mut out = Vec::new();
            for dev in &collection {
                let dev = match dev {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!("[sr-enum] 读取渲染端点条目失败（跳过）: {e:?}");
                        continue;
                    }
                };
                let Ok(device_id) = dev.get_id() else {
                    continue;
                };
                let friendly_name = dev.get_friendlyname().unwrap_or_else(|_| device_id.clone());
                let is_active = matches!(dev.get_state(), Ok(wasapi::DeviceState::Active));
                let mix_rate = dev
                    .get_iaudioclient()
                    .ok()
                    .and_then(|c| c.get_mixformat().ok())
                    .map(|f| f.get_samplespersec());
                let container_id = container_id_of(&dev);
                out.push(RenderEndpoint {
                    device_id,
                    friendly_name,
                    mix_rate,
                    is_active,
                    container_id,
                });
            }
            Ok(out)
        })
        .map_err(|e| Error::Internal(format!("spawn 渲染端点枚举线程失败: {e}")))?;
    match handle.join() {
        Ok(Ok(list)) => Ok(list),
        Ok(Err(detail)) => {
            eprintln!("[sr-enum] 渲染端点枚举失败（返回空列表）: {detail}");
            Ok(Vec::new())
        }
        Err(_) => {
            eprintln!("[sr-enum] 渲染端点枚举线程 panic（返回空列表）");
            Ok(Vec::new())
        }
    }
}

fn container_id_of(device: &wasapi::Device) -> Option<String> {
    unsafe {
        let store = device.as_immdevice().OpenPropertyStore(STGM_READ).ok()?;
        let mut prop = store.GetValue(&PKEY_Device_ContainerId).ok()?;
        let guid = PropVariantToGUID(&prop).ok();
        let _ = PropVariantClear(&mut prop);
        guid.and_then(|g| guid_string(&g))
    }
}

/// GUID → 规范化字符串（小写、无花括号）；全零 → None。
pub fn guid_string(guid: &GUID) -> Option<String> {
    if guid.to_u128() == 0 {
        return None;
    }
    let tail = u64::from_be_bytes([
        0,
        0,
        guid.data4[2],
        guid.data4[3],
        guid.data4[4],
        guid.data4[5],
        guid.data4[6],
        guid.data4[7],
    ]);
    Some(format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:012x}",
        guid.data1, guid.data2, guid.data3, guid.data4[0], guid.data4[1], tail
    ))
}

// ─────────────────────────── 屏幕 ↔ 端点匹配（REQ-10） ───────────────────────────

/// 显示器匹配输入（元数据可能缺失；缺名回退不参与名称证据）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorMeta {
    pub friendly_name: Option<String>,
    pub container_id: Option<String>,
}

/// 匹配依据（当前仅 ContainerId；名称仅作 hint/多端点取优）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchBasis {
    ContainerId,
}

/// 匹配结果（REQ-10 ①–⑤）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointMatch {
    /// 唯一/取优命中。
    Matched {
        device_id: String,
        basis: MatchBasis,
    },
    /// 无 ContainerId 时的名称匹配（标注「名称匹配，仅供参考」）。
    MatchedByHint { device_id: String },
    /// 同分/同 ContainerId 无法取优：交给用户确认，**不得取枚举首项**。
    Ambiguous { candidates: Vec<RenderEndpoint> },
    /// 无关联：预选系统混合。
    Unmatched,
}

const ZERO_GUID: &str = "00000000-0000-0000-0000-000000000000";

/// GUID 规范化：去花括号/空白、小写；全零/空 → None（REQ-10）。
pub fn normalize_guid(value: &str) -> Option<String> {
    let v = value
        .trim()
        .trim_matches(|c| c == '{' || c == '}')
        .trim()
        .to_ascii_lowercase();
    if v.is_empty() || v == ZERO_GUID {
        None
    } else {
        Some(v)
    }
}

fn is_fallback_name(name: &str) -> bool {
    let t = name.trim().to_ascii_lowercase();
    if t.is_empty() {
        return true;
    }
    let Some(rest) = t.strip_prefix("monitor ") else {
        return false;
    };
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

fn normalize_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn bigrams(s: &str) -> Vec<&str> {
    s.as_bytes()
        .windows(2)
        .map(|w| std::str::from_utf8(w).unwrap_or(""))
        .filter(|g| g.len() == 2)
        .collect()
}

fn dice(a: &str, b: &str) -> f64 {
    if a == b {
        return if a.is_empty() { 0.0 } else { 1.0 };
    }
    if a.len() < 2 || b.len() < 2 {
        return 0.0;
    }
    let ga = bigrams(a);
    let gb = bigrams(b);
    let mut hits = 0usize;
    let mut used = vec![false; gb.len()];
    for x in &ga {
        if let Some(pos) = gb
            .iter()
            .enumerate()
            .position(|(i, y)| !used[i] && *y == *x)
        {
            used[pos] = true;
            hits += 1;
        }
    }
    (2.0 * hits as f64) / (ga.len() + gb.len()) as f64
}

fn name_score(monitor_name: &str, endpoint_name: &str) -> f64 {
    if is_fallback_name(monitor_name) {
        return 0.0;
    }
    let m = normalize_name(monitor_name);
    let e = normalize_name(endpoint_name);
    if m.is_empty() || e.is_empty() {
        return 0.0;
    }
    if e.contains(&m) || m.contains(&e) {
        let ratio = m.len().min(e.len()) as f64 / m.len().max(e.len()) as f64;
        return 0.9 + ratio.min(1.0) * 0.1;
    }
    dice(&m, &e)
}

fn pick_best(monitor_name: &str, candidates: &[&RenderEndpoint]) -> Option<EndpointMatch> {
    if is_fallback_name(monitor_name) {
        return None;
    }
    let mut scored: Vec<(f64, &RenderEndpoint)> = candidates
        .iter()
        .map(|ep| (name_score(monitor_name, &ep.friendly_name), *ep))
        .filter(|(score, _)| *score > 0.0)
        .collect();
    if scored.is_empty() {
        return None;
    }
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.device_id.cmp(&b.1.device_id))
    });
    let top = scored[0].0;
    let mut tied: Vec<&RenderEndpoint> = scored
        .iter()
        .filter(|(score, _)| (score - top).abs() < 1e-9)
        .map(|(_, ep)| *ep)
        .collect();
    if tied.len() > 1 {
        tied.sort_by(|a, b| a.device_id.cmp(&b.device_id));
        return Some(EndpointMatch::Ambiguous {
            candidates: tied.into_iter().cloned().collect(),
        });
    }
    Some(EndpointMatch::Matched {
        device_id: scored[0].1.device_id.clone(),
        basis: MatchBasis::ContainerId,
    })
}

/// 屏幕 ↔ 端点匹配纯函数（REQ-10）。
pub fn match_endpoint(monitor: &MonitorMeta, endpoints: &[RenderEndpoint]) -> EndpointMatch {
    let active: Vec<&RenderEndpoint> = endpoints.iter().filter(|e| e.is_active).collect();
    let monitor_name = monitor.friendly_name.as_deref().unwrap_or("");
    let monitor_cid = monitor.container_id.as_deref().and_then(normalize_guid);

    if let Some(cid) = monitor_cid {
        let hits: Vec<&RenderEndpoint> = active
            .iter()
            .copied()
            .filter(|e| {
                e.container_id
                    .as_deref()
                    .and_then(normalize_guid)
                    .as_deref()
                    == Some(cid.as_str())
            })
            .collect();
        if hits.is_empty() {
            // 有 ContainerId 但零命中：不得退化为名称猜测（REQ-10 ④）。
            return EndpointMatch::Unmatched;
        }
        if hits.len() == 1 {
            return EndpointMatch::Matched {
                device_id: hits[0].device_id.clone(),
                basis: MatchBasis::ContainerId,
            };
        }
        return match pick_best(monitor_name, &hits) {
            Some(EndpointMatch::Matched { device_id, .. }) => EndpointMatch::Matched {
                device_id,
                basis: MatchBasis::ContainerId,
            },
            Some(EndpointMatch::Ambiguous { candidates }) => {
                EndpointMatch::Ambiguous { candidates }
            }
            _ => {
                let mut candidates: Vec<RenderEndpoint> = hits.into_iter().cloned().collect();
                candidates.sort_by(|a, b| a.device_id.cmp(&b.device_id));
                EndpointMatch::Ambiguous { candidates }
            }
        };
    }

    // 无 ContainerId（含全零 GUID）：仅 active 端点参与名称 hint。
    match pick_best(monitor_name, &active) {
        Some(EndpointMatch::Matched { device_id, .. }) => {
            EndpointMatch::MatchedByHint { device_id }
        }
        Some(EndpointMatch::Ambiguous { candidates }) => EndpointMatch::Ambiguous { candidates },
        _ => EndpointMatch::Unmatched,
    }
}
