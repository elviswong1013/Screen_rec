// ============================================================================
// tools/ui/recording-visibility.mjs — M19b2/REQ-3/4 开始栏摘要 + 首次告知 + ack runner
//
// 方式：内置静态服务器 + headless Chrome（CDP，无第三方依赖；Node >= 22 全局
// WebSocket/fetch）。驱动真实 ui/index.html + api.js + app.js 的 **mock** 分支：
//   - 摘要展开内容（目标/音源/输出目录/格式；纯录音 WAV 无画布帧率；区域最终尺寸）；
//   - 仅 app 音源一次启动；mic/system/render_endpoint 两击（确认并开始）；
//   - 取消不启动；配置变化复位；Ctrl+Enter 同门禁 + 防 repeat；
//   - 零音源仍走既有拒绝；区域 overlay 确认只交付选区、不算开始确认；
//   - ack 勾选/取消/重载持久化；设置页保存保留 ack；ack 保存失败不伪装成功。
//
// 运行：node tools/ui/recording-visibility.mjs   （失败 exit 1）
// ============================================================================
'use strict';

import { createServer } from 'node:http';
import { spawn, spawnSync } from 'node:child_process';
import { readFile, mkdtemp, rm, stat } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

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

// ── 静态服务器 / CDP（与既有 runner 同构） ─────────────────────────────────
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

// ── 页面驱动辅助 ────────────────────────────────────────────────────────────
async function waitForTargets(cdp) {
  await waitForInPage(
    cdp,
    "(() => { const b = document.querySelector('#left-panel-body'); return !!b && !!b.querySelector('.tgt-row') && !b.querySelector('.skel'); })()",
    15000
  );
}

async function clearStorageAndReload(cdp, url) {
  await cdp.navigate(url);
  await cdp.evaluate(`localStorage.clear()`);
  await cdp.navigate(url);
  await waitForInPage(cdp, "!!window.SR && !!(document.querySelector('#env-badge')||{}).textContent", 8000);
}

// 同时捕获 startRoutes / startAudioOnly（mock 替换，不触达事件模拟）。
async function installStartCapture(cdp) {
  await cdp.evaluate(`(() => {
    window.__starts = [];
    window.__audioStarts = [];
    window.SR.api.startRoutes = async (routes) => {
      window.__starts.push(routes);
      return { route_ids: routes.map((_, i) => 'rv-' + window.__starts.length + '-' + i) };
    };
    window.SR.api.startAudioOnly = async (cfgs) => {
      window.__audioStarts.push(cfgs);
      return { route_ids: cfgs[0].sources.map((_, i) => 'ra-' + window.__audioStarts.length + '-' + i) };
    };
  })()`);
}

const startsLen = (cdp) => cdp.evaluate('window.__starts.length');
const audioStartsLen = (cdp) => cdp.evaluate('window.__audioStarts.length');
const clickStart = (cdp) => cdp.evaluate(`document.querySelector('#btn-start').click()`);
const planVisible = (cdp) => cdp.evaluate(`!!document.querySelector('#start-plan')`);
const planText = (cdp) => cdp.evaluate(`document.querySelector('#start-plan')?.textContent || ''`);
const startBtnText = (cdp) => cdp.evaluate(`document.querySelector('#btn-start')?.textContent.trim() || ''`);

async function selectWindow(cdp) {
  await cdp.evaluate(`(() => {
    const row = [...document.querySelectorAll('.tgt-row')].find((r) => r.dataset.key.startsWith('window:'));
    if (row && !row.classList.contains('selected')) row.click();
  })()`);
  await waitForInPage(cdp, `!!document.querySelector('.tgt-row[data-key^="window:"].selected')`, 5000);
}

// 显式置窗口目标麦克风勾选态（配置跨启动保留，不能盲点点切）。
async function setMicChecked(cdp, want) {
  await cdp.evaluate(`(() => {
    const el = document.querySelector('#cfg-src-mic');
    if (el && el.checked !== ${want}) el.click();
  })()`);
  await new Promise((r) => setTimeout(r, 120));
}

