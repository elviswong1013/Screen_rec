// ============================================================================
// tools/ui/region-record.mjs — M18/REQ-21 区域录制覆盖层行为 runner
//
// 方式：内置静态服务器 + headless Chrome（CDP；Node >= 22 全局 WebSocket/fetch）。
// 覆盖规格 §4 REQ-21：
//   - 几何纯函数：150% DPI 物理换算 + 向下取偶 + 单屏包含（生产 region-geometry.js）；
//   - region.html 真实脚本 + 假 Tauri 网关：record 确认载荷（request_id/rect）、
//     显示吸附后最终尺寸、record 绝不调用截图命令、Esc → cancel、迟到确认错误可见、
//     复用异模式（__regionContextChanged）以最新上下文为准；
//   - real/mock 接线与就绪顺序静态断言（主窗先 listen 再 open）。
//
// 运行：node tools/ui/region-record.mjs   （失败 exit 1）
// ============================================================================
'use strict';

import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { readFile, mkdtemp, rm, stat } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import vm from 'node:vm';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.png': 'image/png',
  '.json': 'application/json',
};

let failed = 0;
function check(name, ok, detail = '') {
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? ` — ${detail}` : ''}`);
  if (!ok) failed++;
}

console.log('[M18 UI runner] mock-only：真实 GUI 贯通由 tools/e2e/region-e2e.ps1 -Scenario record 承担。');

// ── 静态服务器 ──────────────────────────────────────────────────────────────
function startStaticServer() {
  const server = createServer(async (req, res) => {
    try {
      const url = new URL(req.url, 'http://127.0.0.1');
      let rel = decodeURIComponent(url.pathname);
      if (rel === '/') rel = '/ui/index.html';
      const file = path.resolve(ROOT, rel.replace(/^\/+/, ''));
      if (!file.startsWith(ROOT + path.sep)) {
        res.writeHead(403).end('forbidden');
        return;
      }
      const info = await stat(file);
      if (!info.isFile()) {
        res.writeHead(404).end('not found');
        return;
      }
      res.writeHead(200, { 'content-type': MIME[path.extname(file)] || 'application/octet-stream' });
      res.end(await readFile(file));
    } catch {
      res.writeHead(404).end('not found');
    }
  });
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => resolve({ server, port: server.address().port }));
  });
}

// ── Chrome / CDP（与既有 runner 同构） ──────────────────────────────────────
function findChrome() {
  const candidates = [
    process.env.CHROME_PATH,
    'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
    'C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe',
    'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
    'C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe',
  ].filter(Boolean);
  return candidates.find((p) => existsSync(p));
}

async function waitForDebugPort(port, timeoutMs = 15000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/json/version`);
      if (res.ok) return await res.json();
    } catch { /* not ready */ }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`Chrome 调试端口 ${port} 未就绪`);
}

class Cdp {
  constructor(wsUrl) {
    this.nextId = 1;
    this.pending = new Map();
    this.events = new Map();
    this.ws = new WebSocket(wsUrl);
    this.ready = new Promise((resolve, reject) => {
      this.ws.addEventListener('open', () => resolve());
      this.ws.addEventListener('error', (e) => reject(new Error(`CDP WebSocket 错误: ${e.message || e}`)));
    });
    this.ws.addEventListener('message', (ev) => {
      const msg = JSON.parse(ev.data);
      if (msg.id) {
        const p = this.pending.get(msg.id);
        if (!p) return;
        this.pending.delete(msg.id);
        if (msg.error) p.reject(new Error(`${msg.error.message} (${msg.error.code})`));
        else p.resolve(msg.result);
      } else if (msg.method) {
        for (const fn of this.events.get(msg.method) || []) fn(msg.params);
      }
    });
  }

  async send(method, params = {}) {
    await this.ready;
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }

  on(method, fn) {
    const arr = this.events.get(method) || [];
    arr.push(fn);
    this.events.set(method, arr);
  }

  async evaluate(expression) {
    const res = await this.send('Runtime.evaluate', {
      expression,
      awaitPromise: true,
      returnByValue: true,
    });
    if (res.exceptionDetails) {
      throw new Error(`页面 JS 异常: ${res.exceptionDetails.text || res.exceptionDetails.exception?.description}`);
    }
    return res.result?.value;
  }

  async navigate(url) {
    const loaded = new Promise((resolve) => {
      const fn = () => resolve();
      this.on('Page.loadEventFired', fn);
    });
    await this.send('Page.navigate', { url });
    await Promise.race([loaded, new Promise((r) => setTimeout(r, 5000))]);
    await waitForInPage(this, "document.readyState === 'complete'", 10000);
  }

