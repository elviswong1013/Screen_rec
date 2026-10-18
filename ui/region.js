// ============================================================================
// ui/region.js — 区域截图覆盖层交互（REQ-8，运行于 label=region-overlay 窗口）
//
// 流程：
//   1) 启动时取 get_overlay_geometry()（虚拟桌面物理边界 + 窗口缩放）；
//   2) 左键拖拽绘制选区，尺寸提示实时显示物理像素（client × scale）；
//   3) 鼠标松开 / Enter 确认：client 矩形换算为物理矩形后调用
//      take_region_screenshot({rect:{x,y,w,h}})（裁剪参数以 commands.rs RectIn 为准）。
//      后端按序：隐藏覆盖层 → DwmFlush + ≥150ms 等待合成 → sr-shot 物理像素截图
//      → 成功则关闭覆盖层。失败时后端恢复覆盖层可见，本页 catch 里 showError 展示
//      错误（可重新拖拽重试或 Esc 取消），不得先截图后关窗（否则成品会含遮罩/边框像素）；
//      故本流程不再单独调用 take_screenshot / close_region_overlay；
//   4) Esc 取消：直接 close_region_overlay() 关闭本窗口，不发截图请求（保持不变）。
//
// 浏览器降级（直接双击 region.html 调试）：无 __TAURI__ 时用视口几何做 mock，
// take_region_screenshot/close 退化为 console 输出 / window.close()。
// ============================================================================
(() => {
'use strict';

const G = window.RegionGeometry;
const isTauri = typeof window !== 'undefined' && !!window.__TAURI__;
const invoke = isTauri ? window.__TAURI__.core.invoke : null;

const overlay = document.getElementById('overlay');
const hint = document.getElementById('hint');

const HINT_DEFAULT = '按住左键拖出截图区域 · 松开或 Enter 确认 · Esc 取消';
const MIN_CLIENT_PX = 10; // 与原 mock 行为一致：小于该尺寸的选区视为误触

// -- 后端调用封装（与 ui/api.js real 分支同参数约定） -------------------------
const backend = {
  getGeometry: () => invoke('get_overlay_geometry'),
  // P1-A：后端主导截图（隐藏覆盖层 → 等合成 → 截图 → 关闭覆盖层）。
  takeRegionScreenshot: (rect) => invoke('take_region_screenshot', { rect }),
  close: () => invoke('close_region_overlay'),
};
if (!isTauri) {
  backend.getGeometry = () => Promise.resolve({
    origin_x: 0, origin_y: 0,
    width: window.innerWidth, height: window.innerHeight,
    scale: window.devicePixelRatio || 1,
  });
  backend.takeRegionScreenshot = (rect) => {
    console.log('[region mock] take_region_screenshot rect =', rect);
    window.close(); // 模拟真实后端「截图后关闭覆盖层」
    return Promise.resolve({
      path: `mock-region-${rect.w}x${rect.h}.png`,
      width_px: rect.w,
      height_px: rect.h,
    });
  };
  backend.close = () => { window.close(); return Promise.resolve(); };
}

// -- 状态 ---------------------------------------------------------------------
let geom = null;        // get_overlay_geometry() 结果
let start = null;       // 拖拽起点 {x, y}（client CSS px）
let sel = null;         // 选区 DOM
let current = null;     // 最近一次绘制的 client 矩形（供 Enter 确认）
let busy = false;       // 截图请求进行中，屏蔽重复确认

backend.getGeometry()
  .then((g) => { geom = g; })
  .catch((e) => showError(`获取覆盖层几何失败：${e}`));

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
  backend.close().catch((e) => console.error('[region] close_region_overlay 失败:', e));
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
  hint.classList.remove('err');
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
  // 尺寸提示显示物理像素（与用户最终得到的 PNG 尺寸一致）
  const scale = geom ? geom.scale : 1;
  const pw = Math.round(rect.w * scale), ph = Math.round(rect.h * scale);
  sel.innerHTML = `<span class="region-size">${pw} × ${ph}</span>`;
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
})();
