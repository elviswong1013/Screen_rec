// ============================================================================
// tools/ui/settings-sample-rate.mjs — M15/REQ-2 设置页采样率行为 runner
//
// 方式：内置静态服务器 + headless Chrome（CDP，无第三方依赖；Node >= 22 的全局
// WebSocket/fetch）。直接驱动真实 ui/index.html + api.js + app.js 的 **mock** 分支
// （无 __TAURI__），覆盖规格 §4 的五态：
//   `<` 所选 < 原生率（升采样提示）/ `=` 相等 / `>` 低于原生率 /
//   未知（查询返回 null）/ 保存后重载保持 / 查询失败（异常隔离）；
// 另验证麦克风设备变更触发原生率刷新，以及 api.js real/mock 双接线静态断言。
//
// 运行：node tools/ui/settings-sample-rate.mjs   （失败 exit 1）
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

// ── Chrome / CDP ────────────────────────────────────────────────────────────
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

async function openSettings(cdp) {
  // 等 init() 完成（设置/目标加载后左栏不再是骨架），否则 renderSettings 读到
  // state.settings=null 只渲染骨架，不会出现 #set-sr。
  await waitForInPage(
    cdp,
    "(() => { const b = document.querySelector('#left-panel-body'); return !!b && b.children.length > 0 && !b.querySelector('.skel'); })()",
    15000
  );
  await cdp.evaluate(`document.querySelector('[data-nav="settings"]').click()`);
  await waitForInPage(cdp, "!!document.querySelector('#set-sr')", 8000);
}

async function clearStorageAndReload(cdp, url) {
  await cdp.navigate(url);
  await cdp.evaluate(`localStorage.clear()`);
  await cdp.navigate(url);
  await waitForInPage(cdp, "!!window.SR && !!(document.querySelector('#env-badge')||{}).textContent", 8000);
}

// P2-A：正常场景下「录制中修改成品容器」→ 收尾必须按启动时冻结的容器。
// 返回 { path, finalizedId, probe }；probe 在「修改设置前」采集
// （state=recording 且尚未收到终态），用于识别提前终态/路由已删除的假录制。
function runContainerSnapshot(cdp, { start, changeTo, label }) {
  return cdp.evaluate(`(async () => {
    const s = await window.SR.api.getSettings();
    await window.SR.api.setSettings({ ...s, output_container: ${JSON.stringify(start)} });
    return await new Promise((resolve, reject) => {
      const states = new Map();
      let finalizedId = null;
      let probe = null;
      const timer = setTimeout(() => {
        off();
        reject(new Error('等待 route_finalized 超时'));
      }, 20000);
      const off = window.SR.subscribeEvents((e) => {
        if (e.event === 'route_state' && e.id) states.set(e.id, e.state);
        if (e.event === 'route_finalized' && e.id) {
          finalizedId = e.id;
          clearTimeout(timer);
          off();
          resolve({ path: e.path, finalizedId, probe });
        }
      });
      (async () => {
        const { route_ids } = await window.SR.api.startRoutes([{
          target: { kind: 'monitor', index: 0 },
          video: { canvas_w: 320, canvas_h: 180, fps: 30, cursor: false },
          audio: { mode: 'isolated', sources: [] },
          output_dir: 'C:\\\\Users\\\\demo\\\\Videos\\\\Screen_rec',
          _ui_label: ${JSON.stringify(label)},
        }]);
        const id = route_ids[0];
        // mock 的 recording 状态事件延后 30ms 一拍。
        await new Promise((r) => setTimeout(r, 120));
        const cur = await window.SR.api.getSettings();
        probe = {
          id,
          state: states.get(id) || null,
          finalized: finalizedId !== null,
        };
        // 录制中把设置改为 changeTo：收尾必须仍按启动时冻结的 start。
        await window.SR.api.setSettings({ ...cur, output_container: ${JSON.stringify(changeTo)} });
        await window.SR.api.stopRoute(id);
      })().catch((e) => {
        clearTimeout(timer);
        off();
        reject(e);
      });
    });
  })()`);
}

function noteText(cdp) {
  return cdp.evaluate(
    `(() => { const el = document.querySelector('#set-sample-rate-note'); return el ? el.textContent : ''; })()`
  );
}

function selectedRate(cdp) {
  return cdp.evaluate(
    `(() => { const el = document.querySelector('#set-sr'); return el ? el.value : null; })()`
  );
}

// ── 主流程 ──────────────────────────────────────────────────────────────────
const chrome = findChrome();
if (!chrome) {
  console.error('FAIL  未找到 Chrome/Edge（可用 CHROME_PATH 指定）');
  process.exit(1);
}