async function selectMonitor(cdp, index) {
  const sel = `[data-key="monitor:${index}"]`;
  const exists = await cdp.evaluate(`!!document.querySelector(${JSON.stringify(sel)})`);
  if (!exists) {
    await cdp.evaluate(`document.querySelector('[data-tab="monitors"]').click()`);
    await waitForInPage(cdp, `!!document.querySelector(${JSON.stringify(sel)})`, 5000);
  }
  const selected = await cdp.evaluate(`(() => { const el = document.querySelector(${JSON.stringify(sel)}); return !!el && el.classList.contains('selected'); })()`);
  if (!selected) await cdp.evaluate(`document.querySelector(${JSON.stringify(sel)}).click()`);
  await waitForInPage(cdp, `!!document.querySelector('.monitor-audio[data-monitor-index="${index}"]')`, 5000);
}

async function pressCtrlEnter(cdp, repeat = false) {
  await cdp.evaluate(
    `document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', ctrlKey: true, bubbles: true, repeat: ${repeat} }))`
  );
  await new Promise((r) => setTimeout(r, 120));
}

// ── 主流程 ──────────────────────────────────────────────────────────────────
const chrome = findChrome();
if (!chrome) {
  console.error('FAIL  未找到 Chrome/Edge（可用 CHROME_PATH 指定）');
  process.exit(1);
}

// 静态接线断言
const apiSrc = await readFile(path.join(ROOT, 'ui', 'api.js'), 'utf8');
const appSrc = await readFile(path.join(ROOT, 'ui', 'app.js'), 'utf8');
check('app.js 单一 buildStartPlan 来源', appSrc.includes('function buildStartPlan()'), 'DTO/摘要/needs_notice 同函数');
check('app.js 确认状态机（Idle→Expanded→Starting）', appSrc.includes('startConfirm') && appSrc.includes('确认并开始'), '展开/确认两击');
check('app.js ack 保存先读后写', /onNoticeAckChange[\s\S]{0,500}getSettings\(\)[\s\S]{0,300}setSettings/.test(appSrc), '先取当前完整 Settings 仅改 ack');
check(
  'app.js 设置页保存保留 recording_notice_ack',
  /enqueueSettingsWrite[\s\S]{0,900}const base = await api\.getSettings\(\)[\s\S]{0,200}\.\.\.base/.test(appSrc),
  '设置写队列内读取最新 Settings 并合并，保留 ack'
);
check('app.js Ctrl+Enter 防 repeat', /keydown[\s\S]{0,300}e\.repeat/.test(appSrc), '按住重复触发不算第二次确认');
check('api.js mock 默认 recording_notice_ack=false', /recording_notice_ack:\s*false/.test(apiSrc), 'mock 默认设置');
check('api.js mock setSettings 合并保留字段', /Object\.assign\(structuredClone\(settings\),\s*next\)/.test(apiSrc), '前端 mock 合并语义');

