// ============================================================================
// tools/ui/about-licenses.mjs — M19d1 About/许可证入口 runner
//
// 内置静态服务器 + headless Chrome（CDP，无第三方依赖；Node >= 22）。
// 驱动 ui/index.html + api.js + app.js 的 mock 分支：
//   - 关于导航/版本/Windows/FFmpeg 构建
//   - 固定 5 个文档按钮
//   - 点击懒加载纯文本（textContent）
//   - 无效文档 id 被 mock/api 拒绝
// 运行：node tools/ui/about-licenses.mjs（失败 exit 1）
// ============================================================================
'use strict';

import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { readFile, mkdtemp, rm, stat } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
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
      if (res.ok) return;
    } catch { /* not ready */ }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`Chrome 调试端口 ${port} 未就绪`);
}

class Cdp {
  constructor(wsUrl) {
    this.nextId = 1;
    this.pending = new Map();
    this.nextEvent = new Map();
    this.ws = new WebSocket(wsUrl);
    this.ready = new Promise((resolve, reject) => {
      this.ws.addEventListener('open', () => resolve());
      this.ws.addEventListener('error', (e) => reject(new Error(`CDP 错误: ${e.message || e}`)));
    });
    this.ws.addEventListener('message', (ev) => {
      const msg = JSON.parse(ev.data);
      if (msg.id) {
        const p = this.pending.get(msg.id);
        if (!p) return;
        this.pending.delete(msg.id);
        if (msg.error) p.reject(new Error(msg.error.message));
        else p.resolve(msg.result);
      } else if (msg.method) {
        for (const fn of this.nextEvent.get(msg.method) || []) fn(msg.params);
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
  async evaluate(expression) {
    const res = await this.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (res.exceptionDetails) throw new Error(res.exceptionDetails.text || '页面 JS 异常');
    return res.result?.value;
  }
  close() { try { this.ws.close(); } catch { /* ignore */ } }
}

async function waitFor(cdp, expression, timeoutMs = 8000) {
  const deadline = Date.now() + timeoutMs;
  let last;
  while (Date.now() < deadline) {
    last = await cdp.evaluate(expression);
    if (last) return last;
    await new Promise((r) => setTimeout(r, 80));
  }
  throw new Error(`等待超时: ${expression}（最后值 ${JSON.stringify(last)}）`);
}

const chrome = findChrome();
if (!chrome) {
  console.error('FAIL  未找到 Chrome/Edge（可设 CHROME_PATH）');
  process.exit(1);
}

const appSrc = await readFile(path.join(ROOT, 'ui', 'app.js'), 'utf8');
check('app.js 使用 textContent 渲染文档正文', appSrc.includes('pre.textContent = doc.text'));
check('app.js 含 About 渲染入口', appSrc.includes('async function renderAbout()'));

const { server, port } = await startStaticServer();
const profile = await mkdtemp(path.join(tmpdir(), 'sr-m19d-about-'));
const debugPort = 9800 + Math.floor(Math.random() * 150);
const child = spawn(chrome, [
  '--headless=new',
  `--remote-debugging-port=${debugPort}`,
  `--user-data-dir=${profile}`,
  '--no-first-run',
  '--no-default-browser-check',
  '--disable-gpu',
  '--window-size=1280,900',
  'about:blank',
], { stdio: 'ignore' });

let cdp;
try {
  await waitForDebugPort(debugPort);
  const tabRes = await fetch(`http://127.0.0.1:${debugPort}/json/new?${encodeURIComponent('about:blank')}`, { method: 'PUT' });
  const tab = await tabRes.json();
  cdp = new Cdp(tab.webSocketDebuggerUrl);
  await cdp.send('Runtime.enable');
  await cdp.send('Page.enable');
  await cdp.send('Page.navigate', { url: `http://127.0.0.1:${port}/ui/index.html` });
  await waitFor(cdp, "!!window.SR && !!document.querySelector('[data-nav=\"about\"]')", 10000);
  await waitFor(cdp, "document.querySelector('#env-badge').textContent.includes('mock')", 5000);

  await cdp.evaluate(`document.querySelector('[data-nav="about"]').click()`);
  await waitFor(cdp, "document.querySelector('#view-about').classList.contains('active')");
  await waitFor(cdp, "document.querySelector('#about-meta').textContent.includes('0.1.0')", 5000);
  const meta = await cdp.evaluate(`document.querySelector('#about-meta').textContent`);
  check('About 显示版本', meta.includes('0.1.0'), meta.trim());
  check('About 显示 FFmpeg 构建', meta.includes('46d899d7'), meta.trim());
  const docs = await cdp.evaluate(`document.querySelectorAll('#about-docs [data-about-doc]').length`);
  check('固定 5 个文档按钮', docs === 5, String(docs));

  await cdp.evaluate(`document.querySelector('#about-docs [data-about-doc="ffmpeg_license"]').click()`);
  await waitFor(cdp, "document.querySelector('#about-doc-text').textContent.includes('FFmpeg LGPLv3')", 5000);
  const text = await cdp.evaluate(`document.querySelector('#about-doc-text').textContent`);
  check('点击文档加载正文', text.includes('FFmpeg LGPLv3'), text.slice(0, 80));

  const rejected = await cdp.evaluate(`(async () => {
    try { await window.SR.api.readLegalDoc('../FFmpeg-LICENSE.txt'); return false; }
    catch { return true; }
  })()`);
  check('无效文档 id 被拒绝', rejected === true);
} catch (e) {
  failed++;
  console.error(`FAIL  runner 异常：${e.message || e}`);
} finally {
  cdp?.close();
  try { child.kill(); } catch { /* ignore */ }
  server.close();
  await rm(profile, { recursive: true, force: true }).catch(() => {});
}

if (failed) {
  console.error(`\n${failed} 项失败`);
  process.exit(1);
}
console.log('\n全部通过');
