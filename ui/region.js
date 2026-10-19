// ============================================================================
// ui/region.js — 区域覆盖层交互（截图 REQ-8；录制 REQ-21，运行于 label=region-overlay）
//
// 模式由后端上下文决定（get_region_context() -> {mode, request_id}）：
//   - screenshot（默认，旧行为）：拖拽 → 确认 → take_region_screenshot（后端主导
//     隐藏/等合成/截图/关闭）；Esc → close_region_overlay；
//   - record（M18）：拖拽 → 实时显示**吸附后最终尺寸**（向下取偶）→ 确认 →
//     confirm_region_record(request_id, rect)（后端代次复核、隐藏、交付
//     region_target_added、结束生命周期）；Esc → cancel_region_record(request_id)
//     使迟到确认失效。record 路径绝不调用截图命令。
//
// 复用窗口：后端 open_region_overlay 会调用 __regionContextChanged()，本页重新拉取
// 最新代次上下文（异模式以最新代次为准，旧代次结果由后端丢弃）。
//
// 浏览器降级（直接打开 region.html 调试）：无 __TAURI__ 时以 query/视口 mock。
// ============================================================================
(() => {
'use strict';

const G = window.RegionGeometry;
const isTauri = typeof window !== 'undefined' && !!window.__TAURI__;
const invoke = isTauri ? window.__TAURI__.core.invoke : null;

const overlay = document.getElementById('overlay');
const hint = document.getElementById('hint');

const HINT_SCREENSHOT = '按住左键拖出截图区域 · 松开或 Enter 确认 · Esc 取消';
const HINT_RECORD = '按住左键拖出录制区域 · 松开或 Enter 确认 · Esc 取消';
const MIN_CLIENT_PX = 10; // 小于该尺寸的选区视为误触

// -- 后端调用封装（与 ui/api.js real 分支同参数约定） -------------------------
const backend = {
  getGeometry: () => invoke('get_overlay_geometry'),
  getContext: () => invoke('get_region_context'),
  takeRegionScreenshot: (rect) => invoke('take_region_screenshot', { rect }),
  confirmRegionRecord: (requestId, rect) => invoke('confirm_region_record', { requestId, rect }),
  cancelRegionRecord: (requestId) => invoke('cancel_region_record', { requestId }),
  close: () => invoke('close_region_overlay'),
};
if (!isTauri) {
  const q = new URLSearchParams(location.search);
  backend.getGeometry = () => Promise.resolve({
    origin_x: 0, origin_y: 0,
    width: window.innerWidth, height: window.innerHeight,
    scale: window.devicePixelRatio || 1,
  });
  backend.getContext = () => Promise.resolve({
    mode: q.get('mode') || 'screenshot',
    request_id: q.get('request_id') || '',
  });
  backend.takeRegionScreenshot = (rect) => {
    window.SRRegionMock = { action: 'screenshot', rect };
    console.log('[region mock] take_region_screenshot rect =', rect);
    window.close();
    return Promise.resolve({
      path: `mock-region-${rect.w}x${rect.h}.png`,
      width_px: rect.w,
      height_px: rect.h,
    });
  };
  backend.confirmRegionRecord = (requestId, rect) => {
    window.SRRegionMock = { action: 'record', requestId, rect };
    console.log('[region mock] confirm_region_record', requestId, rect);
    window.close();
    return Promise.resolve({ request_id: requestId, rect });
  };
  backend.cancelRegionRecord = (requestId) => {
    window.SRRegionMock = { action: 'cancel', requestId };
    window.close();
    return Promise.resolve();
  };
  backend.close = () => { window.close(); return Promise.resolve(); };
}

// -- 状态 ---------------------------------------------------------------------
let geom = null;        // get_overlay_geometry() 结果
let ctx = { mode: 'screenshot', request_id: '' };
let start = null;       // 拖拽起点 {x, y}（client CSS px）
let sel = null;         // 选区 DOM
let current = null;     // 最近一次绘制的 client 矩形（供 Enter 确认）
let busy = false;       // 请求进行中，屏蔽重复确认

async function loadContext() {
  try {
    ctx = await backend.getContext();
  } catch (e) {
    ctx = { mode: 'screenshot', request_id: '' };
    console.warn('[region] get_region_context 失败（回退截图模式）:', e);
  }
  hint.classList.remove('err');
  hint.textContent = ctx.mode === 'record' ? HINT_RECORD : HINT_SCREENSHOT;
}

function init() {
  backend.getGeometry()
    .then((g) => { geom = g; })
    .catch((e) => showError(`获取覆盖层几何失败：${e}`));
  loadContext();
}

// 复用窗口：后端以 eval 触发；重新拉取最新代次并清空旧选区。
window.__regionContextChanged = () => {
  busy = false;
  resetSelection();
  loadContext();
};

function showError(msg) {
  hint.textContent = `${msg} · Esc 关闭`;
  hint.classList.add('err');
}

function resetSelection() {
  if (sel) { sel.remove(); sel = null; }
  start = null;
  current = null;
}

function cancel() {
  const call = ctx.mode === 'record' && ctx.request_id
    ? backend.cancelRegionRecord(ctx.request_id)
    : backend.close();
  call.catch((e) => console.error('[region] 取消区域覆盖层失败:', e));
}

function finalRect(phys) {
  // 录制：输出 = 向下取偶后的最终尺寸（与后端确认/启动口径一致）。
  return ctx.mode === 'record' ? G.snapPhysicalRect(phys) : phys;
}

async function confirm(rect) {
  if (busy) return;
  if (!geom) { showError('几何信息尚未就绪，请稍候再试'); return; }
  if (rect.w < MIN_CLIENT_PX || rect.h < MIN_CLIENT_PX) {
    resetSelection();
    hint.textContent = '选区过小，已放弃 · 重新拖拽或 Esc 取消';
    return;
  }
  busy = true;
  const phys = G.clientRectToPhysical(geom, rect);
  const final = finalRect(phys);
  hint.classList.remove('err');
  if (ctx.mode === 'record') {
    hint.textContent = `正在确认录制区域 ${final.w} × ${final.h}…`;
    try {
      await backend.confirmRegionRecord(ctx.request_id, phys);
      // 后端隐藏覆盖层 → 交付 → 关闭；成功路径无需再发 close。
    } catch (e) {
      busy = false;
      showError(`区域录制确认失败：${e}`);
    }
    return;
  }
  hint.textContent = `正在截图 ${phys.w} × ${phys.h}…`;
  try {
    // 后端在完成截图后关闭覆盖层（本窗口）；成功时无需再发 close。
    await backend.takeRegionScreenshot(phys);
  } catch (e) {
    busy = false;
    showError(`截图失败：${e}`);
  }
}

// -- 鼠标交互 -----------------------------------------------------------------
overlay.addEventListener('mousedown', (e) => {
  if (busy || e.button !== 0) return;
  resetSelection();
  start = { x: e.clientX, y: e.clientY };
  sel = document.createElement('div');
  sel.className = 'region-sel';
  overlay.appendChild(sel);
});

overlay.addEventListener('mousemove', (e) => {
  if (!start || !sel) return;
  const rect = G.normalizeClientRect(start.x, start.y, e.clientX, e.clientY);
  current = rect;
  Object.assign(sel.style, {
    left: `${rect.x}px`, top: `${rect.y}px`,
    width: `${rect.w}px`, height: `${rect.h}px`,
  });
  // 尺寸提示：录制显示吸附后最终物理尺寸（禁止确认后静默少像素）。
  const scale = geom ? geom.scale : 1;
  const phys = {
    x: Math.round((geom ? geom.origin_x : 0) + rect.x * scale),
    y: Math.round((geom ? geom.origin_y : 0) + rect.y * scale),
    w: Math.max(1, Math.round(rect.w * scale)),
    h: Math.max(1, Math.round(rect.h * scale)),
  };
  const shown = ctx.mode === 'record' ? G.snapPhysicalRect(phys) : phys;
  sel.innerHTML = `<span class="region-size">${shown.w} × ${shown.h}</span>`;
});

overlay.addEventListener('mouseup', (e) => {
  if (!start || e.button !== 0) return;
  const rect = G.normalizeClientRect(start.x, start.y, e.clientX, e.clientY);
  confirm(rect);
});

// -- 键盘交互 -----------------------------------------------------------------
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') { e.preventDefault(); cancel(); return; }
  if (e.key === 'Enter' && current && !busy) { e.preventDefault(); confirm(current); }
});

init();
})();
