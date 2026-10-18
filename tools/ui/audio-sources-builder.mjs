// ============================================================================
// tools/ui/audio-sources-builder.mjs — M16/REQ-10/12/13/14 音源构建行为 runner
//
// 方式：内置静态服务器 + headless Chrome（CDP；Node >= 22 全局 WebSocket/fetch），
// 驱动真实 ui/index.html + api.js + app.js 的 **mock** 分支：
//   - 隔离 + 显示器匹配 → 预选仅端点（start DTO 恰 1 源）；
//   - 端点 + 麦克风勾选 → 2 源（顺序 [endpoint, mic]）；
//   - 取消端点 → 系统预选；全部取消 → REQ-14 提示（不静默启动）；
//   - 模式切换勾选如实保留；
//   - 刷新不覆盖手选；index 交换不串屏（device_path 键）；
//   - 双屏无元数据：临时键不跨刷新复用 / 不合并；
//   - 终态早于启动回包（未知 route_id 缓存）；迟到旧快照不覆盖终态；
//   - 失败告警 + 成品路径并存。
// 另对 endpoint-match.js / route-state.js 的生产纯函数做确定性断言。
//
// 运行：node tools/ui/audio-sources-builder.mjs   （失败 exit 1）
// ============================================================================
'use strict';

import { createServer } from 'node:http';
import { spawn, spawnSync } from 'node:child_process';
import { readFile, mkdtemp, rm, stat } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
  '.json': 'application/json',
};

