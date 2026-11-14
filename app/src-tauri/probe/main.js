// Hydra probe: started by Hydra in Claude's place, with the account's exact environment variables,
// switches and (where Hydra applies one) sandbox, to show where each network path of an Electron
// app like Claude really leaves from. It only talks to public test services, never to Claude's
// own servers as Claude.
'use strict';
const { app, BrowserWindow, ipcMain, net, session, clipboard } = require('electron');
const path = require('path');
const fs = require('fs');
const https = require('https');
const http = require('http');
const dns = require('dns');
const dgram = require('dgram');
const { execFile } = require('child_process');

// Claude takes its profile folder from this switch the same way.
const userData = app.commandLine.getSwitchValue('user-data-dir');
if (userData) app.setPath('userData', userData);

const TRACE = 'https://www.cloudflare.com/cdn-cgi/trace';
const TIMEOUT = 12000;
// Hydra leaves what the findings are judged against in the profile folder.
const expected = (() => {
  try { return JSON.parse(fs.readFileSync(path.join(app.getPath('userData'), 'hydra-probe-expected.json'), 'utf8')); }
  catch { return {}; }
})();

const SWITCHES = [
  'user-data-dir', 'proxy-server', 'disable-quic', 'force-webrtc-ip-handling-policy', 'webrtc-ip-handling-policy',
  'enforce-webrtc-ip-permission-check', 'lang', 'accept-lang', 'no-sandbox',
];
const ENV_KEYS = [
  'HTTP_PROXY', 'HTTPS_PROXY', 'http_proxy', 'https_proxy', 'ALL_PROXY', 'all_proxy', 'NO_PROXY', 'no_proxy',
  'NODE_USE_ENV_PROXY', 'TZ', 'CLAUDE_CONFIG_DIR', 'LANG', 'LC_ALL',
];

function trace(text) {
  const fields = Object.fromEntries(String(text).trim().split('\n').map((l) => l.split('=')).filter((p) => p.length === 2));
  return { ip: fields.ip, where: [fields.loc, fields.colo].filter(Boolean).join(' · '), http: fields.http, warp: fields.warp };
}

function timed(promise) {
  return Promise.race([promise, new Promise((_, reject) => setTimeout(() => reject(new Error('超时')), TIMEOUT))]);
}

/** Runs one check, turning any failure into a result the report can show. */
async function attempt(fn) {
  const started = Date.now();
  try { return { ok: true, ...(await timed(fn())), ms: Date.now() - started }; }
  catch (e) { return { ok: false, error: String(e && e.message || e), ms: Date.now() - started }; }
}

function nodeGet(url, options = {}) {
  return new Promise((resolve, reject) => {
    const lib = url.startsWith('https:') ? https : http;
    const req = lib.get(url, { ...options, timeout: TIMEOUT }, (res) => {
      let body = '';
      res.setEncoding('utf8');
      res.on('data', (c) => { body += c; });
      res.on('end', () => resolve(body));
    });
    req.on('timeout', () => req.destroy(new Error('超时')));
    req.on('error', reject);
  });
}

async function chromiumGet(url) {
  const res = await net.fetch(url, { cache: 'no-store' });
  return res.text();
}

/** A one-off name (the service takes 32 letters and digits): whoever resolves it is the
 * resolver the service then reports. */
function ednsUrl() {
  const chars = 'abcdefghijklmnopqrstuvwxyz0123456789';
  const name = Array.from(crypto.getRandomValues(new Uint8Array(32)), (b) => chars[b % chars.length]).join('');
  return `http://${name}.edns.ip-api.com/json`;
}
function resolver(text) {
  try {
    const d = JSON.parse(text).dns || {};
    return { ip: d.ip, where: d.geo, resolver: true };
  } catch {
    // The request itself got through; only the answer is not the expected one.
    return { detail: '已连到检测服务，但它没有返回解析者' };
  }
}

function udpQuery() {
  return new Promise((resolve, reject) => {
    const socket = dgram.createSocket('udp4');
    // A plain DNS query for example.com, type A.
    const query = Buffer.from('abcd01000001000000000000076578616d706c6503636f6d0000010001', 'hex');
    socket.on('message', () => { socket.close(); resolve({ detail: '1.1.1.1:53 有应答' }); });
    socket.on('error', (e) => { socket.close(); reject(e); });
    socket.send(query, 53, '1.1.1.1', (e) => { if (e) { socket.close(); reject(e); } });
  });
}