const { server, port: httpPort } = await startStaticServer();
const base = `http://127.0.0.1:${httpPort}/ui/index.html`;
const profile = await mkdtemp(path.join(tmpdir(), 'sr-m19b2-ui-'));
const debugPort = 9700 + Math.floor(Math.random() * 200);
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

  // 1) 摘要展开内容 + 仅 app 音源一次启动（窗口目标，隔离模式默认仅应用轨）。
  {
    await clearStorageAndReload(cdp, base);
    await waitForTargets(cdp);
    await installStartCapture(cdp);
    await selectWindow(cdp);

    await cdp.evaluate(`document.querySelector('#btn-start-detail').click()`);
    await waitForInPage(cdp, `!!document.querySelector('#start-plan')`, 5000);
    const detail = await planText(cdp);
    check('摘要展开含目标', detail.includes('目标：'), detail.trim());
    check('摘要展开含音源标签', detail.includes('音源：应用 · '), detail.trim());
    check('摘要展开含输出目录', detail.includes('输出目录：'), detail.trim());
    check('摘要展开含格式 MKV 与画布帧率', detail.includes('MKV') && detail.includes('1920×1080 @30fps'), detail.trim());
    check('仅 app 音源不显示告知条', !(await cdp.evaluate(`!!document.querySelector('#start-notice')`)));
    // 收起
    await cdp.evaluate(`document.querySelector('#btn-start-detail').click()`);
    check('收起摘要', !(await planVisible(cdp)));

    await clickStart(cdp);
    await waitForInPage(cdp, 'window.__starts.length === 1', 5000);
    const first = await cdp.evaluate('window.__starts[0]');
    const kinds = first[0].audio.sources.map((s) => s.kind);
    check('仅 app 音源一次点击启动', kinds.length > 0 && kinds.every((k) => k === 'app'), JSON.stringify(kinds));
    check('启动完成后确认状态复位（面板关闭）', !(await planVisible(cdp)));
  }

  // 2) 麦克风音源 → 两击（第一次仅展开告知）。
  {
    await selectWindow(cdp);
    await setMicChecked(cdp, true);
    const before = await startsLen(cdp);
    await clickStart(cdp);
    await waitForInPage(cdp, `!!document.querySelector('#start-plan')`, 5000);
    check('mic：第一次点击只展开不启动', (await startsLen(cdp)) === before);
    const notice = await cdp.evaluate(`document.querySelector('#start-notice')?.textContent || ''`);
    check('mic：告知条含麦克风且 ≤2 行结构', notice.includes('麦克风') && notice.includes('不再显示此提示'), notice.trim());
    check('mic：按钮变为「确认并开始」', (await startBtnText(cdp)) === '确认并开始', await startBtnText(cdp));
    await clickStart(cdp);
    await waitForInPage(cdp, `window.__starts.length === ${before + 1}`, 5000);
    const dto = (await cdp.evaluate('window.__starts[window.__starts.length - 1]'))[0];
    check('mic：第二次确认才启动且 DTO 含 mic', dto.audio.sources.some((s) => s.kind === 'mic'), JSON.stringify(dto.audio.sources));
    check('mic：启动后复位', !(await planVisible(cdp)));
  }

  // 3) 取消不启动；再次点开始需重新确认。
  {
    await selectWindow(cdp);
    await setMicChecked(cdp, true);
    const before = await startsLen(cdp);
    await clickStart(cdp);
    await waitForInPage(cdp, `!!document.querySelector('#start-plan')`, 5000);
    await cdp.evaluate(`document.querySelector('#btn-start-cancel').click()`);
    check('取消：面板关闭且不启动', !(await planVisible(cdp)) && (await startsLen(cdp)) === before);
    await clickStart(cdp);
    await waitForInPage(cdp, `!!document.querySelector('#start-plan')`, 5000);
    check('取消后再次开始仍需先确认（不启动）', (await startsLen(cdp)) === before);
    await cdp.evaluate(`document.querySelector('#btn-start-cancel').click()`);
  }

  // 4) 配置变化复位：取消勾选麦克风 → 面板关闭；输出目录输入 → 面板关闭。
  {
    await selectWindow(cdp);
    await setMicChecked(cdp, true);
    const before = await startsLen(cdp);
    await clickStart(cdp);
    await waitForInPage(cdp, `!!document.querySelector('#start-plan')`, 5000);
    await setMicChecked(cdp, false); // 取消 mic → 仅 app
    check('音源变化 → 展开态复位', !(await planVisible(cdp)));
    await clickStart(cdp);
    await waitForInPage(cdp, `window.__starts.length === ${before + 1}`, 5000);
    check('复位后仅 app 音源一次启动', true);

    await selectWindow(cdp);
    await setMicChecked(cdp, true);
    await clickStart(cdp);
    await waitForInPage(cdp, `!!document.querySelector('#start-plan')`, 5000);
    await cdp.evaluate(`(() => {
      const el = document.querySelector('#cfg-outdir');
      el.value = el.value + '\\\\sub';
      el.dispatchEvent(new Event('input', { bubbles: true }));
    })()`);
    await new Promise((r) => setTimeout(r, 200));
    check('输出目录变化 → 展开态复位', !(await planVisible(cdp)));
    await cdp.evaluate(`(() => { const el = document.querySelector('#cfg-outdir'); el.value = el.value.replace(/\\\\sub$/, ''); el.dispatchEvent(new Event('input', { bubbles: true })); })()`);
    // 此处面板已复位 Idle（mic 仍勾选，供后续小节显式置态）。
  }

  // 5) Ctrl+Enter 同门禁 + repeat 防重复。
  {
    await selectWindow(cdp);
    await setMicChecked(cdp, true);
    const before = await startsLen(cdp);
    await pressCtrlEnter(cdp);
    check('Ctrl+Enter 第一次只展开', (await planVisible(cdp)) && (await startsLen(cdp)) === before);
    await pressCtrlEnter(cdp, true); // 按住 repeat
    check('Ctrl+Enter repeat 不算第二次确认', (await startsLen(cdp)) === before && (await planVisible(cdp)));
    await pressCtrlEnter(cdp);
    await waitForInPage(cdp, `window.__starts.length === ${before + 1}`, 5000);
    check('Ctrl+Enter 第二次确认启动', true);
  }

  // 6) 零音源仍走既有拒绝（降级场景：窗口目标取消系统音频后零音源）。
  {
    await cdp.navigate(`${base}?scenario=degraded`);
    await waitForTargets(cdp);
    await installStartCapture(cdp);
    await selectWindow(cdp);
    // 降级（无进程环回）：窗口仅系统/麦克风可选；取消系统 → 零音源。
    await cdp.evaluate(`(() => { const el = document.querySelector('#cfg-src-system'); if (el && el.checked) el.click(); })()`);
    await new Promise((r) => setTimeout(r, 150));
    await clickStart(cdp);
    const toast = await waitForText(cdp, '#toast-root', '没有任何音源');
    check('零音源仍按既有逻辑拒绝', toast.includes('没有任何音源'), toast.trim());
    check('零音源不调用 startRoutes、不展开告知', (await startsLen(cdp)) === 0 && !(await planVisible(cdp)));
  }

  // 7) 渲染端点音源 → 两击。
  {
    await cdp.navigate(base);
    await waitForTargets(cdp);
    await installStartCapture(cdp);
    await selectMonitor(cdp, 0);
    const before = await startsLen(cdp);
    await clickStart(cdp);
    await waitForInPage(cdp, `!!document.querySelector('#start-plan')`, 5000);
    const notice = await cdp.evaluate(`document.querySelector('#start-notice')?.textContent || ''`);
    check('render_endpoint：第一次点击只展开', (await startsLen(cdp)) === before);
    check('render_endpoint：告知条含端点', notice.includes('显示器输出端点'), notice.trim());
    await clickStart(cdp);
    await waitForInPage(cdp, `window.__starts.length === ${before + 1}`, 5000);
    const dto = (await cdp.evaluate('window.__starts[window.__starts.length - 1]'))[0];
    check('render_endpoint：确认后 DTO 含端点', dto.audio.sources.some((s) => s.kind === 'render_endpoint'), JSON.stringify(dto.audio.sources));
  }

  // 8) 纯录音：摘要显示 WAV、不显示画布/帧率；系统音源两击。
  {
    await cdp.evaluate(`document.querySelector('#mode-tabs button[data-mode="audio"]').click()`);
    await new Promise((r) => setTimeout(r, 200));
    const before = await audioStartsLen(cdp);
    await clickStart(cdp); // 默认系统音频勾选 → needs_notice
    await waitForInPage(cdp, `!!document.querySelector('#start-plan')`, 5000);
    const detail = await planText(cdp);
    check('纯录音：第一次点击只展开（系统音源）', (await audioStartsLen(cdp)) === before);
    check('纯录音：摘要含 WAV', detail.includes('WAV'), detail.trim());
    check('纯录音：摘要不含画布/帧率', !detail.includes('画布') && !/fps/.test(detail), detail.trim());
    check('纯录音：告知条含系统声音', (await cdp.evaluate(`document.querySelector('#start-notice')?.textContent || ''`)).includes('系统声音'));
    await clickStart(cdp);
    await waitForInPage(cdp, `window.__audioStarts.length === ${before + 1}`, 5000);
    const dto = (await cdp.evaluate('window.__audioStarts[window.__audioStarts.length - 1]'))[0];
    check('纯录音：确认后 startAudioOnly DTO 含 system', dto.sources.some((s) => s.kind === 'system'), JSON.stringify(dto.sources));
    await cdp.evaluate(`document.querySelector('#mode-tabs button[data-mode="screen"]').click()`);
    await new Promise((r) => setTimeout(r, 200));
  }

  // 9) 区域：overlay 确认只交付选区（不算开始确认）；摘要显示最终选区尺寸；两击。
  {
    await cdp.evaluate(`(() => {
      window.SR.api.openRegionOverlay = async (mode, requestId) => { window.__reqId = requestId; return null; };
    })()`);
    await cdp.evaluate(`document.querySelector('#btn-region-record').click()`);
    await waitForInPage(cdp, '!!window.__reqId', 5000);
    const before = await startsLen(cdp);
    await cdp.evaluate(`window.SR.api.confirmRegionRecord(window.__reqId, { x: 0, y: 0, w: 200, h: 100 })`);
    await waitForText(cdp, '#toast-root', '区域已就绪');
    await waitForInPage(cdp, `(document.querySelector('#start-bar .summary')||{}).textContent?.includes('区域 200×100')`, 5000);
    check('区域 overlay 确认只交付选区（不启动）', (await startsLen(cdp)) === before);
    // 区域隔离=仅应用；零应用被禁用 → 切全音轨（默认含系统 → needs_notice）。
    await cdp.evaluate(`document.querySelector('input[name="audio-mode"][value="all_tracks"]').click()`);
    await new Promise((r) => setTimeout(r, 200));
    await clickStart(cdp);
    await waitForInPage(cdp, `!!document.querySelector('#start-plan')`, 5000);
    check('区域：第一次点开始仍只展开（overlay 确认不算数）', (await startsLen(cdp)) === before);
    check('区域：摘要显示最终选区尺寸 200×100', (await planText(cdp)).includes('区域 200×100'), (await planText(cdp)).trim());
    await clickStart(cdp);
    await waitForInPage(cdp, `window.__starts.length === ${before + 1}`, 5000);
    const dto = (await cdp.evaluate('window.__starts[window.__starts.length - 1]'))[0];
    check(
      '区域：确认后 DTO kind=region 且 rect 200×100',
      dto.target.kind === 'region' && dto.target.rect.w === 200 && dto.target.rect.h === 100,
      JSON.stringify(dto.target)
    );
    // 还原音轨模式
    await cdp.evaluate(`document.querySelector('input[name="audio-mode"][value="isolated"]').click()`);
    await new Promise((r) => setTimeout(r, 150));
  }

  // 10) ack 勾选 → 单次启动；重载后保持。
  {
    await clearStorageAndReload(cdp, base);
    await waitForTargets(cdp);
    await installStartCapture(cdp);
    await selectWindow(cdp);
    await setMicChecked(cdp, true);
    const before = await startsLen(cdp);
    await clickStart(cdp);
    await waitForInPage(cdp, `!!document.querySelector('#start-notice-ack')`, 5000);
    await cdp.evaluate(`document.querySelector('#start-notice-ack').click()`); // 勾选「不再显示」
    await waitForText(cdp, '#toast-root', '已保存');
    const ackSaved = await cdp.evaluate(`(async () => (await window.SR.api.getSettings()).recording_notice_ack)()`);
    check('勾选 ack 保存成功（getSettings=true）', ackSaved === true, JSON.stringify(ackSaved));
    check('勾选本身不启动', (await startsLen(cdp)) === before);
    await new Promise((r) => setTimeout(r, 200));
    check('ack=true 后按钮不再是「确认并开始」', (await startBtnText(cdp)) === '开始录制', await startBtnText(cdp));
    await clickStart(cdp);
    await waitForInPage(cdp, `window.__starts.length === ${before + 1}`, 5000);
    check('ack=true：一次点击启动', true);

    await cdp.navigate(base); // 重载（localStorage 保留 ack=true）
    await waitForTargets(cdp);
    await installStartCapture(cdp);
    await selectWindow(cdp);
    await setMicChecked(cdp, true);
    const before2 = await startsLen(cdp);
    await clickStart(cdp);
    await waitForInPage(cdp, `window.__starts.length === ${before2 + 1}`, 5000);
    check('ack 重载后保持：一次启动且未展开告知', !(await planVisible(cdp)));
  }

  // 11) 取消勾选 ack → 两击门禁恢复。
  {
    await selectWindow(cdp);
    await setMicChecked(cdp, true);
    await cdp.evaluate(`document.querySelector('#btn-start-detail').click()`); // 手动展开摘要
    await waitForInPage(cdp, `!!document.querySelector('#start-notice-ack')`, 5000);
    const checked0 = await cdp.evaluate(`document.querySelector('#start-notice-ack').checked`);
    check('ack=true 时展开摘要仍可见已勾选复选框', checked0 === true);
    await cdp.evaluate(`document.querySelector('#start-notice-ack').click()`); // 取消勾选
    await waitForText(cdp, '#toast-root', '已保存');
    const ackSaved = await cdp.evaluate(`(async () => (await window.SR.api.getSettings()).recording_notice_ack)()`);
    check('取消勾选 ack 保存成功（getSettings=false）', ackSaved === false, JSON.stringify(ackSaved));
    await cdp.evaluate(`document.querySelector('#btn-start-detail').click()`); // 收起
    const before = await startsLen(cdp);
    await clickStart(cdp);
    await waitForInPage(cdp, `!!document.querySelector('#start-plan')`, 5000);
    check('取消 ack 后恢复两击门禁（第一次不启动）', (await startsLen(cdp)) === before);
    await cdp.evaluate(`document.querySelector('#btn-start-cancel').click()`);
  }

  // 12) 设置页保存其他字段时保留当前 ack。
  {
    // 先勾选 ack=true（走开始栏复选框）
    await clickStart(cdp);
    await waitForInPage(cdp, `!!document.querySelector('#start-notice-ack')`, 5000);
    await cdp.evaluate(`document.querySelector('#start-notice-ack').click()`);
    await waitForText(cdp, '#toast-root', '已保存');
    await cdp.evaluate(`document.querySelector('#btn-start-detail').click()`); // 收起
    // 设置页改成品格式并保存
    await cdp.evaluate(`document.querySelector('[data-nav="settings"]').click()`);
    await waitForInPage(cdp, '!!document.querySelector("#set-container")', 8000);
    await cdp.evaluate(`(() => {
      const el = document.querySelector('#set-container');
      el.value = 'mp4';
      el.dispatchEvent(new Event('change', { bubbles: true }));
    })()`);
    const selectedContainer = await cdp.evaluate(`document.querySelector('#set-container').value`);
    check('设置页选择 mp4 生效', selectedContainer === 'mp4', JSON.stringify(selectedContainer));
    await cdp.evaluate(`document.querySelector('#btn-save-settings').click()`);
    await waitForText(cdp, '#settings-msg', '已保存');
    const s = await cdp.evaluate(`window.SR.api.getSettings()`);
    check('设置页保存后 ack 保留 true', s.recording_notice_ack === true, JSON.stringify(s.recording_notice_ack));
    check(
      '设置页保存其他字段生效且不丢 M19a 字段',
      s.output_container === 'mp4' && s.include_window_title_in_filename === false && s.audio_sample_rate === 48000,
      JSON.stringify({ c: s.output_container, t: s.include_window_title_in_filename, sr: s.audio_sample_rate })
    );
    await cdp.evaluate(`document.querySelector('[data-nav="record"]').click()`);
    await new Promise((r) => setTimeout(r, 200));
  }

  // 13) ack 保存失败不伪装成功（状态/界面保持真实值）。
  {
    await selectWindow(cdp);
    await setMicChecked(cdp, true);
    await cdp.evaluate(`document.querySelector('#btn-start-detail').click()`);
    await waitForInPage(cdp, `!!document.querySelector('#start-notice-ack')`, 5000);
    await cdp.evaluate(`(() => {
      window.__origSetSettings = window.SR.api.setSettings;
      window.SR.api.setSettings = async () => { throw 'E_IO_MOCK：磁盘不可写'; };
    })()`);
    await cdp.evaluate(`document.querySelector('#start-notice-ack').click()`); // 试图取消 ack=true
    await waitForText(cdp, '#toast-root', '保存失败');
    const ackAfter = await cdp.evaluate(`(async () => (await window.SR.api.getSettings()).recording_notice_ack)()`);
    check('保存失败：ack 保持 true（未伪装成功）', ackAfter === true, JSON.stringify(ackAfter));
    await new Promise((r) => setTimeout(r, 200));
    const boxState = await cdp.evaluate(`document.querySelector('#start-notice-ack')?.checked`);
    check('保存失败：复选框回显真实状态（仍勾选）', boxState === true, JSON.stringify(boxState));
    await cdp.evaluate(`window.SR.api.setSettings = window.__origSetSettings`);
    await cdp.evaluate(`document.querySelector('#btn-start-detail').click()`); // 收起
  }
} catch (e) {
  check('runner 执行', false, e?.stack || String(e));
} finally {
  // 先经 CDP 正常关闭浏览器，再按进程树兜底。
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