let failed = 0;
function check(name, ok, detail = '') {
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? ` — ${detail}` : ''}`);
  if (!ok) failed++;
}

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
      const body = await readFile(file);
      res.writeHead(200, { 'content-type': MIME[path.extname(file)] || 'application/octet-stream' });
      res.end(body);
    } catch {
      res.writeHead(404).end('not found');
    }
  });
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => resolve({ server, port: server.address().port }));
  });
}

// ── Chrome / CDP（与 M15 runner 同构） ─────────────────────────────────────
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

async function waitForText(cdp, selector, needle, timeoutMs = 8000) {
  const expr = `(() => { const el = document.querySelector(${JSON.stringify(selector)}); return el ? el.textContent : ''; })()`;
  const deadline = Date.now() + timeoutMs;
  let text = '';
  while (Date.now() < deadline) {
    text = await cdp.evaluate(expr);
    if (text && text.includes(needle)) return text;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`等待 ${selector} 含 ${JSON.stringify(needle)} 超时（实际 ${JSON.stringify(text)}）`);
}

async function waitForTargets(cdp) {
  await waitForInPage(
    cdp,
    "(() => { const b = document.querySelector('#left-panel-body'); return !!b && !!b.querySelector('.tgt-row') && !b.querySelector('.skel'); })()",
    15000
  );
}

async function selectMonitor(cdp, index) {
  const sel = `[data-key="monitor:${index}"]`;
  let exists = await cdp.evaluate(`!!document.querySelector(${JSON.stringify(sel)})`);
  if (!exists) {
    await cdp.evaluate(`document.querySelector('[data-tab="monitors"]').click()`);
    await waitForInPage(cdp, `!!document.querySelector(${JSON.stringify(sel)})`, 5000);
    exists = true;
  }
  const selected = await cdp.evaluate(`(() => { const el = document.querySelector(${JSON.stringify(sel)}); return !!el && el.classList.contains('selected'); })()`);
  if (!selected) {
    await cdp.evaluate(`document.querySelector(${JSON.stringify(sel)}).click()`);
  }
  await waitForInPage(cdp, `!!document.querySelector('.monitor-audio[data-monitor-index="${index}"]')`, 5000);
}

async function installStartCapture(cdp) {
  await cdp.evaluate(`(() => {
    window.__starts = [];
    window.SR.api.startRoutes = async (routes) => {
      window.__starts.push(routes);
      return { route_ids: routes.map((_, i) => 'test-route-' + (window.__starts.length * 100 + i)) };
    };
  })()`);
}

async function startAndCapture(cdp) {
  const before = await cdp.evaluate('window.__starts.length');
  await cdp.evaluate(`document.querySelector('#btn-start').click()`);
  await waitForInPage(cdp, `window.__starts.length > ${before}`, 5000);
  return cdp.evaluate('window.__starts[window.__starts.length - 1]');
}

function sourceKinds(routes) {
  return (routes || []).flatMap((r) => (r.audio.sources || []).map((s) => s.kind));
}

async function checkBox(cdp, id) {
  return cdp.evaluate(`(() => { const el = document.querySelector('#${id}'); return el ? el.checked : null; })()`);
}

// ── 纯函数（生产 ui/*.js，Node 侧同一实现） ───────────────────────────────

// ── 主流程 ──────────────────────────────────────────────────────────────────
const chrome = findChrome();
if (!chrome) {
  console.error('FAIL  未找到 Chrome/Edge（可用 CHROME_PATH 指定）');
  process.exit(1);
}

// 静态接线断言（real/mock、命令注册、脚本加载）
const apiSrc = await readFile(path.join(ROOT, 'ui', 'api.js'), 'utf8');
const appSrc = await readFile(path.join(ROOT, 'ui', 'app.js'), 'utf8');
const htmlSrc = await readFile(path.join(ROOT, 'ui', 'index.html'), 'utf8');
const cmdSrc = await readFile(path.join(ROOT, 'src-tauri', 'src', 'commands.rs'), 'utf8');
const libSrc = await readFile(path.join(ROOT, 'src-tauri', 'src', 'lib.rs'), 'utf8');
check('api.js real 接线 list_render_endpoints', apiSrc.includes("invoke('list_render_endpoints'"), 'real 分支');
check('api.js mock 接线 listRenderEndpoints', apiSrc.includes('async listRenderEndpoints'), 'mock 分支');
check('app.js 调用 listRenderEndpoints', appSrc.includes('listRenderEndpoints'), 'UI 拉取端点');
check('app.js 使用生产匹配/调和函数', appSrc.includes('matchEndpoint') && appSrc.includes('reconcileMonitorSelections'), 'endpoint-match.js');
check('index.html 加载 endpoint-match.js', htmlSrc.includes('endpoint-match.js'));
check('index.html 加载 route-state.js', htmlSrc.includes('route-state.js'));
check('命令注册 list_render_endpoints', libSrc.includes('commands::list_render_endpoints'));
check('AudioSourceIn render_endpoint 形状', cmdSrc.includes('RenderEndpoint { device_id: String }'));
check('start DTO kind=render_endpoint', appSrc.includes("kind: 'render_endpoint'"), 'UI 发送契约');

// 纯函数确定性断言（Node 侧直接执行生产脚本）
const endpointMatchSrc = await readFile(path.join(ROOT, 'ui', 'endpoint-match.js'), 'utf8');
const routeStateSrc = await readFile(path.join(ROOT, 'ui', 'route-state.js'), 'utf8');
function loadScript(code) {
  const sandbox = { console, Set, Map, Math, Date, JSON };
  sandbox.window = sandbox;
  sandbox.globalThis = sandbox;
  vm.createContext(sandbox);
  vm.runInContext(code, sandbox, { filename: 'ui-script.js' });
  return sandbox;
}
const em = loadScript(endpointMatchSrc).SREndpointMatch;
const rs = loadScript(routeStateSrc).SRRouteState;

// Node VM 加载生产 app.js（仅替换末尾 init() 引导为回归钩子；不改产品代码），
// 确定性覆盖注册/缓存路径的终态单调护栏（REQ-12）。
function fakeElement() {
  return {
    innerHTML: '', textContent: '', value: '', hidden: false, checked: false, disabled: false,
    style: {}, dataset: {}, firstChild: { textContent: '' },
    classList: { toggle() {}, add() {}, remove() {}, contains() { return false; } },
    addEventListener() {}, removeEventListener() {}, appendChild() {}, remove() {},
    querySelector() { return fakeElement(); }, querySelectorAll() { return []; },
  };
}

function fakeDocument() {
  return {
    querySelector() { return fakeElement(); },
    querySelectorAll() { return []; },
    createElement() { return fakeElement(); },
    addEventListener() {},
  };
}

function loadAppRouteRegression(appCode, endpointCode, routeStateCode) {
  const handlers = [];
  const sandbox = {
    console,
    setTimeout: () => 0,
    clearTimeout: () => {},
    setInterval: () => 0,
    clearInterval: () => {},
    location: { search: '' },
    document: fakeDocument(),
    SR: {
      api: {},
      isMock: true,
      subscribeEvents: (fn) => { handlers.push(fn); return () => {}; },
    },
  };
  sandbox.window = sandbox;
  sandbox.globalThis = sandbox;
  vm.createContext(sandbox);
  vm.runInContext(endpointCode, sandbox, { filename: 'ui/endpoint-match.js' });
  vm.runInContext(routeStateCode, sandbox, { filename: 'ui/route-state.js' });
  const initCall = '\ninit();';
  const idx = appCode.lastIndexOf(initCall);
  if (idx < 0) throw new Error('app.js 未找到 init() 引导调用');
  const instrumented = `${appCode.slice(0, idx)}