function run(file, args, env) {
  return new Promise((resolve, reject) => {
    execFile(file, args, { env, timeout: TIMEOUT }, (e, stdout, stderr) => {
      if (e) reject(new Error((stderr || e.message).trim().split('\n').pop()));
      else resolve(stdout);
    });
  });
}

const CURL = process.platform === 'win32' ? 'curl.exe' : '/usr/bin/curl';
const NODE_SCRIPT = `require('https').get(${JSON.stringify(TRACE)}, (r) => { let b = ''; r.on('data', (c) => b += c); r.on('end', () => process.stdout.write(b)); }).on('error', (e) => { console.error(e.message); process.exit(1); });`;

async function mainChecks() {
  const env = { ...process.env };
  const checks = {
    chromiumProxy: attempt(async () => ({ detail: await session.defaultSession.resolveProxy(TRACE) })),
    chromium: attempt(async () => trace(await chromiumGet(TRACE))),
    claudeEdge: attempt(async () => trace(await chromiumGet('https://claude.ai/cdn-cgi/trace'))),
    chromiumDns: attempt(async () => resolver(await chromiumGet(ednsUrl()))),
    // The default agent, which NODE_USE_ENV_PROXY points at the proxy variables.
    nodeHttps: attempt(async () => trace(await nodeGet(TRACE))),
    nodeFetch: attempt(async () => trace(await (await fetch(TRACE)).text())),
    nodeDns: attempt(async () => resolver(await nodeGet(ednsUrl()))),
    // A library that makes its own agent leaves the proxy variables aside.
    nodeOwnAgent: attempt(async () => trace(await nodeGet(TRACE, { agent: new https.Agent() }))),
    // The same with an address instead of a name: blocked only if connections themselves are.
    nodeDirectIp: attempt(async () => trace(await nodeGet('https://1.1.1.1/cdn-cgi/trace', { agent: new https.Agent() }))),
    nodeDnsLookup: attempt(() => new Promise((resolve, reject) => dns.lookup('www.cloudflare.com', (e, address) => e ? reject(e) : resolve({ address })))),
    udp: attempt(udpQuery),
    ipv6: attempt(async () => ({ ip: (await nodeGet('https://api6.ipify.org', { agent: new https.Agent(), family: 6 })).trim() })),
    childCurlProxy: attempt(async () => trace(await run(CURL, ['-sS', '-m', '10', TRACE], env))),
    childCurlDirect: attempt(async () => trace(await run(CURL, ['-sS', '-m', '10', '--noproxy', '*', TRACE], env))),
    childNode: attempt(async () => trace(await run(process.execPath, ['-e', NODE_SCRIPT], { ...env, ELECTRON_RUN_AS_NODE: '1' }))),
    // Node before 22.21 / 24 (and tools like it) ignore the proxy variables altogether.
    childNodeLegacy: attempt(async () => {
      const legacy = { ...env, ELECTRON_RUN_AS_NODE: '1' };
      delete legacy.NODE_USE_ENV_PROXY;
      return trace(await run(process.execPath, ['-e', NODE_SCRIPT], legacy));
    }),
  };
  const results = {};
  for (const [k, v] of Object.entries(checks)) results[k] = await v;
  return {
    info: {
      electron: process.versions.electron, chrome: process.versions.chrome, node: process.versions.node,
      platform: `${process.platform} ${process.arch}`,
      userData: app.getPath('userData'),
      home: app.getPath('home'),
      timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
      switches: Object.fromEntries(SWITCHES.filter((s) => app.commandLine.hasSwitch(s)).map((s) => [s, app.commandLine.getSwitchValue(s) || true])),
      env: Object.fromEntries(ENV_KEYS.filter((k) => k in process.env).map((k) => [k, process.env[k]])),
    },
    expected,
    results,
  };
}

ipcMain.handle('main-checks', () => mainChecks());
ipcMain.handle('copy', (_, text) => {
  clipboard.writeText(text);
  const file = path.join(app.getPath('userData'), `probe-report-${new Date().toISOString().replace(/[:.]/g, '-')}.json`);
  fs.writeFileSync(file, text);
  return file;
});

app.whenReady().then(() => {
  const win = new BrowserWindow({
    width: 980, height: 820, title: 'Hydra 探针',
    webPreferences: { preload: path.join(__dirname, 'preload.js') },
  });
  win.loadFile(path.join(__dirname, 'index.html'));
});
app.on('window-all-closed', () => app.quit());
