// ============================================================================
// ui/route-state.js — 录制卡片终态护栏（M16/REQ-12）
//
// 规则：**迟到旧快照不得覆盖已收到的终态**。终态（done/failed）一旦到达：
// - 非终态事件/快照（recording/stopping/…）一律忽略；
// - 终态与终态之间只补全缺失字段（path / stop_notice），不互相降级。
//
// 同时被浏览器（经典 <script>）与 Node runner（globalThis）使用。
// ============================================================================
(function (root) {
  'use strict';

  const TERMINAL = new Set(['done', 'failed']);

  function mapState(rawState) {
    if (rawState === 'finalized') return 'done';
    if (rawState === 'failed') return 'failed';
    return rawState;
  }

  function isTerminal(value) {
    if (typeof value === 'string') return TERMINAL.has(value);
    return !!value && TERMINAL.has(value.state);
  }

  /**
   * 应用状态（事件/快照共用）。返回是否发生可见变更。
   * @param {object} r 卡片状态
   * @param {string} rawState 后端状态（recording/stopping/finalized/failed）
   * @param {string|null} finalPath
   * @param {object|null} stopNotice
   */
  function applyRouteState(r, rawState, finalPath, stopNotice) {
    if (!r) return false;
    const next = mapState(rawState);
    if (r.state === next && !finalPath && !stopNotice) return false;
    // REQ-12：已终态 → 旧的非终态快照/事件不得回退。
    if (isTerminal(r.state) && !isTerminal(next)) return false;
    if (isTerminal(r.state) && isTerminal(next)) {
      // 终态之间只补全，不降级/不覆盖已有路径。
      let changed = false;
      if (!r.path && finalPath) {
        r.path = finalPath;
        r.paths = r.paths || [finalPath];
        changed = true;
      }
      if (!r.stopNotice && stopNotice) {
        r.stopNotice = stopNotice;
        changed = true;
      }
      return changed;
    }
    r.state = next;
    if (finalPath) {
      r.path = finalPath;
      r.paths = r.paths || [finalPath];
    }
    if (stopNotice) r.stopNotice = stopNotice;
    if (isTerminal(r.state) && !r.finishedAt) r.finishedAt = Date.now();
    return true;
  }

  /** route_error：告警+终态；已完成后再报错只保留错误展示，不回退状态。 */
  function applyRouteError(r, code, message, severity) {
    if (!r) return false;
    r.error = { code, message };
    if (!isTerminal(r.state)) {
      r.state = 'failed';
      r.finishedAt = r.finishedAt || Date.now();
    }
    return true;
  }

  /** route_finalized：成品就绪（即使此前失败，也以成品路径为准 → done）。 */
  function applyRouteFinalized(r, path, paths, stopNotice) {
    if (!r) return false;
    r.state = 'done';
    if (path) {
      r.path = path;
      r.paths = paths || [path];
    }
    if (stopNotice) r.stopNotice = stopNotice;
    r.finishedAt = r.finishedAt || Date.now();
    return true;
  }

  /** 早于卡片注册到达的事件/启动回包（未知 route_id 缓存）。 */
  function applyPending(r, pending) {
    if (!r || !pending) return false;
    if (pending.error) {
      r.error = pending.error;
    }
    if (pending.stop_notice) r.stopNotice = pending.stop_notice;
    const state = pending.state || (pending.path ? 'finalized' : 'recording');
    const path = pending.final_path || pending.path || null;
    // 成品路径是最终结果：即使先收到 route_error（failed），finalized 也以 done 为准。
    if (state === 'finalized' || path) {
      return applyRouteFinalized(r, path, pending.paths, pending.stop_notice || null);
    }
    return applyRouteState(r, state, null, pending.stop_notice || null);
  }

  /** 快照路由条目合并（终态护栏）。 */
  function mergeSnapshotRoute(r, rs) {
    if (!r || !rs) return false;
    return applyRouteState(r, rs.state, rs.final_path, rs.stop_notice);
  }

  root.SRRouteState = {
    mapState,
    isTerminal,
    applyRouteState,
    applyRouteError,
    applyRouteFinalized,
    applyPending,
    mergeSnapshotRoute,
  };
})(typeof window !== 'undefined' ? window : globalThis);