window.__SR_APP_REGRESSION__ = { state, registerRoute, mergePending, pendingStates };
${appCode.slice(idx + initCall.length)}`;
  vm.runInContext(instrumented, sandbox, { filename: 'ui/app.js' });
  return { app: sandbox.__SR_APP_REGRESSION__, handlers };
}

const routeApp = loadAppRouteRegression(appSrc, endpointMatchSrc, routeStateSrc);
{
  const { state, registerRoute, mergePending } = routeApp.app;
  const emitApp = (evt) => routeApp.handlers.forEach((fn) => fn(evt));
  const meta = { kind: 'video', label: '回归目标', icon: '', trackLabels: ['音轨 1'] };

  // ① 已有终态（done + final_path + stop_notice）→ 启动回包再次注册不得降级。
  registerRoute('reg-terminal', meta);
  emitApp({
    event: 'route_state', id: 'reg-terminal', state: 'finalized',
    final_path: 'C:\\out\\reg-terminal.mkv',
    stop_notice: { code: 'E_TARGET_LOST', endpoint: 'ep-a', detail: '端点失效' },
  });
  const beforeReregister = state.routes.get('reg-terminal');
  registerRoute('reg-terminal', { ...meta, label: '回归目标（重注册）' });
  const afterReregister = state.routes.get('reg-terminal');
  check(
    '生产 app.js：终态后重注册不降级（done/final_path/stop_notice 保留）',
    beforeReregister.state === 'done'
      && afterReregister.state === 'done'
      && afterReregister.path === 'C:\\out\\reg-terminal.mkv'
      && afterReregister.stopNotice?.code === 'E_TARGET_LOST'
      && afterReregister.label === '回归目标（重注册）',
    JSON.stringify(afterReregister),
  );

  // ② 注册前收到 failed，随后旧 recording → 注册后不得回落 recording。
  mergePending('reg-pending', {
    state: 'failed',
    error: { code: 'E_TARGET_LOST', message: '目标已失效' },
    stop_notice: { code: 'E_TARGET_LOST', endpoint: 'ep-a' },
  });
  mergePending('reg-pending', { state: 'recording', final_path: null });
  registerRoute('reg-pending', meta);
  const afterPending = state.routes.get('reg-pending');
  check(
    '生产 app.js：注册前 failed + 旧 recording 不降级（error/stop_notice 保留）',
    afterPending.state === 'failed'
      && afterPending.error?.code === 'E_TARGET_LOST'
      && afterPending.stopNotice?.endpoint === 'ep-a',
    JSON.stringify(afterPending),
  );
}

{
  const DEL_CID = '11111111-2222-3333-4444-555555555555';
  const monitors = [
    { index: 0, friendly_name: 'DELL U2720Q', device_path: 'path-DEL', container_id: DEL_CID },
    { index: 1, friendly_name: 'HKC MG27Q', device_path: 'path-HKC', container_id: 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee' },
  ];
  const endpoints = [
    { device_id: 'ep-a', friendly_name: 'DELL U2720Q A', is_active: true, container_id: DEL_CID },
    { device_id: 'ep-b', friendly_name: 'DELL U2720Q B', is_active: true, container_id: DEL_CID },
    { device_id: 'ep-hkc', friendly_name: 'HKC MG27Q (NVIDIA High Definition Audio)', is_active: true, container_id: 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee' },
  ];
  const matched = em.matchEndpoint(monitors[0], endpoints);
  check('纯函数：同 ContainerId 同分 → ambiguous 不取首项', matched.status === 'ambiguous', JSON.stringify(matched));

  const sel = em.reconcileMonitorSelections(new Map(), monitors, endpoints);
  const dell = sel.get('path:path-DEL');
  const hkc = sel.get('path:path-HKC');
  check('纯函数：每显示器独立默认预选', dell.endpointSelected !== hkc.endpointSelected, JSON.stringify({ dell, hkc }));
  dell.micSelected = true;
  dell.endpointSelected = false;
  dell.systemSelected = true;
  // 刷新：device_path 键保留手选；index 交换不串屏。
  const swapped = [ { ...monitors[0], index: 1 }, { ...monitors[1], index: 0 } ];
  const sel2 = em.reconcileMonitorSelections(sel, swapped, endpoints);
  const dell2 = sel2.get('path:path-DEL');
  check('纯函数：刷新不覆盖手选（device_path 键）', dell2.micSelected === true && dell2.systemSelected === true);
  check('纯函数：index 交换后 state 仍跟随 device_path', dell2.device_path === 'path-DEL');

  const noMeta = [
    { index: 0, friendly_name: null, device_path: null, container_id: null },
    { index: 1, friendly_name: null, device_path: null, container_id: null },
  ];
  const temp1 = em.reconcileMonitorSelections(new Map(), noMeta, []);
  temp1.get('idx:0').micSelected = true;
  const temp2 = em.reconcileMonitorSelections(temp1, noMeta, []);
  check('纯函数：临时键不跨刷新复用', temp2.get('idx:0').micSelected === false);
  check('纯函数：双屏无元数据不合并', temp2.size === 2 && temp2.has('idx:0') && temp2.has('idx:1'));

  const r = { state: 'recording', startedAt: 1 };
  rs.applyRouteState(r, 'recording');
  rs.applyRouteState(r, 'finalized', 'C:\\out\\a.mkv');
  const afterLate = rs.applyRouteState(r, 'recording');
  check('纯函数：迟到旧快照不覆盖终态', afterLate === false && r.state === 'done' && r.path === 'C:\\out\\a.mkv');
  const pendingRoute = { state: 'starting' };
  rs.applyPending(pendingRoute, { state: 'failed', error: { code: 'E_TARGET_LOST', message: 'x' } });
  rs.applyPending(pendingRoute, { state: 'finalized', path: 'C:\\out\\b.mkv' });
  check('纯函数：终态早于注册回包（缓存+成品路径）', pendingRoute.state === 'done' && pendingRoute.error?.code === 'E_TARGET_LOST', JSON.stringify(pendingRoute));
}

const { server, port: httpPort } = await startStaticServer();
const base = `http://127.0.0.1:${httpPort}/ui/index.html`;
const profile = await mkdtemp(path.join(tmpdir(), 'sr-m16-ui-'));
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
  { stdio: 'ignore' }
);

