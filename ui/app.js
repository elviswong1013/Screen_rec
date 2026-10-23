// ============================================================================
// ui/app.js — Screen_rec 前端主逻辑（中文单页桌面应用）
// 数据层全部经 api.js（Tauri 真实命令 / 浏览器 mock 自动切换）。
// 经典脚本加载（file:// 兼容，见 api.js 头注），依赖 api.js 先于本文件加载。
// 设计说明：docs/design/ui-spec.md
// ============================================================================
(() => {
'use strict';
const { api, isMock, subscribeEvents } = window.SR;
const {
  matchEndpoint, monitorKey, reconcileMonitorSelections, buildMonitorSources,
} = window.SREndpointMatch;
const {
  mapState, isTerminal,
  applyRouteState, applyRouteError, applyRouteFinalized, applyPending, mergeSnapshotRoute,
} = window.SRRouteState;

// ---------------------------------------------------------------------------
// 工具
// ---------------------------------------------------------------------------
const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];

function esc(s) {
  return String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}

function fmtTime(ms) {
  const s = Math.floor(ms / 1000);
  const p = (n) => String(n).padStart(2, '0');
  return `${p(Math.floor(s / 3600))}:${p(Math.floor((s % 3600) / 60))}:${p(s % 60)}`;
}

function fmtWhen(ts) {
  const d = new Date(ts);
  const p = (n) => String(n).padStart(2, '0');
  return `${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

function fileStamp() {
  const d = new Date();
  const p = (n) => String(n).padStart(2, '0');
  return `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}_${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`;
}

const ICONS = {
  monitor: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8m-4-4v4"/></svg>',
  window: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 9h18"/></svg>',
  mic: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="2" width="6" height="12" rx="3"/><path d="M5 10a7 7 0 0 0 14 0M12 19v3"/></svg>',
  inbox: '<svg width="34" height="34" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M22 12h-6l-2 3h-4l-2-3H2"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/></svg>',
};

function toast(kind, msg, path) {
  const root = $('#toast-root');
  const el = document.createElement('div');
  el.className = `toast ${kind}`;
  el.innerHTML = `<div class="msg">${esc(msg)}${path ? `<div class="path">${esc(path)}</div>` : ''}</div>`;
  root.appendChild(el);
  setTimeout(() => {
    el.classList.add('out');
    setTimeout(() => el.remove(), 300);
  }, path ? 6500 : 4000);
}

// ---------------------------------------------------------------------------
// 全局状态
// ---------------------------------------------------------------------------
const state = {
  view: 'record',
  settings: null,
  about: { info: null, loading: false, selected: null },
  endpointRates: {
    seq: 0,
    render: null,
    renderStatus: 'idle',
    renderError: null,
    mic: null,
    micStatus: 'idle',
    micError: null,
    micDevice: null,
  },
  caps: null,
  monitors: [],
  renderEndpoints: [],
  monitorSel: new Map(),       // stable device_path -> 每显示器音源选择（M16/REQ-10/13）
  windows: [],
  audioSources: [],
  audioDevices: [],
  targetsLoading: false,
  targetTab: 'windows',
  search: '',
  selected: new Set(),       // 'monitor:0' | 'window:66050'
  regionTarget: null,        // M18：{display_device, rect, display{device_name,device_path}}
  regionRequestId: null,     // M18b：当前 record overlay 的 request_id（stale 事件过滤）
  regionAudio: {             // M18/REQ-23：区域独立音源勾选（不重用显示器分支）
    endpointSelected: false, endpointDeviceId: '', systemSelected: true,
    micSelected: false, apps: new Set(),
  },
  recordMode: 'screen',      // 'screen' | 'audio'
  cfg: {
    canvas: '1920x1080', fps: 30, cursor: true,
    audioMode: 'isolated',
    srcSystem: true, srcMic: false, srcApps: new Set(),
    outputDir: '',
  },
  audioOnly: { srcSystem: true, srcMic: false, srcApps: new Set(), outputDir: '' },
  routes: new Map(),         // routeId -> 卡片状态
  merge: { extra: [], picked: new Set(), running: false, progress: 0 },
  shotHistory: [],
  recovery: [],              // 未收尾会话
  recentFiles: [],
  starting: false,
  // M19b2/REQ-3 开始确认状态机（会话内存）：Idle -> Summary(手动摘要) / Confirmed(第一次开始) -> Starting。
  // Summary 只展示摘要，不算开始确认；Starting 由 state.starting 表达；配置变化/取消/成败后复位 Idle。
  startConfirm: { phase: 'idle', signature: null },
};

// M19b2/REQ-4：所有 Settings 写入串行化。ack 勾选与设置页保存可能并发；
// 每个任务在队列内重新读取最新 Settings，避免旧快照后写覆盖新值。
let settingsWriteChain = Promise.resolve();
function enqueueSettingsWrite(task) {
  const run = settingsWriteChain.then(task, task);
  settingsWriteChain = run.catch(() => {});
  return run;
}

const targetKey = (t) => (t.hwnd !== undefined ? `window:${t.hwnd}` : (t.kind === 'region' || t.rect ? 'region:0' : `monitor:${t.index}`));

function findTarget(key) {
  const [kind, id] = key.split(':');
  if (kind === 'region') return state.regionTarget ? { ...state.regionTarget, kind: 'region' } : undefined;
  if (kind === 'monitor') return state.monitors.find((m) => m.index === Number(id));
  return state.windows.find((w) => w.hwnd === Number(id));
}

// ---------------------------------------------------------------------------
// 导航
// ---------------------------------------------------------------------------
$$('.nav-item').forEach((btn) => {
  btn.addEventListener('click', () => {
    state.view = btn.dataset.nav;
    $$('.nav-item').forEach((b) => b.classList.toggle('active', b === btn));
    $$('.view').forEach((v) => v.classList.toggle('active', v.id === `view-${state.view}`));
    if (state.view === 'merge') renderMerge();
    if (state.view === 'settings') renderSettings();
    if (state.view === 'screenshot') renderScreenshotView();
    if (state.view === 'about') renderAbout();
    if (state.view === 'record') renderStartBar(); // 回到录制视图时按最新设置刷新摘要/复位确认
  });
});

// ---------------------------------------------------------------------------
// 能力探测 / 降级提示（AC-8）
// ---------------------------------------------------------------------------
function renderCaps() {
  const c = state.caps;
  const row = $('#caps-row');
  if (!c) { row.innerHTML = '<span class="badge">环境探测中…</span>'; return; }
  const encName = c.selected_encoder || c.encoders.find((e) => e.available)?.name;
  row.innerHTML = [
    c.loopback_ok
      ? '<span class="badge ok">按应用音频 可用</span>'
      : '<span class="badge warn">按应用音频 不可用</span>',
    encName
      ? `<span class="badge ok">编码器 ${esc(encName)}</span>`
      : '<span class="badge err">无可用编码器</span>',
    c.mic_ok
      ? '<span class="badge ok">麦克风权限 已允许</span>'
      : '<span class="badge warn">麦克风权限 未允许</span>',
    c.wgc_ok
      ? '<span class="badge ok">画面捕获 可用</span>'
      : '<span class="badge err">画面捕获 不可用</span>',
  ].join('');

  const deg = $('#degraded-mount');
  const msgs = [];
  if (c.os_supported === false) msgs.push('系统版本低于 Windows 10 2004，部分捕获能力不可用。');
  if (!c.loopback_ok) msgs.push(`当前系统不支持按应用音频捕获：应用音源与「隔离」模式已禁用，仍可使用系统音频与麦克风。${c.layers?.loopback?.detail ? `（${c.layers.loopback.detail}）` : ''}`);
  if (!c.mic_ok) msgs.push(`麦克风不可用：麦克风音源已禁用，可在 Windows 设置 → 隐私 → 麦克风 中允许。${c.layers?.mic?.detail ? `（${c.layers.mic.detail}）` : ''}`);
  if (!encName) msgs.push('未探测到可用的 H.264 编码器，无法开始录制。已探测：' + (c.encoders.map((e) => `${e.name}（${e.available ? '可用' : '不可用'}）`).join('、') || '无') + '。');
  deg.innerHTML = msgs.map((m) => `<div class="notice warn">${esc(m)}</div>`).join('');
}

// ---------------------------------------------------------------------------
// 录制视图 — 目标列表 / 音源（左栏）
// ---------------------------------------------------------------------------
async function refreshTargets() {
  if (state.targetsLoading) return;
  state.targetsLoading = true;
  renderLeftPanel();
  try {
    const prevMonitors = state.monitors;
    const prevSelectedMonitors = [...state.selected]
      .filter((k) => k.startsWith('monitor:'))
      .map((k) => prevMonitors.find((m) => m.index === Number(k.split(':')[1])))
      .filter(Boolean);
    const selectedMonitorPaths = new Set(prevSelectedMonitors.map((m) => m.device_path).filter(Boolean));
    const selectedMonitorIndexesNoPath = new Set(
      prevSelectedMonitors.filter((m) => !m.device_path).map((m) => m.index),
    );
    const [tg, sources, eps] = await Promise.all([
      api.listTargets(),
      api.listAudioSources(),
      api.listRenderEndpoints().catch((e) => {
        console.warn('[app] 渲染端点枚举失败：', e);
        return [];
      }),
    ]);
    state.monitors = tg.monitors;
    state.windows = tg.windows;
    state.audioSources = sources;
    state.renderEndpoints = eps || [];
    // M16/REQ-10：刷新后手选不覆盖；稳定 device_path 换序不串屏。
    state.monitorSel = reconcileMonitorSelections(state.monitorSel, state.monitors, state.renderEndpoints);
    // M18：区域目标跟随稳定设备身份刷新（path 优先，GDI 回退），不改变用户选区。
    if (state.regionTarget) {
      const rt = state.regionTarget;
      const dev = rt.display?.device_path || rt.display?.device_name || '';
      const m = state.monitors.find((x) =>
        (dev && (x.device_path === dev || x.gdi_device_name === dev))
        || (x.x <= rt.rect.x && x.y <= rt.rect.y
          && x.x + x.width >= rt.rect.x + rt.rect.w
          && x.y + x.height >= rt.rect.y + rt.rect.h));
      if (m) {
        rt.display = {
          device_name: m.gdi_device_name || rt.display?.device_name || '',
          device_path: m.device_path || null,
        };
      }
    }
    // 目标选择同样按 device_path 迁移（index 交换后仍指向同一物理屏）。
    const nextSelected = new Set([...state.selected].filter((k) => !k.startsWith('monitor:')));
    for (const path of selectedMonitorPaths) {
      const next = state.monitors.find((m) => m.device_path === path);
      if (next) nextSelected.add(`monitor:${next.index}`);
    }
    for (const idx of selectedMonitorIndexesNoPath) {
      if (state.monitors.some((m) => m.index === idx && !m.device_path)) {
        nextSelected.add(`monitor:${idx}`);
      }
    }
    state.selected = nextSelected;
    // 清理已消失目标的选中态
    for (const key of [...state.selected]) if (!findTarget(key)) state.selected.delete(key);
  } catch (e) {
    toast('err', `刷新目标列表失败：${e}`);
  }
  state.targetsLoading = false;
  renderLeftPanel();
  renderConfig(); // 全音轨模式下的应用音源列表随刷新更新
  renderShotLists();
  renderStartBar();
}

function renderLeftPanel() {
  const body = $('#left-panel-body');
  $('#left-panel-title').firstChild.textContent = state.recordMode === 'screen' ? '目标' : '音源';
  $('#btn-refresh').style.display = state.recordMode === 'screen' ? '' : 'none';

  if (state.targetsLoading) {
    body.innerHTML = '<div class="skel"></div>'.repeat(5);
    return;
  }
  if (state.recordMode === 'audio') { renderAudioSourcesPanel(body, state.audioOnly); return; }

  body.innerHTML = `
    <div class="target-tools">
      <input type="text" id="target-search" placeholder="搜索窗口标题 / 应用名 / 进程名" value="${esc(state.search)}" />
    </div>
    <div class="tgt-tabs">
      <button data-tab="windows"></button><button data-tab="monitors"></button>
    </div>
    <div class="tgt-list" id="tgt-list"></div>`;

  // 搜索输入只刷新列表区，避免重建输入框打断中文输入法组合
  $('#target-search').addEventListener('input', (e) => { state.search = e.target.value; updateTargetList(); });
  $$('.tgt-tabs button', body).forEach((b) => b.addEventListener('click', () => {
    state.targetTab = b.dataset.tab;
    updateTargetList();
  }));
  updateTargetList();
}

function updateTargetList() {
  const q = state.search.trim().toLowerCase();
  const winRows = state.windows
    .filter((w) => !q || w.title.toLowerCase().includes(q) || w.app_name.toLowerCase().includes(q) || w.process_name.toLowerCase().includes(q))
    .map((w) => targetRow(w, `PID ${w.pid}`, `${esc(w.app_name)} · ${esc(w.process_name)}`));
  const monRows = state.monitors.map((m) =>
    targetRow(m, m.primary ? '主显示器' : '', `${m.width}×${m.height} · 位置 (${m.x}, ${m.y})`));

  const tabs = $$('.tgt-tabs button');
  tabs[0].className = state.targetTab === 'windows' ? 'active' : '';
  tabs[0].innerHTML = `窗口 <span class="muted">${winRows.length}</span>`;
  tabs[1].className = state.targetTab === 'monitors' ? 'active' : '';
  tabs[1].innerHTML = `显示器 <span class="muted">${monRows.length}</span>`;

  const list = $('#tgt-list');
  if (state.targetTab === 'windows') {
    list.innerHTML = winRows.length
      ? winRows.join('')
      : `<div class="empty">${ICONS.inbox}<br>${q ? '没有匹配的窗口' : '未发现可捕获的窗口'}</div>`;
  } else {
    list.innerHTML = monRows.length
      ? monRows.join('')
      : `<div class="empty">${ICONS.inbox}<br>未发现显示器</div>`;
  }
  bindTargetRows(list);
}

function targetRow(t, tag, meta) {
  const key = targetKey(t);
  const name = t.hwnd !== undefined ? t.title : `${t.name}`;
  const icon = t.hwnd !== undefined ? ICONS.window : ICONS.monitor;
  return `
    <div class="tgt-row ${state.selected.has(key) ? 'selected' : ''}" data-key="${key}">
      <span class="icon">${icon}</span>
      <span class="main">
        <span class="name">${esc(name)}</span>
        <span class="meta">${meta}</span>
      </span>
      ${tag ? `<span class="tag">${esc(tag)}</span>` : ''}
    </div>`;
}

function bindTargetRows(body) {
  $$('.tgt-row', body).forEach((row) => {
    row.addEventListener('click', () => {
      const key = row.dataset.key;
      state.selected.has(key) ? state.selected.delete(key) : state.selected.add(key);
      row.classList.toggle('selected');
      renderStartBar();
      renderConfig(); // 隔离模式下「显示器目标」音源默认值随目标类型变化
    });
  });
}

// ---------------------------------------------------------------------------
// 录制视图 — 配置面板（右栏）
// ---------------------------------------------------------------------------
function renderConfig() {
  const panel = $('#config-panel');
  if (state.recordMode === 'audio') {
    panel.innerHTML = `
      <div class="panel-title">输出</div>
      <div class="field">
        <label>输出目录</label>
        <input type="text" id="cfg-outdir" value="${esc(state.audioOnly.outputDir)}" />
        <div class="hint">每个来源输出一个独立 WAV 文件（48kHz），停止即成品。</div>
      </div>
      <div class="notice info">纯录音支持系统音频、麦克风与正在发声的应用，可多选；各来源互不混音。</div>`;
    $('#cfg-outdir').addEventListener('input', (e) => { state.audioOnly.outputDir = e.target.value; renderStartBar(); });
    return;
  }

  const c = state.caps;
  const cfg = state.cfg;
  const loopbackOk = c?.loopback_ok !== false;
  const micOk = c?.mic_ok === true && !!state.settings?.mic_device;
  const canvasOpts = ['1280x720', '1920x1080', '2560x1440', '3840x2160']
    .filter((s) => {
      const [w, h] = s.split('x').map(Number);
      return !c || (w <= c.canvas_limits.max_w && h <= c.canvas_limits.max_h);
    })
    .map((s) => `<option value="${s}" ${cfg.canvas === s ? 'selected' : ''}>${s.replace('x', ' × ')}</option>`)
    .join('');

  const selectedMonitors = [...state.selected]
    .filter((k) => k.startsWith('monitor:'))
    .map(findTarget)
    .filter(Boolean);
  const hasMonitorTarget = selectedMonitors.length > 0;
  const hasWindowTarget = [...state.selected].some((k) => k.startsWith('window:'));
  const hasRegionTarget = !!state.regionTarget;
  const micName = micOk ? esc(state.audioDevices.find((d) => d.id === state.settings.mic_device)?.name || state.settings.mic_device) : '';
  const micHint = micOk ? micName : state.caps?.mic_ok !== true ? '权限未允许或无设备' : '未在设置中选择设备';

  // 窗口目标沿用既有全局勾选语义；显示器目标逐屏独立选择（REQ-10/13）。
  let windowHtml = '';
  if (hasWindowTarget) {
    if (!loopbackOk) {
      windowHtml = `
        ${checkRow('cfg-src-system', '系统音频（整机混合）', cfg.srcSystem)}
        ${checkRow('cfg-src-mic', '麦克风', cfg.srcMic, !micOk, micHint)}`;
    } else if (cfg.audioMode === 'isolated') {
      windowHtml = `
        <div class="hint" style="margin:-4px 0 8px;">窗口目标自动携带该应用的独立音轨；同一应用的所有窗口共享一条音轨。</div>
        ${checkRow('cfg-src-mic', '麦克风（独立音轨，不混音）', cfg.srcMic, !micOk, micHint)}`;
    } else {
      const apps = state.audioSources.map((a) =>
        checkRow(`cfg-app-${a.pid}`, `${esc(a.app_name)}`, cfg.srcApps.has(a.pid), false, `${esc(a.process_name)} · PID ${a.pid}${a.active ? '' : ' · 未在发声'}`)).join('');
      windowHtml = `
        ${checkRow('cfg-src-system', '系统音频（整机混合）', cfg.srcSystem)}
        ${checkRow('cfg-src-mic', '麦克风（独立音轨）', cfg.srcMic, !micOk, micHint)}
        <div class="field" style="margin-top:8px;">
          <label>追加应用音源（启动时快照；录制中新增音源不自动加入）</label>
          ${apps || '<div class="empty">当前没有正在发声的应用</div>'}
          <button class="btn-sm btn-ghost" id="cfg-apps-all">全选</button>
        </div>`;
    }
  }

  const monitorHtml = selectedMonitors
    .map((m) => monitorAudioHtml(m, loopbackOk, micOk, micHint))
    .join('');
  const regionHtml = hasRegionTarget ? regionAudioHtml(loopbackOk, micOk, micHint) : '';

  let audioHtml;
  if (!hasWindowTarget && !hasMonitorTarget && !hasRegionTarget) {
    audioHtml = '<div class="hint">选择目标后，此处按目标类型给出可用音源。</div>';
  } else {
    audioHtml = `${windowHtml}${monitorHtml}${regionHtml}`;
  }

  panel.innerHTML = `
    <div class="panel-title">录制配置</div>
    <div class="field-row">
      <div class="field"><label>输出画布（固定，窗口变化时等比缩放）</label><select id="cfg-canvas">${canvasOpts}</select></div>
      <div class="field"><label>帧率</label>
        <select id="cfg-fps">
          <option value="30" ${cfg.fps === 30 ? 'selected' : ''}>30 fps</option>
          <option value="60" ${cfg.fps === 60 ? 'selected' : ''}>60 fps</option>
        </select>
      </div>
    </div>
    ${hasRegionTarget ? '<div class="hint" style="margin:-4px 0 8px;">区域录制按选区物理尺寸输出（向下取偶）；画布设置不生效，不做缩放/补边。</div>' : ''}
    ${checkRow('cfg-cursor', '捕获光标', cfg.cursor)}
    <div class="field" style="margin-top:10px;">
      <label>音轨模式</label>
      <label class="check-row ${loopbackOk ? '' : 'disabled'}">
        <input type="radio" name="audio-mode" value="isolated" ${cfg.audioMode === 'isolated' ? 'checked' : ''} ${loopbackOk ? '' : 'disabled'} />
        隔离（默认）：仅目标应用${cfg.srcMic ? ' + 麦克风' : ''}，各为独立音轨
      </label>
      <label class="check-row">
        <input type="radio" name="audio-mode" value="all_tracks" ${cfg.audioMode === 'all_tracks' ? 'checked' : ''} />
        全音轨：目标应用 + 系统 + 麦克风 + 追加应用，全部独立成轨
      </label>
      ${hasMonitorTarget ? '<div class="hint">显示器目标使用全音轨传输契约：每屏逐项勾选均如实计入；切换隔离/全音轨不清除勾选。</div>' : ''}
      ${hasRegionTarget ? '<div class="hint">区域隔离=仅所选应用各自独立成轨（不含系统/麦克风）；全音轨=系统+麦克风+追加应用（+可选端点）。</div>' : ''}
    </div>
    <div class="field"><label>音源</label>${audioHtml}</div>
    <div class="field">
      <label>输出目录</label>
      <input type="text" id="cfg-outdir" value="${esc(cfg.outputDir)}" />
    </div>`;

  // 绑定
  $('#cfg-canvas').addEventListener('change', (e) => { cfg.canvas = e.target.value; renderStartBar(); });
  $('#cfg-fps').addEventListener('change', (e) => { cfg.fps = Number(e.target.value); renderStartBar(); });
  $('#cfg-outdir').addEventListener('input', (e) => { cfg.outputDir = e.target.value; renderStartBar(); });
  $$('input[name="audio-mode"]', panel).forEach((r) => r.addEventListener('change', () => {
    cfg.audioMode = r.value;
    renderConfig();
    renderStartBar(); // 音轨模式改变会改变 canStart/needs_notice，必须刷新开始栏
  }));
  bindCheck('#cfg-cursor', (v) => { cfg.cursor = v; renderStartBar(); });
  bindCheck('#cfg-src-system', (v) => { cfg.srcSystem = v; renderStartBar(); });
  bindCheck('#cfg-src-mic', (v) => {
    cfg.srcMic = v;
    renderConfig(); // 麦克风勾选会重建音源区
    renderStartBar(); // 同时刷新计划签名/告知门禁
  });
  state.audioSources.forEach((a) => bindCheck(`#cfg-app-${a.pid}`, (v) => { v ? cfg.srcApps.add(a.pid) : cfg.srcApps.delete(a.pid); renderStartBar(); }));
  $('#cfg-apps-all')?.addEventListener('click', () => {
    state.audioSources.forEach((a) => cfg.srcApps.add(a.pid));
    renderConfig(); renderStartBar();
  });

  // 每显示器独立勾选状态（M16/REQ-13）：sources = 恰好所选条目。
  for (const m of selectedMonitors) {
    const sel = state.monitorSel.get(monitorKey(m));
    if (!sel) continue;
    bindCheck(`#cfg-mon-${m.index}-endpoint`, (v) => {
      if (!v) {
        sel.endpointSelected = false;
        sel.systemSelected = true; // 取消端点 → 预选系统混合
      } else {
        const fromMatch = sel.match && sel.match.endpoint && sel.match.endpoint.device_id;
        const deviceId = sel.endpointDeviceId || fromMatch || null;
        if (!deviceId) {
          // REQ-10⑤：Ambiguous/Unmatched 无明确候选时不得取枚举首项。
          // 未在下拉中显式选择具体端点前，保持系统混合并明示。
          sel.endpointSelected = false;
          sel.systemSelected = true;
          toast('warn', '未能唯一匹配输出端点：请先在端点下拉中选择具体设备（当前使用系统混合）');
        } else {
          sel.endpointSelected = true;
          sel.endpointDeviceId = deviceId;
        }
      }
      renderConfig();
      renderStartBar();
    });
    bindCheck(`#cfg-mon-${m.index}-system`, (v) => { sel.systemSelected = v; renderStartBar(); });
    bindCheck(`#cfg-mon-${m.index}-mic`, (v) => { sel.micSelected = v; renderStartBar(); });
    const epSelect = $(`#cfg-mon-${m.index}-endpoint-select`, panel);
    if (epSelect) {
      epSelect.addEventListener('change', () => {
        sel.endpointDeviceId = epSelect.value || null;
        sel.endpointSelected = !!epSelect.value;
        if (!epSelect.value) sel.systemSelected = true;
        renderConfig();
        renderStartBar();
      });
    }
    state.audioSources.forEach((a) => bindCheck(`#cfg-mon-${m.index}-app-${a.pid}`, (v) => {
      v ? sel.apps.add(a.pid) : sel.apps.delete(a.pid);
      renderStartBar();
    }));
  }

  // M18/REQ-23：区域独立音源勾选（仅全音轨显示端点/系统/麦克风；隔离仅应用）。
  if (hasRegionTarget) {
    const ra = state.regionAudio;
    bindCheck('#cfg-region-endpoint', (v) => {
      ra.endpointSelected = v;
      if (!v) ra.endpointDeviceId = '';
      renderConfig();
      renderStartBar();
    });
    const epSelect = $('#cfg-region-endpoint-select', panel);
    if (epSelect) {
      epSelect.addEventListener('change', () => {
        ra.endpointDeviceId = epSelect.value || '';
        ra.endpointSelected = !!epSelect.value;
        renderStartBar();
      });
    }
    bindCheck('#cfg-region-system', (v) => { ra.systemSelected = v; renderStartBar(); });
    bindCheck('#cfg-region-mic', (v) => { ra.micSelected = v; renderStartBar(); });
    state.audioSources.forEach((a) => bindCheck(`#cfg-region-app-${a.pid}`, (v) => {
      v ? ra.apps.add(a.pid) : ra.apps.delete(a.pid);
      renderStartBar();
    }));
  }

  function bindCheck(sel, fn) {
    const el = $(sel, panel);
    if (el) el.addEventListener('change', () => { fn(el.checked); });
  }
}

function checkRow(id, label, checked, disabled = false, sub = '') {
  return `
    <label class="check-row ${disabled ? 'disabled' : ''}">
      <input type="checkbox" id="${id}" ${checked ? 'checked' : ''} ${disabled ? 'disabled' : ''} />
      <span>${label}</span>
      ${sub ? `<span class="sub">${sub}</span>` : ''}
    </label>`;
}

// M16/REQ-10：单显示器音源区（端点自动匹配状态 + 逐项勾选，顺序 endpoint/system/mic/apps）。
function monitorAudioHtml(m, loopbackOk, micOk, micHint) {
  const key = monitorKey(m);
  const sel = state.monitorSel.get(key);
  const match = (sel && sel.match) || matchEndpoint(m, state.renderEndpoints);
  const active = state.renderEndpoints.filter((e) => e.is_active !== false);
  const matchedName = match.endpoint && match.endpoint.friendly_name ? match.endpoint.friendly_name : '';
  const status = match.status === 'matched'
    ? `自动匹配：${matchedName}（ContainerId）`
    : match.status === 'matched_by_hint'
      ? `名称匹配：${matchedName}（仅供参考）`
      : match.status === 'ambiguous'
        ? `未能唯一匹配（${(match.candidates || []).length} 个候选），请确认；确认前使用系统混合`
        : '未能自动匹配；已预选系统混合';
  const endpointId = sel && sel.endpointDeviceId ? sel.endpointDeviceId : '';
  const options = [
    `<option value="" ${endpointId ? '' : 'selected'}>不启用端点（使用系统混合）</option>`,
    ...active.map((e) => `<option value="${esc(e.device_id)}" ${endpointId === e.device_id ? 'selected' : ''}>${esc(e.friendly_name)}${e.is_active ? '' : '（非活动）'}</option>`),
  ].join('');
  const apps = state.audioSources.map((a) =>
    checkRow(`cfg-mon-${m.index}-app-${a.pid}`, `${esc(a.app_name)}`, !!sel && sel.apps.has(a.pid), false, `${esc(a.process_name)} · PID ${a.pid}${a.active ? '' : ' · 未在发声'}`)).join('');
  return `
    <div class="field monitor-audio" data-monitor-key="${esc(key)}" data-monitor-index="${m.index}">
      <label>显示器 ${m.index + 1} · ${esc(m.friendly_name || m.name)} 音源</label>
      <div class="hint" data-match-status>${esc(status)}</div>
      ${loopbackOk ? `
        ${checkRow(`cfg-mon-${m.index}-endpoint`, '该屏输出端点（自动匹配，可手选）', !!sel && sel.endpointSelected)}
        <select id="cfg-mon-${m.index}-endpoint-select" style="margin:2px 0 8px;">${options}</select>` : `
        <div class="notice warn">渲染端点不可用：该屏仅可使用系统/麦克风。</div>`}
      ${checkRow(`cfg-mon-${m.index}-system`, '系统音频（整机混合）', !!sel && sel.systemSelected)}
      ${checkRow(`cfg-mon-${m.index}-mic`, '麦克风（独立音轨）', !!sel && sel.micSelected, !micOk, micHint)}
      <div class="field" style="margin-top:8px;">
        <label>追加应用音源（启动时快照）</label>
        ${apps || '<div class="empty">当前没有正在发声的应用</div>'}
      </div>
    </div>`;
}

// M18/REQ-23：区域音源区（独立分支；隔离仅应用，全音轨可选端点/系统/麦克风/应用）。
function regionAudioHtml(loopbackOk, micOk, micHint) {
  const ra = state.regionAudio;
  const isolated = state.cfg.audioMode === 'isolated' && loopbackOk;
  const apps = state.audioSources.map((a) =>
    checkRow(`cfg-region-app-${a.pid}`, `${esc(a.app_name)}`, ra.apps.has(a.pid), !loopbackOk, `${esc(a.process_name)} · PID ${a.pid}${a.active ? '' : ' · 未在发声'}`)).join('');
  if (isolated) {
    return `
      <div class="field monitor-audio" data-region-audio="isolated">
        <label>区域音源 · 隔离（仅所选应用）</label>
        <div class="hint">仅所选应用各自独立成轨；不隐式加入系统混音/麦克风/端点。</div>
        ${apps || '<div class="empty">当前没有正在发声的应用</div>'}
        <div class="hint">至少选择 1 个应用；否则启动会被拒绝。</div>
      </div>`;
  }
  const active = state.renderEndpoints.filter((e) => e.is_active !== false);
  const endpointId = ra.endpointDeviceId || '';
  const options = [
    `<option value="" ${endpointId ? '' : 'selected'}>不启用端点（使用系统混合）</option>`,
    ...active.map((e) => `<option value="${esc(e.device_id)}" ${endpointId === e.device_id ? 'selected' : ''}>${esc(e.friendly_name)}</option>`),
  ].join('');
  return `
    <div class="field monitor-audio" data-region-audio="all_tracks">
      <label>区域音源 · 全音轨</label>
      <div class="hint">系统 + 麦克风 + 追加应用；可选该屏输出端点（与系统同 ID 时后端去重）。</div>
      ${loopbackOk ? `
        ${checkRow('cfg-region-endpoint', '输出端点（可选，手选）', ra.endpointSelected, !loopbackOk)}
        <select id="cfg-region-endpoint-select" style="margin:2px 0 8px;">${options}</select>` : ''}
      ${checkRow('cfg-region-system', '系统音频（整机混合）', ra.systemSelected)}
      ${checkRow('cfg-region-mic', '麦克风（独立音轨）', ra.micSelected, !micOk, micHint)}
      <div class="field" style="margin-top:8px;">
        <label>追加应用音源（启动时快照）</label>
        ${apps || '<div class="empty">当前没有正在发声的应用</div>'}
      </div>
    </div>`;
}

// 纯录音左栏：音源多选
function renderAudioSourcesPanel(body, store) {
  const micOk = state.caps?.mic_ok === true && !!state.settings?.mic_device;
  const micName = micOk ? esc(state.audioDevices.find((d) => d.id === state.settings.mic_device)?.name || state.settings.mic_device) : '';
  const loopbackOk = state.caps?.loopback_ok !== false;
  const apps = state.audioSources.map((a) =>
    checkRow(`ao-app-${a.pid}`, esc(a.app_name), store.srcApps.has(a.pid), !loopbackOk, `${esc(a.process_name)} · PID ${a.pid}`)).join('');
  body.innerHTML = `
    ${checkRow('ao-system', '系统音频（整机混合）', store.srcSystem)}
    ${checkRow('ao-mic', '麦克风', store.srcMic, !micOk, micOk ? micName : '权限未允许或未选设备')}
    <div class="field" style="margin-top:10px;">
      <label>应用（按进程隔离）</label>
      ${apps || '<div class="empty">当前没有正在发声的应用</div>'}
    </div>
    ${loopbackOk ? '' : '<div class="notice warn">按应用音频在当前系统不可用。</div>'}`;
  const bind = (sel, fn) => { const el = $(sel, body); if (el) el.addEventListener('change', () => { fn(el.checked); renderStartBar(); }); };
  bind('#ao-system', (v) => { store.srcSystem = v; });
  bind('#ao-mic', (v) => { store.srcMic = v; });
  state.audioSources.forEach((a) => bind(`#ao-app-${a.pid}`, (v) => { v ? store.srcApps.add(a.pid) : store.srcApps.delete(a.pid); }));
}

// ---------------------------------------------------------------------------
// 录制视图 — 开始栏与启动
// ---------------------------------------------------------------------------
// M19b2/REQ-3：音源含麦克风/系统/端点 → 开始前需告知确认。
const NOTICE_SOURCE_KINDS = new Set(['mic', 'system', 'render_endpoint']);
const NOTICE_KIND_LABEL = { mic: '麦克风', system: '系统声音', render_endpoint: '显示器输出端点' };

function renderStartBar() {
  const bar = $('#start-bar');
  bar.style.flexWrap = 'wrap'; // 展开摘要面板占整行（inline，不改 styles.css）
  const plan = buildStartPlan();
  // REQ-3：目标/音源/输出/格式任何变化 → 摘要/确认态复位到 Idle，下次启动需重新确认。
  if (state.startConfirm.phase !== 'idle' && state.startConfirm.signature !== plan.signature) {
    state.startConfirm = { phase: 'idle', signature: null };
  }
  const planOpen = state.startConfirm.phase !== 'idle';
  const confirmed = state.startConfirm.phase === 'confirmed';
  const ack = state.settings?.recording_notice_ack === true;
  const confirmPending = confirmed && plan.needs_notice && !ack;
  const label = state.starting
    ? '<span class="spin"></span> 启动中…'
    : confirmPending ? '确认并开始' : plan.buttonLabel;
  bar.innerHTML = `
    <span class="summary">${plan.summary}</span>
    <button class="btn-sm btn-ghost" id="btn-start-detail" ${plan.canExpand || planOpen ? '' : 'disabled'}>${planOpen ? '收起摘要' : '摘要'}</button>
    <span class="muted" style="font-size:12px;">Ctrl+Enter</span>
    <button class="${state.recordMode === 'screen' ? 'btn-rec' : 'btn-primary'}" id="btn-start" ${plan.canStart && !state.starting ? '' : 'disabled'}>${label}</button>
    ${planOpen ? `
      <div id="start-plan" style="flex-basis:100%;border-top:1px solid var(--border);padding-top:8px;">
        ${plan.detail.map((d) => `<div class="hint">${esc(d)}</div>`).join('')}
        ${plan.needs_notice ? `
          <div class="notice warn" id="start-notice" style="margin:6px 0 0;">
            本次录制包含${plan.noticeKinds.map((k) => NOTICE_KIND_LABEL[k]).join('、')}：会记录目标应用之外的声音。
            <div style="margin-top:4px;display:flex;gap:14px;align-items:center;">
              <label style="display:inline-flex;gap:6px;align-items:center;cursor:pointer;">
                <input type="checkbox" id="start-notice-ack" ${ack ? 'checked' : ''} /><span>不再显示此提示</span>
              </label>
              ${confirmPending ? '<button class="btn-sm btn-ghost" id="btn-start-cancel">取消</button>' : ''}
            </div>
          </div>` : ''}
      </div>` : ''}`;
  $('#btn-start').addEventListener('click', onStartClick);
  $('#btn-start-detail').addEventListener('click', () => {
    // 手动摘要只展开内容，不算第一次开始确认；收起/再次点击复位 Idle。
    state.startConfirm = planOpen
      ? { phase: 'idle', signature: null }
      : { phase: 'summary', signature: plan.signature };
    renderStartBar();
  });
  $('#btn-start-cancel')?.addEventListener('click', () => {
    state.startConfirm = { phase: 'idle', signature: null };
    renderStartBar();
  });
  $('#start-notice-ack')?.addEventListener('change', (e) => onNoticeAckChange(e.target));
}

// REQ-4：「不再显示此提示」只保存 ack，不启动；先读当前完整 Settings、
// 仅改该字段再整体保存（保留其他字段）；失败如实提示，不伪装成功。
async function onNoticeAckChange(box) {
  const want = box.checked;
  box.disabled = true;
  try {
    await enqueueSettingsWrite(async () => {
      const cur = await api.getSettings();
      const next = { ...cur, recording_notice_ack: want };
      await api.setSettings(next);
      state.settings = next;
    });
    toast('ok', want ? '已保存：开始前告知不再显示' : '已保存：恢复开始前告知');
  } catch (e) {
    toast('err', `保存失败：${e}`);
  }
  renderStartBar();
}

// M19b2/REQ-3：单一启动计划来源——同时产出启动 DTO、摘要与 needs_notice。
// 覆盖窗口/逐屏/区域/纯录音；纯录音显示 WAV、不含画布/帧率；区域显示最终选区尺寸。
// 零音源不在此拒绝（doStart 保留既有拦截与提示）。
function buildStartPlan() {
  const s = state.settings || {};
  const noticeKindsOf = (sources) =>
    [...new Set(sources.filter((x) => NOTICE_SOURCE_KINDS.has(x.kind)).map((x) => x.kind))];

  if (state.recordMode === 'audio') {
    const a = state.audioOnly;
    const sources = [];
    const labels = [];
    if (a.srcSystem) { sources.push({ kind: 'system' }); labels.push('系统音频'); }
    if (a.srcMic && s.mic_device) { sources.push({ kind: 'mic', device_id: s.mic_device }); labels.push('麦克风'); }
    for (const pid of a.srcApps) {
      const src = state.audioSources.find((x) => x.pid === pid);
      if (src) { sources.push({ kind: 'app', pid: src.pid, created_at_unix_ms: src.created_at_unix_ms ?? 0 }); labels.push(`应用 · ${src.app_name}`); }
    }
    const outDir = a.outputDir.trim() || s.output_dir || '';
    const n = audioOnlySourceCount();
    const dto = [{ sources, output_dir: outDir, _ui_track_labels: labels }];
    const noticeKinds = noticeKindsOf(sources);
    return {
      mode: 'audio',
      dto,
      canStart: n > 0,
      canExpand: n > 0,
      needs_notice: noticeKinds.length > 0,
      noticeKinds,
      buttonLabel: '开始录音',
      summary: n ? `已选 <b>${n}</b> 个音源 · 各输出独立 WAV` : '先在左侧选择至少一个音源',
      detail: [
        `音源：${labels.join('、')}`,
        `输出目录：${outDir} · 格式：WAV（每个来源一个独立文件）`,
      ],
      signature: JSON.stringify({ mode: 'audio', dto }),
    };
  }

  const [w, h] = state.cfg.canvas.split('x');
  const [cw, ch] = [Number(w), Number(h)];
  const outDir = state.cfg.outputDir.trim() || s.output_dir || '';
  const fmt = s.output_container === 'mp4' ? 'MP4' : 'MKV';
  const routeCfgs = [...state.selected].map((key) => {
    const t = findTarget(key);
    const target = t.hwnd !== undefined ? { kind: 'window', hwnd: t.hwnd } : { kind: 'monitor', index: t.index };
    const audio = buildAudioPlan(t);
    return {
      target,
      video: { canvas_w: cw, canvas_h: ch, fps: state.cfg.fps, cursor: state.cfg.cursor },
      audio: { mode: audio.mode, sources: audio.sources },
      output_dir: outDir,
      _ui_label: t.hwnd !== undefined ? t.app_name : `显示器${t.index + 1} · ${t.name}`,
      _ui_track_labels: audio.labels,
    };
  });
  // M18：区域目标（独立分支；display_device 只用稳定路径或 GDI szDevice）。
  // regionDisplayDevice：null=无区域目标；''=缺稳定标识（doStart 拦截提示）。
  let regionDisplayDevice = null;
  if (state.regionTarget) {
    const rt = state.regionTarget;
    regionDisplayDevice = rt.display?.device_path || rt.display?.device_name || rt.display_device || '';
    const audio = buildRegionAudioPlan();
    routeCfgs.push({
      target: {
        kind: 'region',
        display_device: regionDisplayDevice,
        rect: { x: rt.rect.x, y: rt.rect.y, w: rt.rect.w, h: rt.rect.h },
      },
      video: { canvas_w: cw, canvas_h: ch, fps: state.cfg.fps, cursor: state.cfg.cursor },
      audio: { mode: audio.mode, sources: audio.sources },
      output_dir: outDir,
      _ui_label: `区域 ${rt.rect.w}×${rt.rect.h}`,
      _ui_track_labels: audio.labels,
    });
  }
  const n = routeCfgs.length;
  const regionBlocked = !!state.regionTarget && regionSourceCount() === 0;
  const canStart = n > 0 && !!state.caps?.encoders.some((e) => e.available) && !regionBlocked;
  let summary = n
    ? `已选 <b>${n}</b> 个目标${state.regionTarget ? `（区域 ${state.regionTarget.rect.w}×${state.regionTarget.rect.h}）` : ''} · ${w}×${h} @${state.cfg.fps}fps · ${state.cfg.audioMode === 'isolated' ? '隔离' : '全音轨'}`
    : '先在左侧选择要录制的窗口或显示器（可多选，每路独立输出）';
  if (regionBlocked) summary += ' · <span class="muted">区域目标尚未选择音源</span>';
  const noticeKinds = noticeKindsOf(routeCfgs.flatMap((c) => c.audio.sources));
  return {
    mode: 'screen',
    dto: routeCfgs,
    regionDisplayDevice,
    canStart,
    canExpand: n > 0,
    needs_notice: noticeKinds.length > 0,
    noticeKinds,
    buttonLabel: n > 1 ? `开始录制（${n} 路）` : '开始录制',
    summary,
    detail: [
      ...routeCfgs.map((c) => `目标：${c._ui_label} · 音源：${(c._ui_track_labels || []).join('、') || '（无）'}`),
      `输出目录：${outDir} · 成品格式：${fmt} · 画布 ${w}×${h} @${state.cfg.fps}fps`,
    ],
    signature: JSON.stringify({ mode: 'screen', dto: routeCfgs, fmt }),
  };
}

// M19b2/REQ-3 开始门禁：needs_notice 且未勾选「不再显示」时，
// 第一次点击只展开摘要+告知；第二次「确认并开始」才调用启动 API。
// 仅 app 音源或 ack=true：一次点击启动。Ctrl+Enter 走同一门禁（点按钮）。
async function onStartClick() {
  if (state.starting) return;
  const plan = buildStartPlan();
  if (!plan.canStart) return; // 按钮已禁用；仅作键盘/程序化兜底
  const ack = state.settings?.recording_notice_ack === true;
  if (plan.needs_notice && !ack) {
    if (state.startConfirm.phase !== 'confirmed' || state.startConfirm.signature !== plan.signature) {
      // 首次开始，或手动摘要后的第一次开始：只展开告知，不启动。
      state.startConfirm = { phase: 'confirmed', signature: plan.signature };
      renderStartBar();
      return;
    }
  }
  await doStart(plan);
}

async function doStart(plan) {
  state.starting = true;
  renderStartBar();
  try {
    if (plan.mode === 'screen') {
      if (state.regionTarget && !plan.regionDisplayDevice) {
        toast('warn', '区域目标缺少稳定显示器标识（设备路径/GDI 名称），请重新框选');
        return;
      }
      // 前端预检：任何一路音源为空都会被后端拒绝（批次全成功或回滚），提前拦截并说明
      const empty = plan.dto.filter((c) => c.audio.sources.length === 0);
      if (empty.length) {
        toast('warn', `${empty.map((c) => c._ui_label).join('、')} 没有任何音源：显示器目标请勾选该屏输出端点，或系统音频/麦克风；区域隔离请选择至少一个应用（不静默录制）`);
        return;
      }
      const { route_ids } = await api.startRoutes(plan.dto);
      route_ids.forEach((id, i) => registerRoute(id, {
        kind: 'video',
        label: plan.dto[i]._ui_label,
        icon: plan.dto[i].target.kind === 'window' ? ICONS.window : ICONS.monitor,
        trackLabels: plan.dto[i]._ui_track_labels,
      }));
      state.selected.clear();
      state.regionTarget = null;
      state.regionRequestId = null;
      toast('ok', `已开始 ${route_ids.length} 路录制`);
    } else {
      const labels = plan.dto[0]._ui_track_labels;
      // 后端契约：一次调用 N 源 → N 个共享 handle 的 route id（任一停止即整组停止）
      const { route_ids } = await api.startAudioOnly(plan.dto);
      route_ids.forEach((id, i) => registerRoute(id, {
        kind: 'audio',
        label: labels[i] ? `录音 · ${labels[i]}` : `录音 ${i + 1}`,
        icon: ICONS.mic,
        trackLabels: [labels[i] || `来源 ${i + 1}`],
      }));
      toast('ok', `已开始录音（${route_ids.length} 个来源）`);
    }
    renderLeftPanel();
  } catch (e) {
    toast('err', `启动失败：${e}`);
  } finally {
    state.starting = false;
    // REQ-3：启动失败/完成均复位到 Idle；每次新启动尝试都需重新确认。
    state.startConfirm = { phase: 'idle', signature: null };
    renderStartBar();
  }
}

function audioOnlySourceCount() {
  const s = state.audioOnly;
  return (s.srcSystem ? 1 : 0) + (s.srcMic ? 1 : 0) + s.srcApps.size;
}

function buildAudioPlan(target) {
  const cfg = state.cfg;
  const loopbackOk = state.caps?.loopback_ok !== false;
  const isWindow = target?.hwnd !== undefined;
  // M16/REQ-13：显示器目标逐屏独立选择；sources = 恰好所有已勾选条目，
  // 顺序 [endpoint, system, mic, apps]；传输契约固定 mode=all_tracks。
  if (!isWindow) {
    const sel = state.monitorSel.get(monitorKey(target));
    if (sel) {
      const built = buildMonitorSources(
        sel,
        state.renderEndpoints,
        state.settings?.mic_device,
        (pid) => state.audioSources.find((x) => x.pid === pid),
      );
      return { mode: 'all_tracks', sources: built.sources, labels: built.labels };
    }
    // 兜底（理论上 reconcile 已建状态）：按匹配结果预选端点或系统混合。
    const match = matchEndpoint(target, state.renderEndpoints);
    const sources = [];
    const labels = [];
    if (match.status === 'matched' || match.status === 'matched_by_hint') {
      sources.push({ kind: 'render_endpoint', device_id: match.endpoint.device_id });
      labels.push(`端点 · ${match.endpoint.friendly_name}`);
    } else {
      sources.push({ kind: 'system' });
      labels.push('系统音频');
    }
    if (cfg.srcMic && state.settings?.mic_device) {
      sources.push({ kind: 'mic', device_id: state.settings.mic_device });
      labels.push('麦克风');
    }
    return { mode: 'all_tracks', sources, labels };
  }
  // 后端校验（E_INVALID_AUDIO_PLAN）：isolated 不得包含 system 音源（隔离=目标应用轨[+麦克风]）。
  const mode = (loopbackOk && cfg.audioMode === 'isolated' && isWindow) ? 'isolated' : 'all_tracks';
  const sources = [];
  const labels = [];
  const addApp = (pid, created, name) => {
    if (sources.some((s) => s.kind === 'app' && s.pid === pid)) return;
    sources.push({ kind: 'app', pid, created_at_unix_ms: created ?? 0 });
    labels.push(`应用 · ${name}`);
  };
  if (isWindow && loopbackOk) {
    addApp(target.pid, target.created_at_unix_ms, target.app_name);
    if (mode === 'all_tracks' && cfg.srcSystem) { sources.push({ kind: 'system' }); labels.push('系统音频'); }
  } else if (!isWindow && cfg.srcSystem) {
    sources.push({ kind: 'system' });
    labels.push('系统音频');
  } else if (isWindow && !loopbackOk && cfg.srcSystem) {
    // 降级环境（无进程环回）：窗口目标只余系统/麦克风可选
    sources.push({ kind: 'system' });
    labels.push('系统音频');
  }
  if (cfg.srcMic && state.settings?.mic_device) {
    sources.push({ kind: 'mic', device_id: state.settings.mic_device });
    labels.push('麦克风');
  }
  if (mode === 'all_tracks' && loopbackOk) {
    for (const pid of cfg.srcApps) {
      const a = state.audioSources.find((x) => x.pid === pid);
      if (a) addApp(a.pid, a.created_at_unix_ms, a.app_name);
    }
  }
  return { mode, sources, labels };
}

// M18/REQ-23：区域音轨计划独立分支（不得复用显示器 buildAudioPlan）。
function buildRegionAudioPlan() {
  const cfg = state.cfg;
  const ra = state.regionAudio;
  const loopbackOk = state.caps?.loopback_ok !== false;
  const mode = (loopbackOk && cfg.audioMode === 'isolated') ? 'isolated' : 'all_tracks';
  const sources = [];
  const labels = [];
  const addApp = (pid, created, name) => {
    if (sources.some((s) => s.kind === 'app' && s.pid === pid)) return;
    sources.push({ kind: 'app', pid, created_at_unix_ms: created ?? 0 });
    labels.push(`应用 · ${name}`);
  };
  if (mode === 'isolated') {
    // 区域隔离 = 仅所选应用（不隐式设备混音/麦克风/端点）。
    for (const pid of ra.apps) {
      const a = state.audioSources.find((x) => x.pid === pid);
      if (a) addApp(a.pid, a.created_at_unix_ms, a.app_name);
    }
    return { mode, sources, labels };
  }
  // 顺序 [endpoint, system, mic, apps]（与显示器/窗口标签序一致）。
  if (ra.endpointSelected && ra.endpointDeviceId) {
    const ep = state.renderEndpoints.find((e) => e.device_id === ra.endpointDeviceId);
    sources.push({ kind: 'render_endpoint', device_id: ra.endpointDeviceId });
    labels.push(`端点 · ${ep?.friendly_name || ra.endpointDeviceId}`);
  }
  if (ra.systemSelected) {
    sources.push({ kind: 'system' });
    labels.push('系统音频');
  }
  if (ra.micSelected && state.settings?.mic_device) {
    sources.push({ kind: 'mic', device_id: state.settings.mic_device });
    labels.push('麦克风');
  }
  for (const pid of ra.apps) {
    const a = state.audioSources.find((x) => x.pid === pid);
    if (a) addApp(a.pid, a.created_at_unix_ms, a.app_name);
  }
  return { mode, sources, labels };
}

function regionSourceCount() {
  const ra = state.regionAudio;
  if (state.cfg.audioMode === 'isolated' && state.caps?.loopback_ok !== false) return ra.apps.size;
  return (ra.endpointSelected ? 1 : 0) + (ra.systemSelected ? 1 : 0)
    + (ra.micSelected && state.settings?.mic_device ? 1 : 0) + ra.apps.size;
}

// 启动执行已并入 doStart(plan)（M19b2/REQ-3：DTO 由 buildStartPlan 单一来源产出）。

function registerRoute(id, meta) {
  const existing = state.routes.get(id);
  if (existing) {
    // REQ-12：重注册只补充展示元数据，不重置状态/起始时间/成品路径。
    // 已有终态（done/failed）不得因启动回包/快照再次注册而降级。
    Object.assign(existing, meta);
  } else {
    state.routes.set(id, {
      id, ...meta,
      state: 'starting',
      startedAt: Date.now(),
      error: null, progress: 0, path: null, paths: null,
    });
  }
  // 应用先于注册到达的事件（后端可能在 start_routes 返回前已推送终态/recording）
  const pending = pendingStates.get(id);
  if (pending) {
    pendingStates.delete(id);
    applyPending(state.routes.get(id), pending);
  }
  renderRoutes();
}

// 早于注册到达的 route_state/route_error/route_finalized 暂存
// （真实后端下 subscribe 快照与启动回包存在时序竞争；M16/REQ-12）。
const pendingStates = new Map();

function mergePending(id, patch) {
  const prev = pendingStates.get(id) || {};
  const incomingState = patch.state ? mapState(patch.state) : undefined;
  let state = incomingState || prev.state;
  // REQ-12：注册前缓存的事件同样受终态单调护栏约束——已缓存 done/failed 后，
  // 迟到的更早状态不得覆盖；done（成品就绪）优先于 failed。
  if (prev.state && isTerminal(prev.state)) {
    if (!incomingState || !isTerminal(incomingState)) state = prev.state;
    else state = prev.state === 'done' || incomingState === 'done' ? 'done' : 'failed';
  }
  pendingStates.set(id, {
    ...prev,
    ...patch,
    state,
    error: patch.error || prev.error,
    paths: patch.paths || prev.paths,
    final_path: patch.final_path || prev.final_path,
    stop_notice: patch.stop_notice || prev.stop_notice,
  });
}

// ---------------------------------------------------------------------------
// 录制视图 — 录制中卡片
// ---------------------------------------------------------------------------
const STATE_LABEL = {
  starting: ['准备中', 'warn'],
  recording: ['录制中', 'rec'],
  stopping: ['停止中', 'warn'],
  finalizing: ['收尾中', 'warn'],
  done: ['已完成', 'ok'],
  failed: ['失败', 'err'],
  error: ['错误', 'err'],
};

function renderRoutes() {
  const mount = $('#routes-mount');
  const routes = [...state.routes.values()];
  const active = routes.filter((r) => !['done', 'failed'].includes(r.state)).length;
  $('#nav-rec-count').hidden = active === 0;
  $('#nav-rec-count').textContent = active;

  if (!routes.length) { mount.innerHTML = ''; return; }
  const anyActive = routes.some((r) => ['recording', 'starting'].includes(r.state));
  mount.innerHTML = `
    <div class="routes-head">
      <span class="rec-dot" ${anyActive ? '' : 'style="animation:none;background:var(--text-faint);"'}></span>
      <h3>录制任务（${routes.length}）</h3>
      <span class="spacer" style="flex:1;"></span>
      <button class="btn-sm btn-rec" id="btn-stop-all" ${anyActive ? '' : 'disabled'}>全部停止</button>
    </div>
    <div class="routes-grid">${routes.map(routeCard).join('')}</div>`;

  $('#btn-stop-all').addEventListener('click', async () => {
    try { await api.stopAll(); } catch (e) { toast('err', `全部停止失败：${e}`); }
  });
  $$('[data-stop]', mount).forEach((b) => b.addEventListener('click', async () => {
    b.disabled = true;
    try { await api.stopRoute(b.dataset.stop); } catch (e) { toast('err', `停止失败：${e}`); b.disabled = false; }
  }));
  $$('[data-dismiss]', mount).forEach((b) => b.addEventListener('click', () => {
    state.routes.delete(b.dataset.dismiss);
    renderRoutes();
  }));
}

function routeCard(r) {
  const [stateLabel, tone] = STATE_LABEL[r.state] || [r.state, ''];
  const running = ['starting', 'recording'].includes(r.state);
  const cls = ['error', 'failed'].includes(r.state) ? 'error' : r.state === 'done' ? 'done' : r.state === 'recording' ? 'recording' : '';
  const timer = r.state === 'done' && r.finishedAt
    ? `<span class="route-timer small">时长 ${fmtTime(r.finishedAt - r.startedAt)}</span>`
    : r.state === 'failed'
    ? `<span class="route-timer small muted">已停止</span>`
    : `<span class="route-timer" data-timer="${r.id}">${fmtTime(Date.now() - r.startedAt)}</span>`;

  let bodyHtml = '';
  if (running) {
    // route_level 暂无数据源（M9-B §8-1）：真实模式下录制 3s 无电平事件则如实标注
    bodyHtml = r.levelNote
      ? '<div class="muted" style="font-size:12.5px;">电平读数待管线接入，录制本身不受影响。</div>'
      : r.trackLabels.map((l, i) => `
      <div class="meter-row">
        <span class="lbl" title="${esc(l)}">${esc(l)}</span>
        <span class="meter"><i data-meter="${r.id}:${i}"></i></span>
      </div>`).join('');
  } else if (r.state === 'finalizing') {
    bodyHtml = `
      <div class="muted" style="font-size:12.5px;">正在后台合并分段并校验，可继续进行其它操作。</div>
      <div class="progress"><i style="width:${Math.round((r.progress || 0) * 100)}%"></i></div>`;
  } else if (r.state === 'stopping') {
    bodyHtml = '<div class="muted" style="font-size:12.5px;">已请求停止，正在后台收尾（合并分段并校验）…</div>';
  } else if (r.state === 'done') {
    bodyHtml = (r.paths || [r.path]).filter(Boolean).map((p) => `<div class="route-path">${esc(p)}</div>`).join('');
  }
  const errHtml = r.error ? `<div class="route-err">${esc(r.error.code)}：${esc(r.error.message)}</div>` : '';
  const noticeHtml = r.stopNotice
    ? `<div class="route-err">${esc(r.stopNotice.code)}${r.stopNotice.endpoint ? `（${esc(r.stopNotice.endpoint)}）` : ''}：${esc(r.stopNotice.detail || '目标端点已失效；已保留已录制内容')}</div>`
    : '';

  return `
    <div class="route-card ${cls}">
      <div class="route-head">
        <span class="icon">${r.icon}</span>
        <span class="name" title="${esc(r.label)}">${esc(r.label)}</span>
        <span class="badge ${tone}">${r.state === 'recording' ? '<span class="rec-dot"></span>' : ''}${stateLabel}</span>
      </div>
      ${timer}
      ${bodyHtml}
      ${errHtml}
      ${noticeHtml}
      <div class="route-foot">
        <span class="spacer"></span>
        ${running ? `<button class="btn-sm btn-rec" data-stop="${r.id}">停止</button>` : ''}
        ${['done', 'failed'].includes(r.state) ? `<button class="btn-sm btn-ghost" data-dismiss="${r.id}">移除</button>` : ''}
      </div>
    </div>`;
}

// 计时器：500ms 步进刷新录制中卡片；真实模式下补「电平无数据源」标注（M9-B §8-1）
setInterval(() => {
  for (const r of state.routes.values()) {
    if (!['starting', 'recording'].includes(r.state)) continue;
    const el = $(`[data-timer="${r.id}"]`);
    if (el) el.textContent = fmtTime(Date.now() - r.startedAt);
    if (!isMock && !r.levelSeen && !r.levelNote && Date.now() - r.startedAt > 3000) {
      r.levelNote = true;
      renderRoutes();
    }
  }
}, 500);

// ---------------------------------------------------------------------------
// 事件（Channel）
// ---------------------------------------------------------------------------
subscribeEvents((evt) => {
  switch (evt.event) {
    case '_snapshot': {
      // subscribe 返回的状态快照（REQ-31 UI 重绑）：重建仍在活动的路由卡片与恢复列表。
      // 快照不含目标名/音轨标签（RouteStateOut 仅 id/state/final_path），重建卡片以 id 为名。
      // M16/REQ-12：迟到旧快照不得覆盖已收到的终态（mergeSnapshotRoute 终态护栏）。
      const snap = evt.snapshot || {};
      for (const rs of snap.routes || []) {
        if (!state.routes.has(rs.id)) {
          registerRoute(rs.id, { kind: 'video', label: rs.id, icon: ICONS.window, trackLabels: [] });
        }
        mergeSnapshotRoute(state.routes.get(rs.id), rs);
      }
      if (Array.isArray(snap.recovery)) {
        state.recovery = snap.recovery;
        renderRecovery();
      }
      renderRoutes();
      break;
    }
    case 'route_state': {
      const r = state.routes.get(evt.id);
      if (!r) {
        mergePending(evt.id, { state: evt.state, final_path: evt.final_path, stop_notice: evt.stop_notice });
        return;
      }
      applyRouteState(r, evt.state, evt.final_path, evt.stop_notice);
      renderRoutes();
      break;
    }
    case 'route_level': {
      // track 为 "a0"/"a1"… 字符串（M9-B §3.3）；mock 用数字下标，两种都兼容
      const ti = typeof evt.track === 'number' ? evt.track : Number(String(evt.track).replace(/\D/g, '')) || 0;
      const r = state.routes.get(evt.id);
      if (r) r.levelSeen = true;
      const bar = $(`[data-meter="${evt.id}:${ti}"]`);
      if (bar) bar.style.width = `${Math.round(Math.min(1, evt.rms) * 100)}%`;
      break;
    }
    case 'route_error': {
      const r = state.routes.get(evt.id);
      if (!r) {
        // 终态早于启动回包：先缓存（未知 route_id），注册后应用。
        mergePending(evt.id, {
          state: 'failed',
          error: { code: evt.code, message: evt.message },
          stop_notice: evt.stop_notice,
        });
        return;
      }
      // route_error 为终态事件（M9-B §3.3）；后续若仍有 route_finalized（保留内容的
      // 优雅停止）则以 finalize 为准覆盖状态，错误框保留展示。
      applyRouteError(r, evt.code, evt.message, evt.severity);
      if (evt.stop_notice) r.stopNotice = evt.stop_notice;
      renderRoutes();
      toast(evt.severity === 'error' ? 'err' : 'warn', `${r.label}：${evt.message}`);
      break;
    }
    case 'route_finalized': {
      const r = state.routes.get(evt.id);
      if (!r) {
        mergePending(evt.id, {
          state: 'finalized',
          final_path: evt.path,
          path: evt.path,
          paths: evt.paths,
          stop_notice: evt.stop_notice,
        });
        toast('ok', '录制已就绪', evt.path);
        renderRoutes();
        break;
      }
      applyRouteFinalized(r, evt.path, evt.paths, evt.stop_notice);
      toast('ok', `${r ? r.label : '录制'}已就绪`, evt.path);
      renderRoutes();
      break;
    }
    case 'recovery_found':
      loadRecovery();
      break;
    case 'merge_progress': {
      // 仅 mock 会发（后端 sr-merge 同步 API 无进度钩子，M9-B §8-3）；
      // 收到事件则从不定态进度条切换为确定进度。
      state.merge.progress = evt.progress;
      const wrap = $('#merge-progress .progress');
      if (wrap) {
        wrap.classList.remove('indet');
        wrap.firstElementChild.style.width = `${Math.round(evt.progress * 100)}%`;
      }
      break;
    }
  }
});

// ---------------------------------------------------------------------------
// 截图视图
// ---------------------------------------------------------------------------
function renderScreenshotView() {
  renderShotLists();
  renderShotHistory();
}

function renderShotLists() {
  const mon = $('#shot-monitors');
  const win = $('#shot-windows');
  if (!mon || !win) return;
  mon.innerHTML = state.monitors.length
    ? state.monitors.map((m) => `
      <div class="shot-row">
        <span class="main"><span class="name">${esc(m.name)}${m.primary ? '（主）' : ''}</span>
        <span class="meta" style="display:block;">${m.width}×${m.height}</span></span>
        <button class="btn-sm" data-shot-monitor="${m.index}">截图</button>
      </div>`).join('')
    : `<div class="empty">${ICONS.inbox}<br>${state.targetsLoading ? '加载中…' : '未发现显示器（先在录制页刷新）'}</div>`;
  win.innerHTML = state.windows.length
    ? state.windows.map((w) => `
      <div class="shot-row">
        <span class="main"><span class="name">${esc(w.title)}</span>
        <span class="meta" style="display:block;">${esc(w.app_name)} · ${esc(w.process_name)}</span></span>
        <button class="btn-sm" data-shot-window="${w.hwnd}">截图</button>
      </div>`).join('')
    : `<div class="empty">${ICONS.inbox}<br>${state.targetsLoading ? '加载中…' : '未发现窗口（先在录制页刷新）'}</div>`;

  $$('[data-shot-monitor]').forEach((b) => b.addEventListener('click', () => doScreenshot({ kind: 'monitor', index: Number(b.dataset.shotMonitor) }, b)));
  $$('[data-shot-window]').forEach((b) => b.addEventListener('click', () => doScreenshot({ kind: 'window', hwnd: Number(b.dataset.shotWindow) }, b)));
}

async function doScreenshot(req, btn) {
  if (btn) { btn.disabled = true; btn.innerHTML = '<span class="spin"></span>'; }
  try {
    const { path } = await api.takeScreenshot(req);
    state.shotHistory.unshift({ path, when: Date.now(), kind: req.kind });
    toast('ok', '截图已保存', path);
    renderShotHistory();
  } catch (e) {
    toast('err', `截图失败：${e}`);
  }
  if (btn) { btn.disabled = false; btn.textContent = '截图'; }
}

function renderShotHistory() {
  const el = $('#shot-history');
  el.innerHTML = state.shotHistory.length
    ? state.shotHistory.map((h) => `
      <div class="history-item">
        <span class="when">${fmtWhen(h.when)}</span>
        <span class="path" title="${esc(h.path)}">${esc(h.path)}</span>
      </div>`).join('')
    : `<div class="empty">${ICONS.inbox}<br>本次会话还没有截图</div>`;
}

// M18/REQ-21：主窗先建立监听并确认就绪，再打开 record overlay。
let regionListenerPromise = null;
function ensureRegionTargetListener() {
  if (!regionListenerPromise) {
    regionListenerPromise = Promise.all([
      api.listenRegionTarget((payload) => {
        if (!payload || !payload.rect) return;
        // M18b：只接受当前 request_id 的事件；stale 不写 state、不 toast。
        if (!state.regionRequestId || payload.request_id !== state.regionRequestId) {
          console.debug('[app] 忽略 stale region_target_added:', payload.request_id, '当前:', state.regionRequestId);
          return;
        }
        state.regionTarget = {
          display: payload.display || { device_name: '', device_path: null },
          rect: payload.rect,
          request_id: payload.request_id,
        };
        state.regionRequestId = null; // 成功交付：当前请求结束
        renderConfig();
        renderStartBar();
        toast('ok', `区域已就绪：${payload.rect.w} × ${payload.rect.h}`);
      }),
      api.listenRegionRecordCancelled((payload) => {
        if (!payload || payload.request_id !== state.regionRequestId) return;
        state.regionRequestId = null;
        state.regionTarget = null;
        renderConfig();
        renderStartBar();
      }),
    ]).catch((e) => {
      regionListenerPromise = null;
      throw e;
    });
  }
  return regionListenerPromise;
}

function newRegionRequestId() {
  if (window.crypto && typeof window.crypto.randomUUID === 'function') return window.crypto.randomUUID();
  return `region-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

$('#btn-region-record').addEventListener('click', async (e) => {
  const btn = e.currentTarget;
  btn.disabled = true;
  try {
    await ensureRegionTargetListener();
    // 新一轮框选先清空旧区域目标：Esc 取消后不得残留上一次选区。
    state.regionTarget = null;
    const requestId = newRegionRequestId();
    state.regionRequestId = requestId;
    renderConfig();
    renderStartBar();
    await api.openRegionOverlay('record', requestId);
  } catch (err) {
    state.regionRequestId = null;
    toast('err', `无法打开区域录制覆盖层：${err}`);
  } finally {
    btn.disabled = false;
  }
});

// 区域框选（REQ-8）：交互由后端全屏覆盖层窗口完成（label=region-overlay，
// 加载 ui/region.html，负责选区绘制与 client→物理坐标换算）；此处只打开覆盖层。
$('#btn-region-shot').addEventListener('click', async (e) => {
  const btn = e.currentTarget;
  btn.disabled = true;
  try {
    await api.openRegionOverlay();
  } catch (err) {
    toast('err', `无法打开区域截图覆盖层：${err}`);
  } finally {
    btn.disabled = false;
  }
});

// ---------------------------------------------------------------------------
// 合并视图
// ---------------------------------------------------------------------------
async function renderMerge() {
  const list = $('#merge-list');
  list.innerHTML = '<div class="skel"></div>'.repeat(3);
  try { state.recentFiles = await api.recentRecordings(); } catch { state.recentFiles = []; }
  if (!$('#merge-out').value && state.settings) {
    // 合并产物固定 MKV（多视频轨；REQ-18），不随 output_container 变化。
    $('#merge-out').value = `${state.settings.output_dir}\\merged_${fileStamp()}.mkv`;
  }
  renderMergeList();
}

function renderMergeList() {
  const list = $('#merge-list');
  const items = [
    ...state.recentFiles.filter((f) => f.kind !== 'shot').map((f) => ({ path: f.path, meta: `${f.kind === 'audio' ? '音频' : '视频'} · ${fmtWhen(f.when)}` })),
    ...state.merge.extra.map((p) => ({ path: p, meta: '手动添加' })),
  ];
  list.innerHTML = items.length
    ? items.map((f, i) => {
      const order = [...state.merge.picked].indexOf(f.path);
      return `
        <div class="history-item">
          <input type="checkbox" data-merge-pick="${esc(f.path)}" ${state.merge.picked.has(f.path) ? 'checked' : ''} />
          ${order >= 0 ? `<span class="merge-order">${order + 1}</span>` : ''}
          <span class="path" title="${esc(f.path)}">${esc(f.path)}</span>
          <span class="when">${f.meta}</span>
        </div>`;
    }).join('')
    : `<div class="empty">${ICONS.inbox}<br>暂无可合并的录制文件；可在上方手动添加路径</div>`;

  $$('[data-merge-pick]', list).forEach((cb) => cb.addEventListener('change', () => {
    cb.checked ? state.merge.picked.add(cb.dataset.mergePick) : state.merge.picked.delete(cb.dataset.mergePick);
    renderMergeList();
  }));
  $('#btn-merge').disabled = state.merge.picked.size < 2 || state.merge.running;
}

$('#merge-add-btn').addEventListener('click', () => {
  const v = $('#merge-add-input').value.trim();
  if (!v) return;
  state.merge.extra.push(v);
  state.merge.picked.add(v);
  $('#merge-add-input').value = '';
  renderMergeList();
});

$('#btn-merge').addEventListener('click', async () => {
  const paths = [...state.merge.picked];
  const out = $('#merge-out').value.trim();
  if (!out) { toast('warn', '请填写输出路径'); return; }
  if (paths.includes(out)) { toast('warn', '输出路径不能与输入文件相同'); return; }
  state.merge.running = true;
  state.merge.progress = 0;
  $('#btn-merge').disabled = true;
  $('#merge-progress').innerHTML = `
    <div class="muted" style="font-size:12.5px;margin-bottom:6px;">合并中（流拷贝，不重编码）…</div>
    <div class="progress indet"><i></i></div>`;
  try {
    const res = await api.mergeRecordings(paths, out);
    $('#merge-progress').innerHTML = `<div class="notice info" style="margin:0;">合并完成<span class="mono" style="display:block;margin-top:4px;word-break:break-all;">${esc(res.out)}</span></div>`;
    state.merge.picked.clear();
    toast('ok', '合并完成', res.out);
  } catch (e) {
    $('#merge-progress').innerHTML = `<div class="notice err" style="margin:0;">合并失败：${esc(e)}（输入文件已保留）</div>`;
    toast('err', `合并失败：${e}`);
  }
  state.merge.running = false;
  renderMergeList();
});

// ---------------------------------------------------------------------------
// 设置视图
// ---------------------------------------------------------------------------
function renderSettings() {
  const s = state.settings;
  const panel = $('#settings-panel');
  if (!s) { panel.innerHTML = '<div class="skel"></div>'.repeat(4); return; }
  const encs = state.caps?.encoders || [];
  panel.innerHTML = `
    <div class="field">
      <label>输出目录</label>
      <input type="text" id="set-outdir" value="${esc(s.output_dir)}" />
      <div class="hint">录制分段临时目录位于输出卷内（&lt;输出目录&gt;\\.screenrec_tmp），保证收尾同卷提交。</div>
    </div>
    <div class="field-row">
      <div class="field"><label>默认帧率</label>
        <select id="set-fps">
          <option value="30" ${s.fps === 30 ? 'selected' : ''}>30 fps</option>
          <option value="60" ${s.fps === 60 ? 'selected' : ''}>60 fps</option>
        </select>
      </div>
      <div class="field"><label>分段时长（秒）</label>
        <input type="number" id="set-seg" min="1" max="60" value="${s.segment_secs}" />
        <div class="hint">崩溃时最多丢失一个分段的时长。</div>
      </div>
    </div>
    <div class="field">
      <label>录制音频采样率</label>
      <select id="set-sr">
        <option value="44100" ${(s.audio_sample_rate || 48000) === 44100 ? 'selected' : ''}>44.1 kHz</option>
        <option value="48000" ${(s.audio_sample_rate || 48000) === 48000 ? 'selected' : ''}>48 kHz（默认）</option>
        <option value="88200" ${(s.audio_sample_rate || 48000) === 88200 ? 'selected' : ''}>88.2 kHz</option>
        <option value="96000" ${(s.audio_sample_rate || 48000) === 96000 ? 'selected' : ''}>96 kHz</option>
      </select>
      <div class="hint" id="set-sample-rate-note">原生混音率查询中…</div>
    </div>
    <div class="field">
      <label>默认音轨模式</label>
      <select id="set-audiomode">
        <option value="isolated" ${s.default_audio_mode === 'isolated' ? 'selected' : ''}>隔离（仅目标应用）</option>
        <option value="all_tracks" ${s.default_audio_mode === 'all_tracks' ? 'selected' : ''}>全音轨（所有来源独立成轨）</option>
      </select>
    </div>
    <div class="field">
      <label>成品格式</label>
      <select id="set-container">
        <option value="mkv" ${(s.output_container || 'mkv') === 'mkv' ? 'selected' : ''}>MKV（默认）</option>
        <option value="mp4" ${s.output_container === 'mp4' ? 'selected' : ''}>MP4</option>
      </select>
      <div class="hint">录制内部始终 MKV 分段（崩溃安全）；MP4 为收尾流拷贝；多音轨 MP4 播放器通常选一轨播放（不自动混音）。</div>
    </div>
    <div class="field">
      <label>麦克风设备</label>
      <select id="set-mic">
        <option value="">不启用</option>
        ${state.audioDevices.map((d) => `<option value="${esc(d.id)}" ${s.mic_device === d.id ? 'selected' : ''}>${esc(d.name)}</option>`).join('')}
      </select>
      ${state.audioDevices.length ? '' : '<div class="hint">未检测到麦克风设备或权限未允许。</div>'}
    </div>
    <div class="field">
      <label>编码器</label>
      <select id="set-enc">
        <option value="auto" ${s.encoder === 'auto' ? 'selected' : ''}>自动（启动前探测：nvenc → qsv → amf → h264_mf）</option>
        ${encs.map((e) => `<option value="${esc(e.name)}" ${s.encoder === e.name ? 'selected' : ''} ${e.available ? '' : 'disabled'}>${esc(e.name)}${e.available ? '' : '（本机不可用）'}</option>`).join('')}
      </select>
    </div>
    <label class="check-row"><input type="checkbox" id="set-cursor" ${s.cursor ? 'checked' : ''} /><span>默认捕获光标</span></label>
    <label class="check-row"><input type="checkbox" id="set-keep-segments" ${s.keep_segments ? 'checked' : ''} /><span>保留原始分段（调试/排查用）</span></label>
    <div class="hint" style="margin:-6px 0 0 26px;">默认关闭：成品验证成功后自动清理 .screenrec_tmp 会话目录；录制失败时始终保留原段。</div>
    <label class="check-row"><input type="checkbox" id="set-window-title" ${s.include_window_title_in_filename ? 'checked' : ''} /><span>成品文件名包含窗口标题</span></label>
    <div class="hint" style="margin:-6px 0 0 26px;">默认关闭。开启后窗口标题（可能含文档名/聊天对象）会写入文件名。</div>
    <div style="margin-top:18px;display:flex;gap:10px;align-items:center;">
      <button class="btn-primary" id="btn-save-settings">保存设置</button>
      <span id="settings-msg" class="muted" style="font-size:12.5px;"></span>
    </div>`;

  $('#btn-save-settings').addEventListener('click', async () => {
    const form = {
      output_dir: $('#set-outdir').value.trim(),
      fps: Number($('#set-fps').value),
      audio_sample_rate: Number($('#set-sr').value),
      output_container: $('#set-container').value,
      cursor: $('#set-cursor').checked,
      default_audio_mode: $('#set-audiomode').value,
      mic_device: $('#set-mic').value || null,
      encoder: $('#set-enc').value,
      segment_secs: Math.max(1, Number($('#set-seg').value) || 5),
      keep_segments: $('#set-keep-segments').checked,
      include_window_title_in_filename: $('#set-window-title').checked,
    };
    const msg = $('#settings-msg');
    try {
      // REQ-4：在写队列内读取最新 Settings，保留表单未覆盖的字段（尤其 ack）。
      await enqueueSettingsWrite(async () => {
        const base = await api.getSettings();
        const next = { ...base, ...form };
        await api.setSettings(next);
        state.settings = next;
      });
      applySettingsDefaults();
      msg.textContent = '已保存';
      toast('ok', '设置已保存');
    } catch (e) {
      msg.textContent = `保存失败：${e}`;
      toast('err', `保存失败：${e}`);
    }
  });

  // REQ-2 刷新时机：设置页打开 + 麦克风设备选择变更。未知/失败只在展示层降级，
  // 不参与重采样比率推导，也不阻断录制（捕获侧另有 REQ-7 回退）。
  $('#set-sr').addEventListener('change', () => renderSampleRateNote());
  $('#set-mic').addEventListener('change', () => refreshSampleRates());
  refreshSampleRates();
}

/// M19d1：About 页（版本/系统/FFmpeg 构建 + 固定 allowlist 文档懒加载）。
async function renderAbout() {
  const meta = $('#about-meta');
  const docs = $('#about-docs');
  const pre = $('#about-doc-text');
  if (!meta || !docs || !pre) return;

  if (!state.about.info && !state.about.loading) {
    state.about.loading = true;
    meta.textContent = '加载中…';
    try {
      state.about.info = await api.getAppInfo();
    } catch (e) {
      meta.textContent = '读取关于信息失败';
      state.about.loading = false;
      return;
    }
    state.about.loading = false;
  }
  if (!state.about.info) return;
  const info = state.about.info;
  meta.innerHTML = `
    <div class="hint">版本：${esc(info.version || 'unknown')}</div>
    <div class="hint">Windows：${esc(state.caps?.os_build || '未知')}</div>
    <div class="hint">FFmpeg：${esc(info.ffmpeg_build || 'unknown')}</div>
    <div class="hint">本版本只展示已随包的 FFmpeg 文档；第三方通知、EULA、隐私条款不在本页。</div>`;
  docs.innerHTML = (info.docs || []).map((d) => `
    <div class="hint" style="margin:8px 0;">
      <button class="btn-sm btn-ghost" data-about-doc="${esc(d.id)}">${esc(d.title)}</button>
      <span class="muted" style="font-size:12px;margin-left:8px;">${esc(d.status)} · ${Number(d.bytes) || 0} B</span>
    </div>`).join('');
  $$('#about-docs [data-about-doc]').forEach((btn) => {
    btn.addEventListener('click', () => loadAboutDoc(btn.dataset.aboutDoc, false));
  });
  if (state.about.selected) loadAboutDoc(state.about.selected, true);
}

async function loadAboutDoc(id, quiet) {
  const pre = $('#about-doc-text');
  if (!pre) return;
  pre.hidden = false;
  pre.textContent = '加载中…';
  try {
    const doc = await api.readLegalDoc(id);
    pre.textContent = doc.text;
    state.about.selected = id;
  } catch (e) {
    pre.textContent = '文档不可用';
    if (!quiet) toast('err', '读取文档失败');
  }
}

/// 原生率展示（REQ-2）：渲染端点 / 麦克风 = 设备混音率；进程环回如实标注
/// 「跟随引擎混音率（通常 48k）」；所选 > 已知原生率时提示升采样。
function renderSampleRateNote() {
  const note = $('#set-sample-rate-note');
  if (!note) return;
  const selected = Number($('#set-sr')?.value || state.settings?.audio_sample_rate || 48000);
  const info = state.endpointRates;
  const describe = (label, status, value, error) => {
    if (status === 'loading') return `${label}：查询中…`;
    if (status === 'error') return `${label}：未知（查询失败${error ? `：${error}` : ''}）`;
    if (status === 'ok' && value) return `${label}：${value.rate} Hz（${value.device_name}）`;
    if (status === 'unknown') return `${label}：未知`;
    return `${label}：未查询`;
  };
  const parts = [];
  parts.push(describe('渲染端点原生混音率', info.renderStatus, info.render, info.renderError));
  const micDevice = $('#set-mic')?.value || null;
  if (!micDevice) parts.push('麦克风采集：未启用');
  else parts.push(describe('麦克风原生混音率', info.micStatus, info.mic, info.micError));
  parts.push('进程环回：跟随引擎混音率（通常 48k）');

  // 只使用「已知」原生率计算提示；未知/失败值不参与（REQ-2）。
  const known = [];
  if (info.renderStatus === 'ok' && info.render) {
    known.push({ label: '渲染端点', rate: info.render.rate });
  }
  if (micDevice && info.micStatus === 'ok' && info.mic) {
    known.push({ label: '麦克风', rate: info.mic.rate });
  }
  const exceeded = known.filter((k) => selected > k.rate);
  if (exceeded.length) {
    parts.push(`升采样（不提升真实音质）：所选 ${selected} Hz 高于${exceeded
      .map((k) => `${k.label}原生 ${k.rate} Hz`)
      .join('、')}。`);
  }
  note.textContent = parts.join(' ｜ ');
}

/// 查询当前渲染端点 + 所选麦克风设备原生混音率（并发；seq 防过期结果覆盖）。
async function refreshSampleRates() {
  const note = $('#set-sample-rate-note');
  if (!note) return;
  const micDevice = $('#set-mic')?.value || null;
  const seq = ++state.endpointRates.seq;
  state.endpointRates.micDevice = micDevice;
  state.endpointRates.renderStatus = 'loading';
  state.endpointRates.micStatus = micDevice ? 'loading' : 'idle';
  state.endpointRates.renderError = null;
  state.endpointRates.micError = null;
  renderSampleRateNote();

  const probe = async (deviceId) => {
    try {
      const value = await api.getEndpointMixRate(deviceId);
      return value ? { status: 'ok', value } : { status: 'unknown', value: null };
    } catch (e) {
      return { status: 'error', value: null, error: String(e) };
    }
  };
  const [render, mic] = await Promise.all([
    probe(null),
    micDevice ? probe(micDevice) : Promise.resolve({ status: 'idle', value: null }),
  ]);
  if (seq !== state.endpointRates.seq) return; // 过期结果（设备/页面已切换）
  state.endpointRates.render = render.value;
  state.endpointRates.renderStatus = render.status;
  state.endpointRates.renderError = render.error || null;
  state.endpointRates.mic = mic.value;
  state.endpointRates.micStatus = mic.status;
  state.endpointRates.micError = mic.error || null;
  renderSampleRateNote();
}

function applySettingsDefaults() {
  const s = state.settings;
  if (!s) return;
  state.cfg.fps = s.fps;
  state.cfg.cursor = s.cursor;
  state.cfg.audioMode = s.default_audio_mode;
  if (!state.cfg.outputDir) state.cfg.outputDir = s.output_dir;
  if (!state.audioOnly.outputDir) state.audioOnly.outputDir = s.output_dir;
}

// ---------------------------------------------------------------------------
// 恢复横幅（AC-10 / REQ-22）
// ---------------------------------------------------------------------------
async function loadRecovery() {
  try {
    const { sessions } = await api.recoverSessions();
    state.recovery = sessions;
  } catch (e) {
    toast('err', `恢复扫描失败：${e}`);
    state.recovery = [];
  }
  renderRecovery();
}

function renderRecovery() {
  const mount = $('#recovery-mount');
  mount.innerHTML = state.recovery.map((s) => {
    // M9-B §8-7/8-8 规则：
    //  - 全部路由已收尾（final_path 非空 / state=final）→ 已收尾残留，仅给「清理」
    //  - 启动回滚空会话（manifest_ok 且无密封段无尾段）→ 仅给「清理」
    //  - active=true → 不给「丢弃」（discard_session 无条件删目录）
    const allFinal = s.routes.length > 0 && s.routes.every((r) => r.final_path || r.state === 'final');
    const emptyResidue = s.manifest_ok && !(s.sealed_segments && s.sealed_segments.length) && !s.tail_segment;
    const cleanupOnly = allFinal || emptyResidue;
    const title = allFinal ? '历史会话（已收尾，原段保留）'
      : emptyResidue ? '空会话残留（启动失败回滚）'
      : '发现未收尾的录制会话';
    const routeDesc = s.routes.map((r) => `${esc(r.name)}（${r.segments} 段）`).join('、') || '无路由记录';
    const meta = cleanupOnly
      ? `会话 ${esc(s.session_id)} · 开始于 ${fmtWhen(s.created_at)} · ${s.dir ? esc(s.dir) : ''}`
      : `会话 ${esc(s.session_id)} · ${s.routes.length} 路（${routeDesc}）· 开始于 ${fmtWhen(s.created_at)} — 上次程序异常退出，已写入的分段未丢失，可收尾为成品或丢弃。`;
    const buttons = cleanupOnly
      ? '<button class="btn-sm" data-act="discard">清理</button>'
      : `<button class="btn-sm btn-primary" data-act="finalize">收尾</button>
         ${s.active ? '' : '<button class="btn-sm" data-act="discard">丢弃</button>'}`;
    return `
    <div class="recovery-banner" data-session="${esc(s.session_id)}">
      <span class="spin" hidden></span>
      <div class="grow">
        <div class="title">${title}</div>
        <div class="meta">${meta}</div>
      </div>
      ${buttons}
    </div>`;
  }).join('');

  $$('.recovery-banner', mount).forEach((banner) => {
    const id = banner.dataset.session;
    const spin = $('.spin', banner);
    $$('button', banner).forEach((btn) => btn.addEventListener('click', async () => {
      if (btn.dataset.act === 'discard' && btn.dataset.confirm !== '1') {
        btn.dataset.confirm = '1';
        btn.textContent = btn.textContent === '清理' ? '确认清理？（会话目录将删除）' : '确认丢弃？（分段将删除）';
        btn.classList.add('btn-rec');
        return;
      }
      $$('button', banner).forEach((b) => (b.disabled = true));
      spin.hidden = false;
      try {
        if (btn.dataset.act === 'finalize') {
          const res = await api.finalizeSession(id);
          const finals = res?.finals || [];
          toast('ok', `会话收尾完成（${finals.length} 个成品）`, finals[0]?.path);
        } else {
          await api.discardSession(id);
          toast('ok', '已清理该会话的残留分段');
        }
        state.recovery = state.recovery.filter((x) => x.session_id !== id);
        banner.remove();
      } catch (e) {
        spin.hidden = true;
        $$('button', banner).forEach((b) => (b.disabled = false));
        toast('err', `操作失败：${e}（残留分段已保留，可重试）`);
      }
    }));
  });
}

// ---------------------------------------------------------------------------
// 键盘操作
// ---------------------------------------------------------------------------
document.addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
    // REQ-3：与开始按钮同一门禁；按住重复触发不算第二次确认。
    if (e.repeat) return;
    if (state.view === 'record') $('#btn-start')?.click();
  } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'r') {
    if (state.view === 'record') { e.preventDefault(); refreshTargets(); }
  }
});

// ---------------------------------------------------------------------------
// 初始化
// ---------------------------------------------------------------------------
async function init() {
  $('#env-badge').textContent = isMock ? '演示数据（mock）' : '已连接后端';
  $('#env-badge').classList.toggle('real', !isMock);

  try { state.settings = await api.getSettings(); } catch (e) { toast('err', `读取设置失败：${e}`); }
  applySettingsDefaults();
  try { state.audioDevices = await api.listAudioDevices(); } catch { state.audioDevices = []; }
  try {
    state.caps = await api.probeCapabilities();
    $('#nav-foot-os').textContent = `Windows ${state.caps.os_build}`;
  } catch (e) {
    state.caps = null;
    toast('err', `环境探测失败：${e}`);
  }

  renderCaps();
  renderLeftPanel();
  renderConfig();
  renderStartBar();
  renderShotHistory();
  loadRecovery();
  refreshTargets();
  // REQ-21 就绪顺序：主窗先建监听（就绪确认）再允许打开 record overlay。
  ensureRegionTargetListener().catch((e) => console.warn('[app] region_target_added 监听失败：', e));

  // 录制模式切换
  $$('#mode-tabs button').forEach((b) => b.addEventListener('click', () => {
    state.recordMode = b.dataset.mode;
    $$('#mode-tabs button').forEach((x) => x.classList.toggle('active', x === b));
    renderLeftPanel();
    renderConfig();
    renderStartBar();
  }));
  $('#btn-refresh').addEventListener('click', refreshTargets);
}

init();
})();
