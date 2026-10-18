// ============================================================================
// ui/endpoint-match.js — 屏幕 ↔ 渲染端点匹配与每显示器音源选择状态
//
// 规格：specs/002-recording-modes.md §3.2 REQ-10（M16）。
// - ContainerId 优先（GUID 规范化：去花括号/小写；全零 GUID 视为缺失）；
// - 同 ContainerId 多 active 端点 → 名称最接近者；同分 → ambiguous（不取首项）；
// - 无 ContainerId → 仅 active 端点参与名称 hint（空串/"Monitor N" 回退名不作证据）；
// - 手选状态按稳定 device_path 保存；缺 path 用 idx:{index} 临时键，且**不跨刷新复用**。
//
// 本文件同时被浏览器（经典 <script>）与 Node runner（globalThis）使用，无外部依赖。
// ============================================================================
(function (root) {
  'use strict';

  const ZERO_GUID = '00000000-0000-0000-0000-000000000000';
  const FALLBACK_NAME = /^monitor\s+\d+$/i;

  function normalizeGuid(value) {
    if (typeof value !== 'string') return null;
    let v = value.trim().replace(/[{}]/g, '').toLowerCase();
    if (!v) return null;
    if (v === ZERO_GUID) return null;
    return v;
  }

  function isFallbackName(name) {
    const v = typeof name === 'string' ? name.trim() : '';
    return !v || FALLBACK_NAME.test(v);
  }

  function normalizeName(name) {
    return String(name || '').toLowerCase().replace(/[^a-z0-9]+/g, '');
  }

  function dice(a, b) {
    if (a === b) return a ? 1 : 0;
    if (a.length < 2 || b.length < 2) return a === b ? 1 : 0;
    const bigrams = new Map();
    for (let i = 0; i < a.length - 1; i++) {
      const g = a.slice(i, i + 2);
      bigrams.set(g, (bigrams.get(g) || 0) + 1);
    }
    let hits = 0;
    for (let i = 0; i < b.length - 1; i++) {
      const g = b.slice(i, i + 2);
      const n = bigrams.get(g) || 0;
      if (n > 0) {
        hits++;
        bigrams.set(g, n - 1);
      }
    }
    return (2 * hits) / (a.length - 1 + b.length - 1);
  }

  function nameScore(monitorName, endpointName) {
    const m = normalizeName(monitorName);
    const e = normalizeName(endpointName);
    if (!m || !e || isFallbackName(monitorName)) return 0;
    if (e.includes(m) || m.includes(e)) {
      return 0.9 + Math.min(0.1, Math.min(m.length, e.length) / Math.max(m.length, e.length) / 10);
    }
    return dice(m, e);
  }

  function activeEndpoints(endpoints) {
    return (endpoints || []).filter((e) => e && e.is_active !== false);
  }

  function pickBest(monitorName, candidates) {
    if (!candidates.length || isFallbackName(monitorName)) return null;
    const scored = candidates
      .map((ep) => ({ ep, score: nameScore(monitorName, ep.friendly_name) }))
      .filter((x) => x.score > 0)
      .sort((a, b) => b.score - a.score || String(a.ep.device_id).localeCompare(String(b.ep.device_id)));
    if (!scored.length) return null;
    const top = scored[0];
    const tied = scored.filter((x) => Math.abs(x.score - top.score) < 1e-6);
    if (tied.length > 1) {
      return {
        status: 'ambiguous',
        candidates: tied
          .map((x) => x.ep)
          .sort((a, b) => String(a.device_id).localeCompare(String(b.device_id))),
      };
    }
    return { status: 'matched', endpoint: top.ep, score: top.score };
  }

  function monitorMeta(monitor) {
    return {
      friendly_name: monitor && (monitor.friendly_name || monitor.name) || null,
      container_id: monitor ? monitor.container_id : null,
    };
  }

  function matchEndpoint(monitor, endpoints) {
    const meta = monitorMeta(monitor || {});
    const active = activeEndpoints(endpoints);
    const monitorCid = normalizeGuid(meta.container_id);
    if (monitorCid) {
      const hits = active.filter((ep) => normalizeGuid(ep.container_id) === monitorCid);
      if (hits.length === 1) {
        return { status: 'matched', basis: 'container_id', endpoint: hits[0] };
      }
      if (hits.length > 1) {
        const best = pickBest(meta.friendly_name, hits);
        if (best && best.status === 'matched') {
          return { status: 'matched', basis: 'container_id', endpoint: best.endpoint };
        }
        return {
          status: 'ambiguous',
          basis: 'container_id',
          candidates: (best && best.candidates) || hits
            .slice()
            .sort((a, b) => String(a.device_id).localeCompare(String(b.device_id))),
        };
      }
      // 有 ContainerId 但无命中：不得退化为名称猜测（REQ-10 ④）。
      return { status: 'unmatched' };
    }
    // 无 ContainerId（含全零 GUID）：仅 active 端点参与名称 hint。
    const best = pickBest(meta.friendly_name, active);
    if (!best) return { status: 'unmatched' };
    if (best.status === 'ambiguous') return best;
    return { status: 'matched_by_hint', endpoint: best.endpoint };
  }

  // ── 每显示器手选状态 ──────────────────────────────────────────────────────

  function monitorKey(monitor) {
    const path = monitor && monitor.device_path;
    return path ? `path:${path}` : `idx:${monitor ? monitor.index : 0}`;
  }

  function isTempKey(key) {
    return typeof key === 'string' && key.startsWith('idx:');
  }

  function defaultSelection(monitor, match) {
    const endpoint = match && (match.status === 'matched' || match.status === 'matched_by_hint')
      ? match.endpoint
      : null;
    return {
      key: monitorKey(monitor),
      device_path: (monitor && monitor.device_path) || null,
      temp: !(monitor && monitor.device_path),
      match: match || { status: 'unmatched' },
      endpointDeviceId: endpoint ? endpoint.device_id : null,
      endpointSelected: !!endpoint,
      systemSelected: !endpoint,
      micSelected: false,
      apps: new Set(),
    };
  }

  /**
   * 刷新目标列表后调和选择状态：
   * - 稳定 device_path 键：保留用户手选/勾选（刷新不覆盖）；
   * - idx:{index} 临时键：每次刷新重建默认预选（不跨刷新复用，防双屏合并/串屏）。
   */
  function reconcileMonitorSelections(prev, monitors, endpoints) {
    const next = new Map();
    for (const monitor of monitors || []) {
      const key = monitorKey(monitor);
      const match = matchEndpoint(monitor, endpoints);
      const existing = !isTempKey(key) && prev ? prev.get(key) : null;
      if (existing) {
        next.set(key, {
          ...existing,
          key,
          device_path: monitor.device_path || null,
          temp: false,
          match,
        });
      } else {
        next.set(key, defaultSelection(monitor, match));
      }
    }
    return next;
  }

  /** 组装某显示器的 sources（顺序固定 [endpoint, system, mic, apps]；仅实际勾选）。 */
  function buildMonitorSources(sel, endpoints, micDevice, appLookup) {
    const sources = [];
    const labels = [];
    if (!sel) return { sources, labels };
    if (sel.endpointSelected && sel.endpointDeviceId) {
      const ep = (endpoints || []).find((e) => e.device_id === sel.endpointDeviceId);
      sources.push({ kind: 'render_endpoint', device_id: sel.endpointDeviceId });
      labels.push(ep && ep.friendly_name ? `端点 · ${ep.friendly_name}` : '显示器输出端点');
    }
    if (sel.systemSelected) {
      sources.push({ kind: 'system' });
      labels.push('系统音频');
    }
    if (sel.micSelected && micDevice) {
      sources.push({ kind: 'mic', device_id: micDevice });
      labels.push('麦克风');
    }
    for (const pid of sel.apps || []) {
      const app = appLookup ? appLookup(pid) : null;
      if (app) {
        sources.push({ kind: 'app', pid: app.pid, created_at_unix_ms: app.created_at_unix_ms ?? 0 });
        labels.push(`应用 · ${app.app_name}`);
      }
    }
    return { sources, labels };
  }

  root.SREndpointMatch = {
    normalizeGuid,
    isFallbackName,
    nameScore,
    matchEndpoint,
    monitorKey,
    isTempKey,
    defaultSelection,
    reconcileMonitorSelections,
    buildMonitorSources,
  };
})(typeof window !== 'undefined' ? window : globalThis);