// 静态接线断言（real/mock 双接线、显式透传、UI 字段）
const apiSrc = await readFile(path.join(ROOT, 'ui', 'api.js'), 'utf8');
const appSrc = await readFile(path.join(ROOT, 'ui', 'app.js'), 'utf8');
check('api.js real 接线 get_endpoint_mix_rate', apiSrc.includes("invoke('get_endpoint_mix_rate'"), 'real 分支必须独立透传新命令');
check(
  'api.js real 接线 deviceId 参数',
  /getEndpointMixRate[\s\S]{0,160}deviceId/.test(apiSrc),
  'real 分支应传 deviceId（Tauri camelCase）'
);
check('api.js mock 接线 getEndpointMixRate', /getEndpointMixRate\s*\(/.test(apiSrc.slice(apiSrc.indexOf('makeMock'))) || apiSrc.includes('async getEndpointMixRate'), 'mock 分支应提供同形返回值');
check('app.js 写入 audio_sample_rate', appSrc.includes('audio_sample_rate'), '保存设置必须包含采样率字段');
check('app.js 调用 getEndpointMixRate', appSrc.includes('getEndpointMixRate'), '设置页应查询原生混音率');
check('index.html 仅为挂载点', !(await readFile(path.join(ROOT, 'ui', 'index.html'), 'utf8')).includes('id="set-sr"'), '采样率控件由 app.js 动态生成');
// M17/REQ-16：成品容器 UI + mock 三路径（正常/提前终态/恢复）按容器返回。
check('app.js 写入 output_container', appSrc.includes('output_container'), '保存设置必须包含成品容器字段');
check('app.js 渲染成品格式下拉', appSrc.includes('id="set-container"'), '设置页需提供成品格式下拉');
check(
  'api.js mock 三路径按容器返回',
  apiSrc.includes('containerExt') &&
    /finalizeRoute[\s\S]{0,800}container/.test(apiSrc) &&
    /early_terminal[\s\S]{0,800}container/.test(apiSrc) &&
    /finalizeSession[\s\S]{0,800}containerExt/.test(apiSrc),
  'mock 的正常/提前终态/恢复路径必须按 settings.output_container 返回扩展名'
);

const { server, port: httpPort } = await startStaticServer();
const base = `http://127.0.0.1:${httpPort}/ui/index.html`;
const profile = await mkdtemp(path.join(tmpdir(), 'sr-m15-ui-'));
const debugPort = 9400 + Math.floor(Math.random() * 200);
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

  // 1) `<`：所选 48000 > 原生 44100 → 升采样提示
  {
    await clearStorageAndReload(cdp, `${base}?native_rate=44100&mic_rate=44100`);
    await openSettings(cdp);
    const value = await selectedRate(cdp);
    check('`<` 默认所选 48000', value === '48000', `得到 ${value}`);
    const text = await waitForText(cdp, '#set-sample-rate-note', '44100');
    check('`<` 显示原生率 44100', /44100/.test(text), text.trim());
    check('`<` 升采样提示', text.includes('升采样') && text.includes('不提升真实音质'), text.trim());
  }

  // 2) `=`：所选 = 原生 48000 → 无升采样提示
  {
    await clearStorageAndReload(cdp, `${base}?native_rate=48000&mic_rate=48000`);
    await openSettings(cdp);
    const text = await waitForText(cdp, '#set-sample-rate-note', '48000');
    check('`=` 显示原生率 48000', /48000/.test(text), text.trim());
    check('`=` 无升采样提示', !text.includes('升采样'), text.trim());
  }

  // 3) `>`：所选 48000 < 原生 96000 → 无升采样提示
  {
    await clearStorageAndReload(cdp, `${base}?native_rate=96000&mic_rate=96000`);
    await openSettings(cdp);
    const text = await waitForText(cdp, '#set-sample-rate-note', '96000');
    check('`>` 显示原生率 96000', /96000/.test(text), text.trim());
    check('`>` 无升采样提示', !text.includes('升采样'), text.trim());
  }

  // 4) 未知：查询返回 null → 显示「未知」，且保存仍可用
  {
    await clearStorageAndReload(cdp, `${base}?native_unknown=1`);
    await openSettings(cdp);
    const text = await waitForText(cdp, '#set-sample-rate-note', '未知');
    check('未知态显示「未知」', /未知/.test(text), text.trim());
    check('未知态不推导升采样提示', !text.includes('升采样'), text.trim());
    await cdp.evaluate(`(() => {
      const el = document.querySelector('#set-sr'); el.value = '96000';
      el.dispatchEvent(new Event('change', { bubbles: true }));
      document.querySelector('#btn-save-settings').click();
    })()`);
    await waitForText(cdp, '#settings-msg', '已保存');
    check('未知态保存仍成功', true);
  }

  // 5) 保存重载：96000 持久化（mock localStorage）后重载仍为 96000
  {
    await cdp.navigate(`${base}?native_unknown=1`);
    await openSettings(cdp);
    const value = await waitForInPage(cdp, "document.querySelector('#set-sr') && document.querySelector('#set-sr').value", 8000);
    check('保存重载后所选 96000', value === '96000', `得到 ${value}`);
  }

  // 6) 查询失败：与未知分离（异常被 UI 吞掉，不影响其它设置）
  {
    await clearStorageAndReload(cdp, `${base}?native_fail=1`);
    await openSettings(cdp);
    const text = await waitForText(cdp, '#set-sample-rate-note', '未知');
    check('查询失败显示「未知」', /未知/.test(text), text.trim());
    check('查询失败标注失败原因', text.includes('失败'), text.trim());
    check('查询失败不影响下拉可交互', (await selectedRate(cdp)) !== null);
  }

  // 7) 麦克风设备变更 → 原生率刷新（96000 → 44100）
  {
    await clearStorageAndReload(cdp, `${base}?native_rate=96000&mic_rate=44100`);
    await openSettings(cdp);
    await waitForText(cdp, '#set-sample-rate-note', '96000');
    await cdp.evaluate(`(() => {
      const el = document.querySelector('#set-mic');
      el.value = '';
      el.dispatchEvent(new Event('change', { bubbles: true }));
    })()`);
    await waitForText(cdp, '#set-sample-rate-note', '未启用');
    await cdp.evaluate(`(() => {
      const el = document.querySelector('#set-mic');
      el.value = 'mic-realtek';
      el.dispatchEvent(new Event('change', { bubbles: true }));
    })()`);
    const text = await waitForText(cdp, '#set-sample-rate-note', '44100');
    check('麦克风设备变更刷新原生率', /44100/.test(text), text.trim());
  }

  // 8) 四档选项存在
  {
    await openSettings(cdp);
    const options = await cdp.evaluate(
      `[...document.querySelectorAll('#set-sr option')].map((o) => o.value)`
    );
    check(
      '四档选项 44100/48000/88200/96000',
      JSON.stringify(options) === JSON.stringify(['44100', '48000', '88200', '96000']),
      JSON.stringify(options)
    );
    const text = await noteText(cdp);
    check('进程环回标注跟随引擎混音率', text.includes('跟随引擎混音率'), text.trim());
  }

  // 9) M17/REQ-16：成品格式下拉 + 说明 + 保存重载
  {
    await clearStorageAndReload(cdp, base);
    await openSettings(cdp);
    const container = await cdp.evaluate(
      `(() => { const el = document.querySelector('#set-container'); return el ? el.value : null; })()`
    );
    check('成品格式默认 mkv', container === 'mkv', `得到 ${container}`);
    const hint = await cdp.evaluate(
      `(() => { const el = document.querySelector('#set-container'); return el ? el.closest('.field').textContent : ''; })()`
    );
    check(
      '成品格式说明（内部 MKV 分段/崩溃安全 + 多音轨 MP4 限制）',
      hint.includes('崩溃安全') && hint.includes('多音轨 MP4'),
      hint.trim()
    );
    await cdp.evaluate(`(() => {
      const el = document.querySelector('#set-container');
      el.value = 'mp4';
      el.dispatchEvent(new Event('change', { bubbles: true }));
      document.querySelector('#btn-save-settings').click();
    })()`);
    await waitForText(cdp, '#settings-msg', '已保存');
    await cdp.navigate(base);
    await openSettings(cdp);
    const value = await waitForInPage(
      cdp,
      "document.querySelector('#set-container') && document.querySelector('#set-container').value",
      8000
    );
    check('成品格式保存重载后为 mp4', value === 'mp4', `得到 ${value}`);
  }

  // 10) M17/REQ-16：mock 正常路径按容器返回
  {
    const p = await cdp.evaluate(`(async () => {
      const paths = await new Promise(async (resolve) => {
        const off = window.SR.subscribeEvents((e) => {
          if (e.event === 'route_finalized') { off(); resolve(e.paths || [e.path]); }
        });
        const { route_ids } = await window.SR.api.startRoutes([{
          target: { kind: 'monitor', index: 0 },
          video: { canvas_w: 320, canvas_h: 180, fps: 30, cursor: false },
          audio: { mode: 'isolated', sources: [] },
          output_dir: 'C:\\\\Users\\\\demo\\\\Videos\\\\Screen_rec',
          _ui_label: 'M17-normal',
        }]);
        await window.SR.api.stopRoute(route_ids[0]);
      });
      return paths[0];
    })()`);
    check('mock 正常路径按容器返回 .mp4', typeof p === 'string' && p.endsWith('.mp4'), p);
  }

  // 11) M17/REQ-16：mock 提前终态路径按容器返回
  {
    await cdp.navigate(`${base}?scenario=early_terminal`);
    await waitForInPage(cdp, '!!window.SR', 8000);
    const p = await cdp.evaluate(`(async () => new Promise(async (resolve) => {
      const off = window.SR.subscribeEvents((e) => {
        if (e.event === 'route_finalized') { off(); resolve(e.path); }
      });
      await window.SR.api.startRoutes([{
        target: { kind: 'monitor', index: 0 },
        video: { canvas_w: 320, canvas_h: 180, fps: 30, cursor: false },
        audio: { mode: 'isolated', sources: [] },
        output_dir: 'C:\\\\Users\\\\demo\\\\Videos\\\\Screen_rec',
        _ui_label: 'M17-early',
      }]);
    }))()`);
    check('mock 提前终态路径按容器返回 .mp4', typeof p === 'string' && p.endsWith('.mp4'), p);
  }

  // 12) M17/REQ-16：mock 恢复路径按容器返回
  {
    const p = await cdp.evaluate(`(async () => {
      const { sessions } = await window.SR.api.recoverSessions();
      const sid = sessions[0].session_id;
      const res = await window.SR.api.finalizeSession(sid);
      return res.finals[0].path;
    })()`);
    check('mock 恢复路径按容器返回 .mp4', typeof p === 'string' && p.endsWith('.mp4'), p);
  }
  // 13) P1-2/P2-A：录制中改成品格式 → 收尾仍按启动快照容器。
  // 前置必须先切回正常场景：上一节 `scenario=early_terminal` 会在 startRoutes
  // 返回前发终态并删除路由，无法覆盖「录制中改设置」。
  {
    await cdp.navigate(base);
    await waitForInPage(cdp, '!!window.SR', 8000);
    const r = await runContainerSnapshot(cdp, {
      start: 'mkv',
      changeTo: 'mp4',
      label: 'P1-2-snapshot-mkv',
    });
    check(
      'P2-A：改设置时该 route 仍在录制（存在且状态=recording）',
      r?.probe?.state === 'recording' && r?.probe?.finalized === false,
      JSON.stringify(r?.probe)
    );
    check(
      'P2-A：终态按本次启动的 route ID 返回',
      r?.probe?.id && r.finalizedId === r.probe.id,
      `finalized=${r?.finalizedId} started=${r?.probe?.id}`
    );
    check(
      '录制中改成品格式 → 收尾仍按启动快照 .mkv',
      typeof r?.path === 'string' && r.path.endsWith('.mkv'),
      r?.path
    );
  }

  // 14) P2-A 反向分支：MP4 启动 → 录制中改 MKV → 收尾仍 .mp4。
  // 双分支断言可识别「收尾读取当前设置」的错误实现（正向测不出时反向必失败）。
  {
    const r = await runContainerSnapshot(cdp, {
      start: 'mp4',
      changeTo: 'mkv',
      label: 'P2-A-snapshot-mp4',
    });
    check(
      'P2-A 反向：改设置时该 route 仍在录制（存在且状态=recording）',
      r?.probe?.state === 'recording' && r?.probe?.finalized === false,
      JSON.stringify(r?.probe)
    );
    check(
      'P2-A 反向：终态按本次启动的 route ID 返回',
      r?.probe?.id && r.finalizedId === r.probe.id,
      `finalized=${r?.finalizedId} started=${r?.probe?.id}`
    );
    check(
      'P2-A 反向：录制中改成品格式 → 收尾仍按启动快照 .mp4',
      typeof r?.path === 'string' && r.path.endsWith('.mp4'),
      r?.path
    );
  }
} catch (e) {
  check('runner 执行', false, e?.stack || String(e));
} finally {
  // 先经 CDP 正常关闭浏览器（Chrome 启动器会派生独立浏览器进程，仅 kill 启动器
  // 会留下孤儿进程）；再按进程树兜底。
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