  close() {
    try { this.ws.close(); } catch { /* ignore */ }
  }
}

async function waitForInPage(cdp, expression, timeoutMs = 8000, intervalMs = 80) {
  const deadline = Date.now() + timeoutMs;
  let last;
  while (Date.now() < deadline) {
    last = await cdp.evaluate(expression);
    if (last) return last;
    await new Promise((r) => setTimeout(r, intervalMs));
  }
  throw new Error(`等待页面条件超时: ${expression}`);
}

async function drag(cdp, x1, y1, x2, y2) {
  await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: x1, y: y1, button: 'left', clickCount: 1 });
  const steps = 6;
  for (let i = 1; i <= steps; i++) {
    await cdp.send('Input.dispatchMouseEvent', {
      type: 'mouseMoved',
      x: Math.round(x1 + ((x2 - x1) * i) / steps),
      y: Math.round(y1 + ((y2 - y1) * i) / steps),
      button: 'left',
      buttons: 1,
    });
    await new Promise((r) => setTimeout(r, 20));
  }
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: x2, y: y2, button: 'left', clickCount: 1 });
  await new Promise((r) => setTimeout(r, 120));
}

// ── 静态接线 / 就绪顺序 / 契约断言 ─────────────────────────────────────────
const apiSrc = await readFile(path.join(ROOT, 'ui', 'api.js'), 'utf8');
const appSrc = await readFile(path.join(ROOT, 'ui', 'app.js'), 'utf8');
const regionSrc = await readFile(path.join(ROOT, 'ui', 'region.js'), 'utf8');
const htmlSrc = await readFile(path.join(ROOT, 'ui', 'index.html'), 'utf8');
const cmdSrc = await readFile(path.join(ROOT, 'src-tauri', 'src', 'commands.rs'), 'utf8');
const regionRustSrc = await readFile(path.join(ROOT, 'src-tauri', 'src', 'region.rs'), 'utf8');
const libSrc = await readFile(path.join(ROOT, 'src-tauri', 'src', 'lib.rs'), 'utf8');
const capSrc = await readFile(path.join(ROOT, 'src-tauri', 'capabilities', 'default.json'), 'utf8');
const routeSrc = await readFile(path.join(ROOT, 'crates', 'sr-pipeline', 'src', 'route.rs'), 'utf8');
const endpointMatchSrc = await readFile(path.join(ROOT, 'ui', 'endpoint-match.js'), 'utf8');
const routeStateSrc = await readFile(path.join(ROOT, 'ui', 'route-state.js'), 'utf8');

