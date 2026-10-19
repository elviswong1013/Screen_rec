// ============================================================================
// ui/api.js — 命令封装层（唯一与后端交互的模块）
//
// 契约来源：specs/001-screen-recorder.md §3.3（commands / Channel）。
// 运行时两种模式：
//   - Tauri 环境（window.__TAURI__ 存在，withGlobalTauri）：走真实 invoke。
//     注意 Tauri v2 参数约定：Rust snake_case 参数在 JS 侧用 camelCase
//     （如 stop_route(route_id) → invoke('stop_route', { routeId })）。
//     后端录制命令（M9 待接线）落地时只需核对本文件 real 分支。
//   - 纯浏览器（无 __TAURI__）：自动降级为内置 mock —— 示例数据 + 模拟
//     事件流（电平 10Hz / 状态机 / 收尾进度 / 崩溃残留会话），
//     浏览器直接打开 index.html 即可驱动全部 UI 交互。
//
// 加载方式：经典脚本（非 ES module）。原因：file:// 协议下 Chrome/Edge 的 CORS
// 策略会拦截「外部 module import」，导致双击 index.html 时脚本完全不执行；
// 经典 <script src> 在 file:// 与 Tauri webview 中均可用。本文件通过
// window.SR 导出 { api, isMock, subscribeEvents }，app.js 依赖加载顺序。
//
// mock 场景开关（URL query，便于演示/自测）：
//   ?scenario=degraded   能力降级（无进程环回、麦克风权限拒绝、仅 h264_mf）
//   ?scenario=error      录制 6s 后首路注入 E_TARGET_LOST
//   ?scenario=empty      空窗口 / 空音源（演示空态）
//   ?scenario=norecovery 不返回崩溃残留会话
//   ?endpoints=ambiguous 端点匹配歧义（候选 a/b + 无关首项，供 REQ-10 回归）
// ============================================================================
(() => {
'use strict';

const isTauri = typeof window !== 'undefined' && !!window.__TAURI__;

// ---------------------------------------------------------------------------
// 事件订阅（Channel）
// handler(event) 收到 { event: 'route_state' | 'route_level' | 'route_error'
//   | 'route_finalized' | 'recovery_found' | 'merge_progress', ...payload }
// merge_progress / route_state.progress 为 UI 扩展字段，见 docs/design/ui-spec.md。
// ---------------------------------------------------------------------------
const listeners = new Set();

// 递归移除 `_ui_` 前缀的展示用元数据（真实 invoke 前调用）。
function stripUiMeta(v) {
  if (Array.isArray(v)) return v.map(stripUiMeta);
  if (v && typeof v === 'object') {
    const out = {};
    for (const [k, val] of Object.entries(v)) {
      if (!k.startsWith('_ui_')) out[k] = stripUiMeta(val);
    }
    return out;
  }
  return v;
}

function subscribeEvents(handler) {
  listeners.add(handler);
  return () => listeners.delete(handler);
}

function emit(evt) {
  for (const fn of listeners) {
    try { fn(evt); } catch (e) { console.error('[api] event handler error', e); }
  }
}

// ---------------------------------------------------------------------------
// 命令接口（UI 只使用此对象）
// ---------------------------------------------------------------------------
const api = isTauri ? makeReal() : makeMock();
const isMock = !isTauri;

// 经典脚本导出：app.js 从 window.SR 取用。
window.SR = { api, isMock, subscribeEvents };

// ---------------------------------------------------------------------------
// 真实后端（Tauri）
// 契约定稿：docs/milestones/m9-commands-report.md（M9-B）
//   - 顶层参数 camelCase（routeId/sessionId/outputDir），嵌套字段 snake_case
//   - subscribe(channel) 立即返回 StateSnapshot {routes, recovery}（REQ-31 重绑）
//   - probe_capabilities 原始字段（init_ok/mic_privacy:bool/canvas_limits.max_width…）
//     在本层归一化为 UI 内部形状，UI 代码不直接依赖后端字段名
// ---------------------------------------------------------------------------
function makeReal() {
  const { invoke, Channel } = window.__TAURI__.core;

  const ch = new Channel();
  ch.onmessage = (msg) => {
    // 后端事件格式：{ event: string, ...payload }（字段 snake_case）
    if (msg && typeof msg === 'object') emit(msg);
  };
  invoke('subscribe', { channel: ch })
    .then((snapshot) => emit({ event: '_snapshot', snapshot }))
    .catch((e) => console.warn('[api] subscribe 失败:', e));

  // ProbeOut → 内部形状（mock 分支产出同一形状）
  const normalizeProbe = (p) => ({
    os_build: p.os_build,
    os_supported: p.os_supported !== false,
    encoders: (p.encoders || []).map((e) => ({ name: e.name, available: !!e.init_ok, detail: e.detail })),
    selected_encoder: p.selected_encoder || null,
    loopback_ok: !!p.loopback_ok,
    mic_ok: !!p.mic_privacy,
    wgc_ok: !!p.wgc_ok,
    canvas_limits: {
      max_w: p.canvas_limits?.max_width ?? 7680,
      max_h: p.canvas_limits?.max_height ?? 4320,
    },
    layers: p.layers || null, // {loopback|mic|wgc: {status, detail}}（REQ-28 六态明细）
  });

  return {
    probeCapabilities: () => invoke('probe_capabilities').then(normalizeProbe),
    listTargets: () => invoke('list_targets'),
    // M16/REQ-9：渲染端点枚举（device_id/friendly_name/mix_rate/is_active/container_id）。
    listRenderEndpoints: () => invoke('list_render_endpoints'),
    listAudioSources: () => invoke('list_audio_sources'),
    listAudioDevices: () => invoke('list_audio_devices'),
    // M15/REQ-2：端点原生混音率（独立于 probe_capabilities；失败返回 null）。
    // 显式透传返回值，不做 normalizeProbe 式字段挑选（未知值不得推导重采样比率）。
    getEndpointMixRate: (deviceId) => invoke('get_endpoint_mix_rate', { deviceId: deviceId ?? null }),
    // `_ui_*` 字段为 mock 展示用元数据（标签/音轨名），发送前剥离，避免污染契约。
    startRoutes: (routes) => invoke('start_routes', { routes: stripUiMeta(routes) }),
    stopRoute: (routeId) => invoke('stop_route', { routeId }),
    stopAll: () => invoke('stop_all'),
    // 后端为单批次扁平参数 {sources, outputDir?}，一次会话 N 源 → N 个共享 handle 的 route id；
    // UI 仅发单配置（见 ui-spec.md §4）。
    startAudioOnly: (configs) => invoke('start_audio_only', {
      sources: stripUiMeta(configs[0].sources),
      outputDir: configs[0].output_dir,
    }),
    // 后端签名 {request: {target, output_dir?}}；req 即 target 三选一形状。
    takeScreenshot: (req) => invoke('take_screenshot', { request: { target: req } }),
    mergeRecordings: (paths, out) => invoke('merge_recordings', { paths, out }),
    recoverSessions: () => invoke('recover_sessions'),
    finalizeSession: (sessionId) => invoke('finalize_session', { sessionId }),
    discardSession: (sessionId) => invoke('discard_session', { sessionId }),
    getSettings: () => invoke('get_settings'),
    setSettings: (settings) => invoke('set_settings', { settings }),
    // M11-B 区域截图覆盖层契约（REQ-8；窗口生命周期与几何由后端管理，
    // 名称固定：get_overlay_geometry / open_region_overlay / close_region_overlay）。
    /**
     * @returns {Promise<{origin_x:number, origin_y:number, width:number, height:number, scale:number}>}
     *   虚拟桌面物理边界（origin 可为负）与 overlay 窗口缩放（CSS px → 物理 px）。
     */
    getOverlayGeometry: () => invoke('get_overlay_geometry'),
    /** 创建/复用无边框置顶全屏窗口（label=region-overlay，加载 ui/region.html）。 */
    openRegionOverlay: (mode = 'screenshot', requestId = null) =>
      invoke('open_region_overlay', { mode, requestId }),
    closeRegionOverlay: () => invoke('close_region_overlay'),
    // M18/REQ-21：区域录制上下文/确认/取消（record 路径绝不进截图命令）。
    getRegionContext: () => invoke('get_region_context'),
    confirmRegionRecord: (requestId, rect) =>
      invoke('confirm_region_record', { requestId, rect }),
    cancelRegionRecord: (requestId) => invoke('cancel_region_record', { requestId }),
    // 主窗必须先完成监听再打开覆盖层（REQ-21 就绪顺序）。
    listenRegionTarget: (handler) => window.__TAURI__.event.listen('region_target_added', (event) => handler(event.payload)),
    listenRegionRecordCancelled: (handler) => window.__TAURI__.event.listen('region_record_cancelled', (event) => handler(event.payload)),
    // mock 专属辅助在真实模式下的安全降级：
    recentRecordings: () => Promise.resolve([]),
  };
}

// ---------------------------------------------------------------------------
// Mock 实现
// ---------------------------------------------------------------------------
function makeMock() {
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const params = new URLSearchParams(location.search);
  const scenario = params.get('scenario') || '';
  // M15/REQ-2 行为 runner 注入：原生率档位 / 未知 / 查询失败。
  const nativeRate = Number(params.get('native_rate')) || 48_000;
  const micRate = Number(params.get('mic_rate')) || nativeRate;
  const nativeUnknown = params.get('native_unknown') === '1';
  const nativeFail = params.get('native_fail') === '1';
  const now = Date.now();

  // -- 示例数据（与 normalizeProbe 归一化后的内部形状一致） -------------------
  const capsDegraded = scenario === 'degraded';
  const caps = {
    os_build: '10.0.26200.9711',
    os_supported: true,
    encoders: [
      { name: 'h264_nvenc', available: !capsDegraded, detail: capsDegraded ? '未检测到 NVIDIA 设备' : 'ok' },
      { name: 'h264_qsv', available: !capsDegraded, detail: capsDegraded ? '未检测到 Intel QSV' : 'ok' },
      { name: 'h264_amf', available: false, detail: '未检测到 AMD 设备' },
      { name: 'h264_mf', available: true, detail: 'ok' },
    ],
    selected_encoder: capsDegraded ? 'h264_mf' : 'h264_nvenc',
    loopback_ok: !capsDegraded,
    mic_ok: !capsDegraded,
    wgc_ok: true,
    canvas_limits: { max_w: 7680, max_h: 4320 },
    layers: capsDegraded ? {
      loopback: { status: 'unavailable', detail: 'process loopback 试激活失败' },
      mic: { status: 'denied', detail: '麦克风隐私权限未允许' },
      wgc: { status: 'ok', detail: 'WGC 试激活成功' },
    } : null,
  };

  // M16/REQ-10：mock 显示器元数据（device_path 为稳定键；运行器可切换无元数据/换序场景）。
  const DEL_DP = '\\\\?\\DISPLAY#DELA1B2#5&abc&0&UID4357#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}';
  const HKC_DP = '\\\\?\\DISPLAY#HKC1234#5&def&0&UID4352#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}';
  const DEL_CID = '11111111-2222-3333-4444-555555555555';
  const HKC_CID = 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee';
  const monitorsAll = params.get('monitors_nometa') === '1'
    ? [
        { index: 0, name: 'Monitor 0', friendly_name: null, gdi_device_name: '\\\\.\\DISPLAY1', device_path: null, container_id: null, width: 3840, height: 2160, x: 0, y: 0, primary: true },
        { index: 1, name: 'Monitor 1', friendly_name: null, gdi_device_name: '\\\\.\\DISPLAY2', device_path: null, container_id: null, width: 2560, height: 1440, x: 3840, y: 120, primary: false },
      ]
    : [
        { index: 0, name: 'DELL U2720Q', friendly_name: 'DELL U2720Q', gdi_device_name: '\\\\.\\DISPLAY1', device_path: DEL_DP, container_id: DEL_CID, width: 3840, height: 2160, x: 0, y: 0, primary: true },
        { index: 1, name: 'HKC MG27Q', friendly_name: 'HKC MG27Q', gdi_device_name: '\\\\.\\DISPLAY2', device_path: HKC_DP, container_id: HKC_CID, width: 2560, height: 1440, x: 3840, y: 120, primary: false },
      ];
  let monitors = monitorsAll.map((m) => ({ ...m }));
  let targetCalls = 0;

  // M16/REQ-9：mock 渲染端点（默认两屏各自命中 ContainerId；可切换空/无元数据）。
  const endpointScenario = params.get('endpoints') || '';
  const renderEndpointsAll = endpointScenario === 'none' ? [] : [
    { device_id: 'ep-dell', friendly_name: 'DELL U2720Q (NVIDIA High Definition Audio)', mix_rate: 48000, is_active: true, container_id: DEL_CID },
    { device_id: 'ep-hkc', friendly_name: 'HKC MG27Q (NVIDIA High Definition Audio)', mix_rate: 48000, is_active: true, container_id: HKC_CID },
    { device_id: 'ep-usb', friendly_name: 'USB Audio Device', mix_rate: 44100, is_active: true, container_id: null },
  ];
  // REQ-10⑤ 回归夹具：候选 a/b（同 ContainerId、同分）+ 枚举首项无关端点。
  const ambiguousEndpoints = [
    { device_id: 'ep-unrelated', friendly_name: 'USB Audio Device', mix_rate: 44100, is_active: true, container_id: null },
    { device_id: 'ep-candidate-a', friendly_name: 'DELL U2720Q (候选 A)', mix_rate: 48000, is_active: true, container_id: DEL_CID },
    { device_id: 'ep-candidate-b', friendly_name: 'DELL U2720Q (候选 B)', mix_rate: 48000, is_active: true, container_id: DEL_CID },
  ];
  const renderEndpoints = endpointScenario === 'nocontainer'
    ? renderEndpointsAll.map((e) => ({ ...e, container_id: null, friendly_name: e.friendly_name.replace(/^.*? \(/, 'Audio ') }))
    : endpointScenario === 'ambiguous'
      ? ambiguousEndpoints
      : renderEndpointsAll;

  const windowsAll = [
    { hwnd: 0x30A12, title: '哔哩哔哩 (゜-゜)つロ 干杯~ - Google Chrome', app_name: 'Google Chrome', process_name: 'chrome.exe', pid: 15240, created_at_unix_ms: now - 3600_000 },
    { hwnd: 0x21B07, title: 'PotPlayer - 纪录片.行星.E01.mkv', app_name: 'PotPlayer', process_name: 'PotPlayerMini64.exe', pid: 8872, created_at_unix_ms: now - 2400_000 },
    { hwnd: 0x1F9C0, title: '网易云音乐', app_name: '网易云音乐', process_name: 'cloudmusic.exe', pid: 12304, created_at_unix_ms: now - 5400_000 },
    { hwnd: 0x41022, title: 'ui-spec.md - Screen_rec - Visual Studio Code', app_name: 'Visual Studio Code', process_name: 'Code.exe', pid: 20116, created_at_unix_ms: now - 1800_000 },
    { hwnd: 0x2C411, title: '微信', app_name: '微信', process_name: 'Weixin.exe', pid: 6640, created_at_unix_ms: now - 7200_000 },
    { hwnd: 0x18D3A, title: '文件资源管理器', app_name: '文件资源管理器', process_name: 'explorer.exe', pid: 4512, created_at_unix_ms: now - 9600_000 },
  ];
  const windows = scenario === 'empty' ? [] : windowsAll;

  const audioSourcesAll = [
    { pid: 15240, created_at_unix_ms: now - 3600_000, process_name: 'chrome.exe', app_name: 'Google Chrome', active: true },
    { pid: 8872, created_at_unix_ms: now - 2400_000, process_name: 'PotPlayerMini64.exe', app_name: 'PotPlayer', active: true },
    { pid: 12304, created_at_unix_ms: now - 5400_000, process_name: 'cloudmusic.exe', app_name: '网易云音乐', active: true },
    { pid: 11220, created_at_unix_ms: now - 1000_000, process_name: 'msedge.exe', app_name: 'Microsoft Edge', active: false },
  ];
  const audioSources = scenario === 'empty' ? [] : audioSourcesAll;

  const audioDevices = capsDegraded ? [] : [
    { id: 'mic-realtek', name: '麦克风阵列 (Realtek High Definition Audio)', kind: 'mic', is_default: true },
    { id: 'mic-usb', name: 'USB Audio Device', kind: 'mic', is_default: false },
  ];

  // -- 设置持久化（localStorage） -------------------------------------------
  const SETTINGS_KEY = 'screenrec.mock.settings';
  const defaultSettings = () => ({
    output_dir: 'C:\\Users\\demo\\Videos\\Screen_rec',
    fps: 30,
    audio_sample_rate: 48_000,
    output_container: 'mkv',
    cursor: true,
    default_audio_mode: 'isolated',
    mic_device: capsDegraded ? null : 'mic-realtek',
    encoder: 'auto',
    segment_secs: 5,
    keep_segments: false,
  });
  let settings;
  try {
    settings = { ...defaultSettings(), ...JSON.parse(localStorage.getItem(SETTINGS_KEY) || '{}') };
  } catch { settings = defaultSettings(); }

  // M17/REQ-16：成品容器（录制内部始终 MKV 分段；正常/提前终态/恢复三路径按此返回）。
  const containerExt = () => (settings.output_container === 'mp4' ? 'mp4' : 'mkv');

  // -- 命名（REQ-24 形状） ---------------------------------------------------
  const sanitize = (s) => s.replace(/[\\/:*?"<>|]/g, '_').slice(0, 60);
  const stamp = () => {
    const d = new Date();
    const p = (n, w = 2) => String(n).padStart(w, '0');
    return `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}_${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`;
  };

  // -- 录制会话模拟 -----------------------------------------------------------
  let routeSeq = 0;
  const routes = new Map(); // id -> { levelTimer, tracks, label, outDir, kind }
  const recentFiles = [
    { path: 'C:\\Users\\demo\\Videos\\Screen_rec\\20261016_214408_PotPlayer.mkv', kind: 'video', when: now - 86400_000 },
    { path: 'C:\\Users\\demo\\Videos\\Screen_rec\\20261016_215702_Chrome.mkv', kind: 'video', when: now - 82800_000 },
    { path: 'C:\\Users\\demo\\Videos\\Screen_rec\\20261016_220110_系统音频.wav', kind: 'audio', when: now - 79200_000 },
  ];

  function smoothLevel(track) {
    track._v = track._v ?? 0.15;
    const target = track.silent ? 0.02 : 0.25 + Math.random() * 0.55;
    track._v += (target - track._v) * 0.25 + (Math.random() - 0.5) * 0.08;
    return Math.max(0, Math.min(1, track._v));
  }

  function startRouteSim(routeId, meta) {
    const trackStates = meta.trackLabels.map(() => ({ silent: Math.random() < 0.15 }));
    const entry = { ...meta, trackStates, levelTimer: null, finalized: false };
    routes.set(routeId, entry);

    // 事件延后一拍：模拟真实后端「先回包、后推状态」的时序，避免同步事件丢失。
    setTimeout(() => emit({ event: 'route_state', id: routeId, state: 'recording' }), 30);
    entry.levelTimer = setInterval(() => {
      trackStates.forEach((t, i) => {
        emit({ event: 'route_level', id: routeId, track: i, rms: smoothLevel(t) });
      });
    }, 100); // ≤10Hz（REQ-31）

    if (scenario === 'error' && routeSeq === 1) {
      // 演示单路故障隔离：6s 后目标丢失 → 该路优雅停止并收尾，其它路不受影响。
      setTimeout(() => {
        if (!routes.has(routeId) || entry.finalized) return;
        emit({ event: 'route_error', id: routeId, code: 'E_TARGET_LOST', message: '目标窗口已关闭，该路已停止并保留已录制内容', severity: 'error' });
        finalizeRoute(routeId);
      }, 6000);
    }
  }

  function finalizeRoute(routeId) {
    const entry = routes.get(routeId);
    if (!entry || entry.finalized) return;
    entry.finalized = true;
    clearInterval(entry.levelTimer);
    emit({ event: 'route_state', id: routeId, state: 'stopping' });
    // 停止采集即时响应 + 后台收尾（T5）；后端状态机 recording→stopping→finalized，
    // 无 finalizing 进度事件（M9-B §3.3），mock 不伪造。
    setTimeout(() => {
      const base = `${entry.outDir}\\${stamp()}_${sanitize(entry.label)}`;
      const paths = entry.kind === 'audio'
        ? entry.trackLabels.map((l) => `${base}${entry.trackLabels.length > 1 ? `_${sanitize(l)}` : ''}.wav`)
        : [`${base}.${entry.container || 'mkv'}`];
      paths.forEach((path, i) => recentFiles.unshift({ path, kind: entry.kind === 'audio' ? 'audio' : 'video', when: Date.now() + i }));
      emit({ event: 'route_state', id: routeId, state: 'finalized' });
      emit({ event: 'route_finalized', id: routeId, path: paths[0], paths });
      routes.delete(routeId);
    }, 2500);
  }

  // -- 恢复会话模拟（SessionOut 形状，M9-B §3.2） ------------------------------
  const RECOVERY_KEY = 'screenrec.mock.recoveryHandled';
  const recoverySessionId = `${now - 69000_000}_0`;
  const recoveryRoutes = [
    { route_id: 'route-0', name: 'PotPlayer', streams: ['v', 'a0'], state: 'recording', segments: 47, final_path: null },
    { route_id: 'route-1', name: 'Google Chrome', streams: ['v', 'a0'], state: 'recording', segments: 47, final_path: null },
  ];
  const buildRecoverySession = () => {
    const dir = `${settings.output_dir}\\.screenrec_tmp\\${recoverySessionId}`;
    return {
      session_id: recoverySessionId,
      dir,
      created_at: now - 69000_000,
      active: false,
      manifest_ok: true,
      routes: recoveryRoutes,
      sealed_segments: [0, 1].flatMap((r) =>
        [0, 1, 2].map((i) => `${dir}\\out${r}_0000${i}.mkv`)),
      tail_segment: `${dir}\\out0_00046.mkv`,
    };
  };

  return {
    async probeCapabilities() { await wait(250); return structuredClone(caps); },
    async listTargets() {
      await wait(600);
      targetCalls++;
      if (params.get('monitors_swap_on_refresh') === '1' && targetCalls > 1) {
        // index 交换：device_path/friendly_name 跟随物理屏，index 变化。
        monitors = monitorsAll
          .map((m) => ({ ...m, index: m.index === 0 ? 1 : m.index === 1 ? 0 : m.index }))
          .sort((a, b) => a.index - b.index);
      } else {
        monitors = monitorsAll.map((m) => ({ ...m }));
      }
      return structuredClone({ monitors, windows });
    },
    async listRenderEndpoints() { await wait(150); return structuredClone(renderEndpoints); },
    async listAudioSources() { await wait(400); return structuredClone(audioSources); },
    async listAudioDevices() { await wait(200); return structuredClone(audioDevices); },

    // M15/REQ-2：mock 与 real 同形返回 {rate, kind, device_name}；null = 未知。
    async getEndpointMixRate(deviceId) {
      await wait(80);
      if (nativeFail) throw 'E_ENDPOINT_QUERY：端点混音率查询失败';
      if (nativeUnknown) return null;
      const kind = deviceId ? 'capture' : 'render';
      const rate = deviceId ? micRate : nativeRate;
      const name = deviceId
        ? (audioDevices.find((d) => d.id === deviceId)?.name || deviceId)
        : '默认渲染端点（mock）';
      return { rate, kind, device_name: name };
    },

    async startRoutes(routeCfgs) {
      await wait(800);
      if (!caps.encoders.some((e) => e.available)) {
        throw 'E_ENCODER_UNAVAILABLE：无可用编码器（已探测 h264_nvenc / h264_qsv / h264_amf / h264_mf）';
      }
      const route_ids = routeCfgs.map((cfg) => {
        const id = `route-${++routeSeq}`;
        const label = cfg._ui_label || '未命名目标';
        startRouteSim(id, {
          kind: 'video',
          label,
          outDir: cfg.output_dir,
          container: containerExt(), // M17/REQ-16：启动时冻结成品容器
          trackLabels: cfg._ui_track_labels?.length ? cfg._ui_track_labels : ['音轨 1'],
        });
        return id;
      });
      if (scenario === 'early_terminal' && route_ids.length) {
        // M16/REQ-12 UI 场景：终态早于 start_routes 回包 → app.js 必须先缓存再应用。
        const id = route_ids[0];
        const entry = routes.get(id);
        const path = entry
          ? `${entry.outDir}\\${stamp()}_${sanitize(entry.label)}.${entry.container || containerExt()}`
          : `${settings.output_dir}\\${stamp()}_early.${containerExt()}`;
        if (entry) {
          entry.finalized = true;
          clearInterval(entry.levelTimer);
          routes.delete(id);
        }
        emit({ event: 'route_finalized', id, path, paths: [path] });
      }
      return { route_ids };
    },

    async stopRoute(routeId) {
      await wait(120);
      // 纯录音簇语义：任一 id 停止即整组停止（M9-B §4）
      const entry = routes.get(routeId);
      const ids = entry?.cluster ?? [routeId];
      ids.forEach((id) => finalizeRoute(id));
      return { route_id: routeId, requested: true };
    },

    async stopAll() {
      await wait(150);
      const ids = [...routes.keys()];
      ids.forEach((id) => finalizeRoute(id));
      return { requested: ids };
    },

    async startAudioOnly(configs) {
      await wait(600);
      // 后端契约：单批次 {sources, outputDir?} → N 源 = N 个共享 handle 的 route id
      const cfg = configs[0];
      const labels = cfg._ui_track_labels?.length
        ? cfg._ui_track_labels
        : cfg.sources.map((_, i) => `来源 ${i + 1}`);
      const cluster = [];
      const route_ids = labels.map((label) => {
        const id = `route-${++routeSeq}`;
        cluster.push(id);
        startRouteSim(id, {
          kind: 'audio',
          label: `录音 · ${label}`,
          outDir: cfg.output_dir,
          trackLabels: [label],
          cluster, // 共享引用：map 完成后整簇可见
        });
        return id;
      });
      return { route_ids };
    },

    async takeScreenshot(req) {
      await wait(500);
      const name = req.kind === 'monitor' ? `显示器${req.index + 1}`
        : req.kind === 'window' ? '窗口'
        : '区域';
      const path = `${settings.output_dir}\\${stamp()}_${name}截图.png`;
      recentFiles.unshift({ path, kind: 'shot', when: Date.now() });
      return { path };
    },

    async mergeRecordings(paths, out) {
      await wait(300);
      let p = 0;
      await new Promise((resolve) => {
        const t = setInterval(() => {
          p += 0.12 + Math.random() * 0.15;
          if (p >= 1) { clearInterval(t); resolve(); }
          else emit({ event: 'merge_progress', progress: Math.min(0.99, p) });
        }, 220);
      });
      emit({ event: 'merge_progress', progress: 1 });
      recentFiles.unshift({ path: out, kind: 'video', when: Date.now() });
      return { out };
    },

    async recoverSessions() {
      await wait(300);
      if (scenario === 'norecovery') return { sessions: [] };
      let handled = [];
      try { handled = JSON.parse(localStorage.getItem(RECOVERY_KEY) || '[]'); } catch { /* ignore */ }
      const sessions = handled.includes(recoverySessionId) ? [] : [buildRecoverySession()];
      // 注：recovery_found 事件保留给真实后端的「扫描到即推送」场景；
      // mock 不在此处 emit，避免与 UI 的轮询式 loadRecovery 形成事件↔请求回路。
      return { sessions };
    },

    async finalizeSession(sessionId) {
      await wait(1600); // 收尾：concat + 校验，后台执行（REQ-21 D10）
      markRecoveryHandled(sessionId);
      return {
        finals: recoveryRoutes.map((r) => ({
          route_id: r.route_id,
          path: `${settings.output_dir}\\${stamp()}_${sanitize(r.name)}.${containerExt()}`,
        })),
        repaired_wavs: [],
      };
    },

    async discardSession(sessionId) {
      await wait(300);
      markRecoveryHandled(sessionId);
      return { ok: true };
    },

    async getSettings() { await wait(120); return structuredClone(settings); },

    async setSettings(next) {
      await wait(200);
      if (!next.output_dir || !next.output_dir.trim()) throw 'output_dir 不得为空';
      if (!next.fps) throw 'fps 不得为 0';
      if (!next.segment_secs) throw 'segment_secs 不得为 0';
      if (![44_100, 48_000, 88_200, 96_000].includes(next.audio_sample_rate)) {
        next.audio_sample_rate = 48_000; // REQ-1：与后端同归一化口径
      }
      if (!['mkv', 'mp4'].includes(next.output_container)) {
        next.output_container = 'mkv'; // REQ-16：与后端字段级容错同口径
      }
      settings = structuredClone(next);
      localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
      return null;
    },

    // mock 专属：合并页的候选文件列表（真实接线后由「最近录制」或文件对话框替代）
    async recentRecordings() { await wait(150); return structuredClone(recentFiles); },

    // M11-B 区域覆盖层的浏览器降级演示：新开一个 region.html 窗口模拟覆盖层。
    async getOverlayGeometry() {
      return {
        origin_x: 0, origin_y: 0,
        width: window.innerWidth, height: window.innerHeight,
        scale: window.devicePixelRatio || 1,
      };
    },
    async openRegionOverlay(mode = 'screenshot', requestId = null) {
      await wait(80);
      // mock：record 模式经 URL 参数携带上下文；浏览器窗口直接调试 region.html。
      const q = new URLSearchParams({ mode, request_id: requestId || '' });
      window.open(`region.html?${q}`, 'region-overlay', 'popup');
      return null;
    },
    async closeRegionOverlay() { return null; },
    async getRegionContext() {
      const q = new URLSearchParams(location.search);
      return { mode: q.get('mode') || 'screenshot', request_id: q.get('request_id') || '' };
    },
    async confirmRegionRecord(requestId, rect) {
      await wait(120);
      window.SRRegionMock = { requestId, rect };
      emit({
        event: 'region_target_added',
        request_id: requestId,
        rect,
        display: { device_name: '\\\\.\\DISPLAY1', device_path: DEL_DP },
      });
      return window.SRRegionMock;
    },
    async cancelRegionRecord(requestId) {
      emit({ event: 'region_record_cancelled', request_id: requestId });
      return null;
    },
    async listenRegionTarget(handler) {
      const fn = (evt) => { if (evt && evt.event === 'region_target_added') handler(evt); };
      listeners.add(fn);
      return () => listeners.delete(fn);
    },
    async listenRegionRecordCancelled(handler) {
      const fn = (evt) => { if (evt && evt.event === 'region_record_cancelled') handler(evt); };
      listeners.add(fn);
      return () => listeners.delete(fn);
    },
  };

  function markRecoveryHandled(id) {
    let handled = [];
    try { handled = JSON.parse(localStorage.getItem(RECOVERY_KEY) || '[]'); } catch { /* ignore */ }
    handled.push(id);
    localStorage.setItem(RECOVERY_KEY, JSON.stringify(handled));
  }
}
})();