let cdp;
try {
  await waitForDebugPort(debugPort);
  const tabRes = await fetch(`http://127.0.0.1:${debugPort}/json/new?${encodeURIComponent('about:blank')}`, { method: 'PUT' });
  const tab = await tabRes.json();
  cdp = new Cdp(tab.webSocketDebuggerUrl);
  await cdp.send('Runtime.enable');
  await cdp.send('Page.enable');

  // 1) 隔离 + 显示器匹配 → 预选仅端点；取消端点 → 系统预选；全取消 → REQ-14。
  {
    await cdp.navigate(`${base}?native_rate=48000`);
    await waitForTargets(cdp);
    await installStartCapture(cdp);
    await selectMonitor(cdp, 0);
    check('隔离预选端点勾选', (await checkBox(cdp, 'cfg-mon-0-endpoint')) === true);
    check('隔离预选系统未勾选', (await checkBox(cdp, 'cfg-mon-0-system')) === false);
    const status = await waitForText(cdp, '.monitor-audio [data-match-status]', '自动匹配');
    check('匹配状态可见', /ContainerId/.test(status), status.trim());

    const first = await startAndCapture(cdp);
    check('隔离 start DTO mode=all_tracks', first[0].audio.mode === 'all_tracks', JSON.stringify(first[0].audio));
    check('隔离预选恰 1 源=端点', sourceKinds(first).length === 1 && sourceKinds(first)[0] === 'render_endpoint', JSON.stringify(sourceKinds(first)));
    check('端点 device_id 来自匹配', first[0].audio.sources[0].device_id === 'ep-dell');

    await selectMonitor(cdp, 0); // start 成功会清空目标选择
    await cdp.evaluate(`document.querySelector('#cfg-mon-0-mic').click()`);
    const second = await startAndCapture(cdp);
    const kinds2 = sourceKinds(second);
    check('端点+麦克风 → 2 源（顺序 endpoint,mic）', JSON.stringify(kinds2) === JSON.stringify(['render_endpoint', 'mic']), JSON.stringify(kinds2));

    await selectMonitor(cdp, 0);
    // 重新勾选端点+麦克风（上一步 start 后仍保留勾选状态，但目标被清空）。
    await cdp.evaluate(`document.querySelector('#cfg-mon-0-endpoint').click()`); // 取消端点
    check('取消端点 → 系统预选', (await checkBox(cdp, 'cfg-mon-0-system')) === true);
    const third = await startAndCapture(cdp);
    const kinds3 = sourceKinds(third);
    check('取消端点后 sources 不含端点', !kinds3.includes('render_endpoint') && kinds3.includes('system'), JSON.stringify(kinds3));

    await selectMonitor(cdp, 0);
    // 全部取消：system/mic/endpoint 均未勾选。
    if (await checkBox(cdp, 'cfg-mon-0-system')) await cdp.evaluate(`document.querySelector('#cfg-mon-0-system').click()`);
    if (await checkBox(cdp, 'cfg-mon-0-mic')) await cdp.evaluate(`document.querySelector('#cfg-mon-0-mic').click()`);
    const beforeZero = await cdp.evaluate('window.__starts.length');
    await cdp.evaluate(`document.querySelector('#btn-start').click()`);
    const toast = await waitForText(cdp, '#toast-root', '没有任何音源');
    check('零音源提示含端点选项引导', toast.includes('端点'), toast.trim());
    check('零音源不调用 startRoutes', (await cdp.evaluate('window.__starts.length')) === beforeZero);
  }

  // 2) 模式切换勾选如实保留 + 刷新不覆盖手选。
  {
    await cdp.navigate(`${base}`);
    await waitForTargets(cdp);
    await installStartCapture(cdp);
    await selectMonitor(cdp, 0);
    await cdp.evaluate(`document.querySelector('#cfg-mon-0-mic').click()`);
    await cdp.evaluate(`document.querySelector('input[name="audio-mode"][value="all_tracks"]').click()`);
    check('切到全音轨：端点勾选保留', (await checkBox(cdp, 'cfg-mon-0-endpoint')) === true);
    check('切到全音轨：麦克风勾选保留', (await checkBox(cdp, 'cfg-mon-0-mic')) === true);
    await cdp.evaluate(`document.querySelector('input[name="audio-mode"][value="isolated"]').click()`);
    check('切回隔离：勾选仍保留', (await checkBox(cdp, 'cfg-mon-0-endpoint')) === true && (await checkBox(cdp, 'cfg-mon-0-mic')) === true);
    await cdp.evaluate(`document.querySelector('#btn-refresh').click()`);
    await new Promise((r) => setTimeout(r, 900));
    check('刷新不覆盖手选（端点）', (await checkBox(cdp, 'cfg-mon-0-endpoint')) === true);
    check('刷新不覆盖手选（麦克风）', (await checkBox(cdp, 'cfg-mon-0-mic')) === true);
    const dto = await startAndCapture(cdp);
    check('模式切换后按实际勾选组装', JSON.stringify(sourceKinds(dto)) === JSON.stringify(['render_endpoint', 'mic']), JSON.stringify(sourceKinds(dto)));
  }

  // 3) index 交换不串屏（device_path 键）。
  {
    await cdp.navigate(`${base}?monitors_swap_on_refresh=1`);
    await waitForTargets(cdp);
    await selectMonitor(cdp, 0);
    await selectMonitor(cdp, 1);
    await cdp.evaluate(`document.querySelector('#cfg-mon-0-mic').click()`);
    await cdp.evaluate(`document.querySelector('#btn-refresh').click()`);
    await new Promise((r) => setTimeout(r, 1000));
    const sections = await cdp.evaluate(`[...document.querySelectorAll('.monitor-audio')].map((el) => ({
      key: el.dataset.monitorKey,
      index: Number(el.dataset.monitorIndex),
      mic: !!el.querySelector('input[id$="-mic"]')?.checked,
    }))`);
    const dell = sections.find((s) => s.key.includes('DELA1B2'));
    const hkc = sections.find((s) => s.key.includes('HKC1234'));
    check('index 交换后 DELL 段跟随 device_path（index=1）', dell?.index === 1, JSON.stringify(sections));
    check('index 交换后 DELL 手选麦克风保留', dell?.mic === true, JSON.stringify(dell));
    check('index 交换后 HKC 段不受影响', hkc?.index === 0 && hkc?.mic === false, JSON.stringify(hkc));
  }

  // 4) 双屏无元数据：临时键不跨刷新复用 / 不合并。
  {
    await cdp.navigate(`${base}?monitors_nometa=1&endpoints=none`);
    await waitForTargets(cdp);
    await selectMonitor(cdp, 0);
    await selectMonitor(cdp, 1);
    await cdp.evaluate(`document.querySelector('#cfg-mon-0-mic').click()`);
    const before = await cdp.evaluate(`[...document.querySelectorAll('.monitor-audio')].map((el) => el.dataset.monitorKey)`);
    check('双屏无元数据使用 idx: 临时键', JSON.stringify(before.sort()) === JSON.stringify(['idx:0', 'idx:1']), JSON.stringify(before));
    await cdp.evaluate(`document.querySelector('#btn-refresh').click()`);
    await new Promise((r) => setTimeout(r, 1000));
    const after = await cdp.evaluate(`[...document.querySelectorAll('.monitor-audio')].map((el) => ({
      key: el.dataset.monitorKey,
      index: Number(el.dataset.monitorIndex),
      mic: !!el.querySelector('input[id$="-mic"]')?.checked,
    }))`);
    check('临时键刷新后不复用（麦克风重置）', after.every((s) => s.mic === false), JSON.stringify(after));
    check('双屏仍为两个独立段（不合并）', after.length === 2 && after[0].key !== after[1].key, JSON.stringify(after));
  }

  // 5) 终态早于启动回包 + 迟到旧状态不覆盖终态。
  {
    await cdp.navigate(`${base}?scenario=early_terminal`);
    await waitForTargets(cdp);
    await selectMonitor(cdp, 0);
    await cdp.evaluate(`document.querySelector('#btn-start').click()`);
    await waitForInPage(cdp, "!!document.querySelector('.route-card.done .route-path')", 10000);
    const path1 = await cdp.evaluate(`document.querySelector('.route-card.done .route-path')?.textContent || ''`);
    await new Promise((r) => setTimeout(r, 500)); // 30ms recording 状态应已被终态护栏忽略
    const stillDone = await cdp.evaluate(`!!document.querySelector('.route-card.done') && !document.querySelector('.route-card.recording')`);
    check('终态早于启动回包：卡片为 done 且含成品路径', path1.length > 0, path1);
    check('迟到旧状态不覆盖终态', stillDone === true);
  }

  // 6) 失败告警 + 成品路径并存（route_error 后 route_finalized）。
  {
    await cdp.navigate(`${base}?scenario=error`);
    await waitForTargets(cdp);
    await selectMonitor(cdp, 0);
    await cdp.evaluate(`document.querySelector('#btn-start').click()`);
    await waitForInPage(cdp, "!!document.querySelector('.route-card.done')", 15000);
    const err = await cdp.evaluate(`document.querySelector('.route-card .route-err')?.textContent || ''`);
    const path = await cdp.evaluate(`document.querySelector('.route-card.done .route-path')?.textContent || ''`);
    check('失败告警保留 E_TARGET_LOST', err.includes('E_TARGET_LOST'), err.trim());
    check('失败告警与成品路径并存', path.length > 0, path);
  }

  // 7) REQ-10⑤ Ambiguous：未确认候选不得取枚举首项；显式选择候选后绑定该候选。
  {
    await cdp.navigate(`${base}?endpoints=ambiguous`);
    await waitForTargets(cdp);
    await installStartCapture(cdp);
    await selectMonitor(cdp, 0);
    check('Ambiguous 默认不勾选端点', (await checkBox(cdp, 'cfg-mon-0-endpoint')) === false);
    check('Ambiguous 默认系统混合', (await checkBox(cdp, 'cfg-mon-0-system')) === true);
    const status = await waitForText(cdp, '.monitor-audio [data-match-status]', '候选');
    check('Ambiguous 明示候选数量', status.includes('2 个候选'), status.trim());

    // 未选择具体候选时勾选端点：不得绑定列表首项 unrelated-first，回退系统混合。
    await cdp.evaluate(`document.querySelector('#cfg-mon-0-endpoint').click()`);
    check(
      'Ambiguous 未确认候选 → 端点勾选回退、系统混合保持',
      (await checkBox(cdp, 'cfg-mon-0-endpoint')) === false && (await checkBox(cdp, 'cfg-mon-0-system')) === true,
    );
    const first = await startAndCapture(cdp);
    const firstSources = first[0].audio.sources || [];
    check(
      'Ambiguous 勾选端点不绑定 unrelated-first',
      !firstSources.some((s) => s.kind === 'render_endpoint') && !JSON.stringify(firstSources).includes('ep-unrelated'),
      JSON.stringify(firstSources),
    );
    check('Ambiguous 未确认时使用系统混合', sourceKinds(first).includes('system'), JSON.stringify(sourceKinds(first)));

    // 显式选择候选 b → 绑定 b。
    await selectMonitor(cdp, 0);
    await cdp.evaluate(`(() => {
      const el = document.querySelector('#cfg-mon-0-endpoint-select');
      el.value = 'ep-candidate-b';
      el.dispatchEvent(new Event('change', { bubbles: true }));
    })()`);
    const second = await startAndCapture(cdp);
    const selected = (second[0].audio.sources || []).find((s) => s.kind === 'render_endpoint');
    check('Ambiguous 显式选择候选 b → 绑定 b', selected?.device_id === 'ep-candidate-b', JSON.stringify(second[0].audio.sources));
    check('Ambiguous 显式选择后仍不绑定 unrelated-first', !JSON.stringify(second[0].audio.sources).includes('ep-unrelated'));
  }
} catch (e) {
  check('runner 执行', false, e?.stack || String(e));
} finally {
  try {
    await cdp?.send('Browser.close');
  } catch { /* ignore */ }
  cdp?.close();
  if (process.platform === 'win32') {
    spawnSync('taskkill', ['/PID', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
  }
  child.kill();
  await new Promise((r) => setTimeout(r, 300));
  server.close();
  await rm(profile, { recursive: true, force: true }).catch(() => {});
}

if (failed) {
  console.error(`\n${failed} 个检查失败`);
  process.exit(1);
}
console.log('\n全部通过');