check('api.js real open_region_overlay(mode, requestId)', /openRegionOverlay[\s\S]{0,120}invoke\('open_region_overlay',\s*\{\s*mode,\s*requestId\s*\}/.test(apiSrc));
check('api.js real get_region_context', apiSrc.includes("invoke('get_region_context')"));
check('api.js real confirm_region_record(requestId, rect)', /confirmRegionRecord[\s\S]{0,140}invoke\('confirm_region_record',\s*\{\s*requestId,\s*rect\s*\}/.test(apiSrc));
check('api.js real cancel_region_record', apiSrc.includes("invoke('cancel_region_record'"));
check('api.js real listen region_target_added', apiSrc.includes("listen('region_target_added'"));
check('api.js real listen region_record_cancelled', apiSrc.includes("listen('region_record_cancelled'"));
check('api.js mock 同形接线', apiSrc.includes('async confirmRegionRecord') && apiSrc.includes('async getRegionContext'));
check('index.html 提供区域录制入口', htmlSrc.includes('id="btn-region-record"'));
check('app.js 区域分支独立（buildRegionAudioPlan）', appSrc.includes('function buildRegionAudioPlan') && !/buildRegionAudioPlan[\s\S]{0,200}buildMonitorSources/.test(appSrc));
check('app.js 区域目标 kind=region + display_device', appSrc.includes("kind: 'region'") && appSrc.includes('display_device'));
check(
  'app.js 主窗先 listen 再 open（REQ-21 就绪顺序）',
  /ensureRegionTargetListener\(\)[\s\S]{0,220}openRegionOverlay\('record'/.test(appSrc),
  '按钮 handler 必须先 await 监听再打开 overlay'
);
check('region.js record 确认命令', regionSrc.includes('confirm_region_record') && regionSrc.includes('confirmRegionRecord'));
const recordCall = regionSrc.indexOf('await backend.confirmRegionRecord');
const screenshotCall = regionSrc.indexOf('await backend.takeRegionScreenshot');
check(
  'region.js record 绝不调用截图命令',
  recordCall >= 0 && screenshotCall > recordCall &&
    regionSrc.slice(recordCall, screenshotCall).includes('return;'),
  'record 确认分支必须在截图调用之前返回'
);
check(
  'commands.rs TargetIn::Region display_device + serde default',
  /Region\s*\{[\s\S]{0,160}#\[serde\(default\)\][\s\S]{0,100}display_device/.test(cmdSrc)
);
check('commands.rs MonitorOut.gdi_device_name', cmdSrc.includes('gdi_device_name'));
check(
  'sr-pipeline Region 目标含稳定设备键',
  /Region\s*\{\s*display_device:\s*String,\s*rect:\s*RegionRect,?\s*\}/.test(routeSrc)
);
check('lib.rs 注册 get/confirm/cancel region 命令', libSrc.includes('region::get_region_context') && libSrc.includes('region::confirm_region_record') && libSrc.includes('region::cancel_region_record'));
check('capabilities 授权 event listen/unlisten', capSrc.includes('core:event:allow-listen') && capSrc.includes('core:event:allow-unlisten'));
check(
  '最小区域尺寸 64（命令/管线）',
  /REGION_MIN_SIZE:\s*u32\s*=\s*64/.test(regionRustSrc) && routeSrc.includes('64x64')
);
check(
  'app.js 只接受当前 request_id 的 region_target_added',
  appSrc.includes('payload.request_id !== state.regionRequestId') && appSrc.includes('state.regionRequestId = null')
);
check('app.js 监听 region_record_cancelled 并清理', appSrc.includes('listenRegionRecordCancelled'));

// ── 生产 app.js：stale event 过滤（Node VM；mock-only） ────────────────────
function fakeRegionElement() {
  return {
    innerHTML: '', textContent: '', value: '', hidden: false, checked: false, disabled: false,
    style: {}, dataset: {}, firstChild: { textContent: '' },
    classList: { toggle() {}, add() {}, remove() {}, contains() { return false; } },
    addEventListener() {}, removeEventListener() {}, appendChild() {}, remove() {},
    querySelector() { return fakeRegionElement(); }, querySelectorAll() { return []; },
  };
}
function fakeRegionDocument() {
  return {
    querySelector() { return fakeRegionElement(); },
    querySelectorAll() { return []; },
    createElement() { return fakeRegionElement(); },
    addEventListener() {},
  };
}
{
  const captured = { target: null, cancelled: null };
  const sandbox = {
    console,
    setTimeout: () => 0,
    clearTimeout: () => {},
    setInterval: () => 0,
    clearInterval: () => {},
    location: { search: '' },
    document: fakeRegionDocument(),
    SR: {
      api: {
        listenRegionTarget: (fn) => { captured.target = fn; return Promise.resolve(() => {}); },
        listenRegionRecordCancelled: (fn) => { captured.cancelled = fn; return Promise.resolve(() => {}); },
      },
      isMock: true,
      subscribeEvents: () => () => {},
    },
  };
  sandbox.window = sandbox;
  sandbox.globalThis = sandbox;
  vm.createContext(sandbox);
  vm.runInContext(endpointMatchSrc, sandbox, { filename: 'ui/endpoint-match.js' });
  vm.runInContext(routeStateSrc, sandbox, { filename: 'ui/route-state.js' });
  const initCall = '\ninit();';
  const idx = appSrc.lastIndexOf(initCall);
  if (idx < 0) throw new Error('app.js 未找到 init() 引导调用');
  const instrumented = `${appSrc.slice(0, idx)}
window.__M18_REGION_APP__ = { state, ensureRegionTargetListener };
${appSrc.slice(idx + initCall.length)}`;
  vm.runInContext(instrumented, sandbox, { filename: 'ui/app.js' });
  const app = sandbox.__M18_REGION_APP__;
  await app.ensureRegionTargetListener();
  app.state.regionRequestId = 'req-new';
  const payloadOld = { request_id: 'req-old', rect: { x: 0, y: 0, w: 400, h: 300 }, display: { device_name: 'D1', device_path: 'P1' } };
  captured.target(payloadOld);
  check('生产 app.js：stale region_target_added 被忽略（不写 regionTarget/不清理当前 id）',
    app.state.regionTarget === null && app.state.regionRequestId === 'req-new');
  captured.target({ ...payloadOld, request_id: 'req-new' });
  check('生产 app.js：当前 request_id 被接受并清理 id',
    !!app.state.regionTarget && app.state.regionTarget.request_id === 'req-new' && app.state.regionRequestId === null);
  app.state.regionRequestId = 'req-cancel';
  captured.cancelled({ request_id: 'req-old' });
  check('生产 app.js：stale cancel 不清除当前目标', app.state.regionRequestId === 'req-cancel' && !!app.state.regionTarget);
  captured.cancelled({ request_id: 'req-cancel' });
  check('生产 app.js：当前 cancel 清理 request_id 与区域目标',
    app.state.regionRequestId === null && app.state.regionTarget === null);
}

// ── 几何纯函数（生产 region-geometry.js，Node VM） ─────────────────────────
const geomSrc = await readFile(path.join(ROOT, 'ui', 'region-geometry.js'), 'utf8');
const sandbox = { module: { exports: {} }, console };
sandbox.globalThis = sandbox;
vm.createContext(sandbox);
vm.runInContext(geomSrc, sandbox, { filename: 'ui/region-geometry.js' });
const G = sandbox.module.exports;

const geom = { origin_x: -100, origin_y: 50, width: 3840, height: 2160, scale: 1.5 };
const phys = G.clientRectToPhysical(geom, { x: 100, y: 50, w: 99, h: 99 });
check('150% DPI：client→物理（origin/scale）', phys.x === 50 && phys.y === 125 && phys.w === 149 && phys.h === 149, JSON.stringify(phys));
const snapped = G.snapPhysicalRect(phys);
check('输出尺寸向下取偶（149→148）', snapped.w === 148 && snapped.h === 148, JSON.stringify(snapped));
check('evenFloor 奇数/偶数/负数安全', G.evenFloor(401) === 400 && G.evenFloor(400) === 400 && G.evenFloor(-3) === 0);
check('单屏包含：屏内 true', G.rectInsideMonitor({ x: 0, y: 0, w: 1920, h: 1080 }, { x: 0, y: 0, width: 1920, height: 1080 }) === true);
check('单屏包含：跨屏 false', G.rectInsideMonitor({ x: 1800, y: 0, w: 200, h: 100 }, { x: 0, y: 0, width: 1920, height: 1080 }) === false);

// ── 真实 region.html + 假 Tauri 网关 ───────────────────────────────────────
const chrome = findChrome();
if (!chrome) {
  console.error('FAIL  未找到 Chrome/Edge（可用 CHROME_PATH 指定）');
  process.exit(1);
}

const { server, port: httpPort } = await startStaticServer();
const base = `http://127.0.0.1:${httpPort}/ui/region.html`;
const profile = await mkdtemp(path.join(tmpdir(), 'sr-m18-ui-'));
const debugPort = 9600 + Math.floor(Math.random() * 200);
const child = spawn(
  chrome,
  [
    '--headless=new',
    `--remote-debugging-port=${debugPort}`,
    `--user-data-dir=${profile}`,
    '--no-first-run',
    '--no-default-browser-check',
    '--disable-gpu',
    '--hide-scrollbars',
    '--window-size=1280,900',
    'about:blank',
  ],
  { stdio: 'ignore' },
);

const fakeTauri = `
(() => {
  window.__calls = [];
  window.__ctx = { mode: 'record', request_id: 'req-1' };
  window.__confirmError = null;
  const invoke = async (cmd, args) => {
    window.__calls.push({ cmd, args: args || null });
    if (cmd === 'get_overlay_geometry') {
      return { origin_x: -100, origin_y: 50, width: 3840, height: 2160, scale: 1.5 };
    }
    if (cmd === 'get_region_context') return { ...window.__ctx };
    if (cmd === 'confirm_region_record') {
      if (window.__confirmError) throw window.__confirmError;
      return { request_id: args.requestId, rect: args.rect };
    }
    if (cmd === 'cancel_region_record') return null;
    if (cmd === 'take_region_screenshot') return { path: 'x.png', width_px: args.rect.w, height_px: args.rect.h };
    if (cmd === 'close_region_overlay') return null;
    return null;
  };
  window.__TAURI__ = { core: { invoke }, event: { listen: async () => () => {} } };
})();
`;

let cdp;
try {
  await waitForDebugPort(debugPort);
  const tabRes = await fetch(`http://127.0.0.1:${debugPort}/json/new?${encodeURIComponent('about:blank')}`, { method: 'PUT' });
  const tab = await tabRes.json();
  cdp = new Cdp(tab.webSocketDebuggerUrl);
  await cdp.send('Runtime.enable');
  await cdp.send('Page.enable');
  await cdp.send('Page.addScriptToEvaluateOnNewDocument', { source: fakeTauri });

  // 1) record：初始化上下文 + 拖拽显示吸附后尺寸 + 确认载荷（raw rect, request_id）。
  await cdp.navigate(base);
  await waitForInPage(cdp, "document.getElementById('hint').textContent.includes('录制区域')", 5000);
  check('record：初始化拉取 get_region_context', await cdp.evaluate("window.__calls.some(c => c.cmd === 'get_region_context')"));
  check('record：提示文案为录制', (await cdp.evaluate("document.getElementById('hint').textContent")).includes('录制区域'));

  await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: 100, y: 100, button: 'left', clickCount: 1 });
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 199, y: 199, button: 'left', buttons: 1 });
  await new Promise((r) => setTimeout(r, 80));
  const sizeHint = await cdp.evaluate("document.querySelector('.region-size')?.textContent || ''");
  check('record：拖拽实时显示吸附后最终尺寸 148×148', sizeHint.replace(/\s/g, '') === '148×148', sizeHint);
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: 199, y: 199, button: 'left', clickCount: 1 });
  await new Promise((r) => setTimeout(r, 200));

  const calls = await cdp.evaluate('window.__calls');
  const confirm = calls.find((c) => c.cmd === 'confirm_region_record');
  check('record：调用 confirm_region_record（而非截图命令）', !!confirm && !calls.some((c) => c.cmd === 'take_region_screenshot'));
  check(
    'record：确认载荷 request_id + raw 物理 rect（149x149，由后端取偶）',
    !!confirm && confirm.args.requestId === 'req-1' && confirm.args.rect.w === 149 && confirm.args.rect.h === 149,
    JSON.stringify(confirm?.args),
  );

  // 2) 复用异模式：__regionContextChanged → 最新代次 screenshot；确认走截图命令。
  await cdp.evaluate("window.__ctx = { mode: 'screenshot', request_id: '' }; window.__regionContextChanged();");
  await waitForInPage(cdp, "document.getElementById('hint').textContent.includes('截图区域')", 5000);
  check('复用异模式：重新拉取上下文为 screenshot', (await cdp.evaluate("document.getElementById('hint').textContent")).includes('截图区域'));
  await cdp.evaluate('window.__calls.length = 0');
  await drag(cdp, 300, 300, 399, 399);
  const shotCalls = await cdp.evaluate('window.__calls');
  check(
    'screenshot：调用 take_region_screenshot（不含 record 确认）',
    shotCalls.some((c) => c.cmd === 'take_region_screenshot') && !shotCalls.some((c) => c.cmd === 'confirm_region_record'),
    JSON.stringify(shotCalls.map((c) => c.cmd)),
  );

  // 3) Esc → cancel_region_record(request_id)（迟到确认由后端代次拒绝）。
  await cdp.evaluate("window.__ctx = { mode: 'record', request_id: 'req-2' }; window.__regionContextChanged();");
  await waitForInPage(cdp, "document.getElementById('hint').textContent.includes('录制区域')", 5000);
  await cdp.evaluate('window.__calls.length = 0');
  await cdp.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 });
  await new Promise((r) => setTimeout(r, 150));
  const cancelCalls = await cdp.evaluate('window.__calls');
  check(
    'Esc：record 模式调用 cancel_region_record(req-2)',
    cancelCalls.some((c) => c.cmd === 'cancel_region_record' && c.args.requestId === 'req-2'),
    JSON.stringify(cancelCalls.map((c) => c.cmd)),
  );

  // 4) 迟到确认（E_INVALID_REQUEST）/越界（E_INVALID_REGION）→ 错误可见。
  for (const [code, label] of [
    ['E_INVALID_REQUEST: 区域录制确认已失效', '迟到确认'],
    ['E_INVALID_REGION: 区域必须完全位于单个显示器内', '跨屏/越界'],
  ]) {
    await cdp.evaluate(`window.__confirmError = ${JSON.stringify(code)}; window.__ctx = { mode: 'record', request_id: 'req-err' }; window.__regionContextChanged();`);
    await waitForInPage(cdp, "document.getElementById('hint').textContent.includes('录制区域')", 5000);
    await drag(cdp, 500, 500, 599, 599);
    const hint = await cdp.evaluate("(() => { const h = document.getElementById('hint'); return { text: h.textContent, err: h.classList.contains('err') }; })()");
    check(`${label}：错误提示可见（.err）`, hint.err && hint.text.includes('确认失败'), JSON.stringify(hint));
    await cdp.evaluate('window.__confirmError = null');
  }
} finally {
  if (cdp) cdp.close();
  try { child.kill(); } catch { /* ignore */ }
  server.close();
  await rm(profile, { recursive: true, force: true }).catch(() => {});
}

console.log(failed === 0 ? '\n全部通过' : `\n失败 ${failed} 项`);
process.exit(failed === 0 ? 0 : 1);
