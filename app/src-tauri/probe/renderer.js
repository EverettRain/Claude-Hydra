'use strict';
// Page-side checks, the verdict for every path, and the report (shown in full, exported with IPs
// masked so it can be shared).

const STUN = 'stun:stun.l.google.com:19302';

function webrtc() {
  return new Promise((resolve) => {
    const found = [];
    let pc;
    try { pc = new RTCPeerConnection({ iceServers: [{ urls: STUN }] }); } catch (e) { resolve({ error: String(e) }); return; }
    pc.createDataChannel('probe');
    pc.onicecandidate = (e) => {
      if (!e.candidate) return;
      const parts = e.candidate.candidate.split(' ');
      found.push({ type: parts[7], address: parts[4], protocol: parts[2] });
    };
    pc.createOffer().then((o) => pc.setLocalDescription(o)).catch(() => {});
    setTimeout(() => { pc.close(); resolve({ candidates: found }); }, 6000);
  });
}

async function pageChecks() {
  const ua = navigator.userAgentData;
  return {
    timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    offset: -new Date().getTimezoneOffset() / 60,
    languages: navigator.languages.join(', '),
    userAgent: navigator.userAgent,
    brands: ua ? ua.brands.map((b) => `${b.brand} ${b.version}`).join(', ') : '',
    webdriver: navigator.webdriver,
    webrtc: await webrtc(),
  };
}

function mask(ip) {
  if (!ip) return ip;
  if (ip.includes(':')) return ip.split(':').slice(0, 2).join(':') + ':…';
  const p = ip.split('.');
  return p.length === 4 ? `${p[0]}.${p[1]}.x.x` : ip;
}

function isPublic(address) {
  if (!address || address.endsWith('.local')) return false;
  return !/^(10\.|127\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.|169\.254\.|fe80:|fc|fd|::1)/i.test(address);
}

/** Every row of the report: group, label, verdict and text. `shared` hides IPs and the home
 * folder (which carries the user name) for a report meant to be passed on. */
function rows(data, page, shared) {
  const { results: r, expected: x, info } = data;
  const show = shared ? mask : (ip) => ip;
  const local = (text) => shared && info.home ? String(text).split(info.home).join('~') : text;
  const proxied = !!x.relayPort;
  const exit = x.exitIp || (r.chromium.ok && r.chromium.ip);
  const out = [];
  const add = (group, label, status, text) => out.push({ group, label, status, text });
  const where = (res) => [show(res.ip), res.where].filter(Boolean).join('  ');

  // Paths that should leave through the node.
  const routed = (group, label, res) => {
    if (!res.ok) return add(group, label, 'error', res.error);
    if (!proxied) return add(group, label, 'info', `${where(res)}（账号未配置节点）`);
    add(group, label, res.ip === exit ? 'pass' : 'fail', `${where(res)}${res.ip === exit ? '' : '（与节点出口不同）'}${res.http ? '  ' + res.http : ''}`);
  };
  // A system-wide proxy in TUN mode answers local lookups with addresses from 198.18.0.0/15
  // ("fake IP") and carries whatever bypasses Hydra, often to the same node.
  const fakeIp = r.nodeDnsLookup.ok && /^198\.1[89]\./.test(r.nodeDnsLookup.address);
  // Paths built to bypass the proxy settings: anything that gets out went around Hydra's relay
  // and its exit guard, whatever exit it ended up on.
  const bypass = (group, label, res, what) => {
    if (!res.ok) return add(group, label, 'pass', `连不出去：${res.error}`);
    if (!proxied) return add(group, label, 'info', res.detail || where(res));
    const caught = fakeIp && res.ip && res.ip === exit ? '（出口与节点相同，是被系统级代理接管了，没有经过 Hydra 的中继与出口保护）' : '';
    add(group, label, 'fail', `${res.detail || where(res)}  ← ${what}${caught}`);
  };

  const tzWanted = x.timezone;
  const tzVerdict = (tz) => !tzWanted ? 'info' : tz === tzWanted ? 'pass' : 'fail';
  add('页面', '时区', tzVerdict(page.timezone), `${page.timezone}（UTC${page.offset >= 0 ? '+' : ''}${page.offset}）${tzWanted ? `，账号应为 ${tzWanted}` : ''}`);
  add('页面', '语言', 'info', page.languages);
  add('页面', '自动化标记', page.webdriver ? 'warn' : 'pass', `navigator.webdriver = ${page.webdriver}`);
  const cands = page.webrtc.candidates || [];
  const exposed = cands.filter((c) => isPublic(c.address));
  // A private address is no exit, but it tells a page this machine's place on its network.
  const lan = cands.some((c) => c.address && !isPublic(c.address) && !c.address.endsWith('.local'));
  // Electron takes its WebRTC policy only from the app (webContents.setWebRTCIPHandlingPolicy);
  // the Chromium switches Hydra passes do not reach it.
  const policy = cands.length ? '  ← Electron 应用的 WebRTC 策略只能由应用自己设置，Hydra 的启动开关对它无效' : '';
  add('页面', 'WebRTC', page.webrtc.error ? 'info' : exposed.length ? 'fail' : lan ? 'warn' : 'pass',
    page.webrtc.error || (cands.length ? cands.map((c) => `${c.type} ${c.protocol} ${c.address.endsWith('.local') ? c.address : show(c.address)}`).join('，') + policy : '没有候选地址'));
  add('页面', '浏览器标识', 'info', page.brands || page.userAgent);

  add('Chromium 网络', '代理设置', proxied ? (/SOCKS|PROXY/i.test(r.chromiumProxy.detail || '') ? 'pass' : 'fail') : 'info', r.chromiumProxy.ok ? r.chromiumProxy.detail : r.chromiumProxy.error);
  routed('Chromium 网络', 'Cloudflare 看到的出口', r.chromium);
  routed('Chromium 网络', 'claude.ai 边缘看到的出口', r.claudeEdge);
  add('Chromium 网络', '替你解析域名的 DNS', r.chromiumDns.ok ? 'info' : 'error', r.chromiumDns.ok ? where(r.chromiumDns) : r.chromiumDns.error);

  routed('主进程 Node', 'https（默认代理设置）', r.nodeHttps);
  routed('主进程 Node', 'fetch（默认代理设置）', r.nodeFetch);
  const d = r.nodeDns;
  const sameResolver = d.ok && r.chromiumDns.ok && d.ip && (d.ip === r.chromiumDns.ip || d.where === r.chromiumDns.where);
  add('主进程 Node', '请求时替你解析域名的 DNS', !d.ok ? 'error' : !proxied ? 'info' : sameResolver ? 'pass' : 'fail',
    !d.ok ? d.error : `${d.detail || where(d)}${proxied ? (sameResolver ? '（与 Chromium 相同，经节点解析）' : '  ← 与经节点的解析者不同') : ''}`);
  bypass('主进程 Node', '自建连接的库（不用默认代理）', r.nodeOwnAgent, '自己建连接的库绕过了代理变量');
  bypass('主进程 Node', '直连 IP（不查域名）', r.nodeDirectIp, '直接连 IP 绕过了代理变量');
  const l = r.nodeDnsLookup;
  add('主进程 Node', '本机 DNS 查询', !l.ok ? 'pass' : !proxied ? 'info' : fakeIp ? 'warn' : 'fail',
    !l.ok ? `查不到：${l.error}`
      : fakeIp ? `得到 ${l.address}：系统级代理（TUN，fake IP）接管了本机 DNS，查询不会直接发给运营商；绕过 Hydra 的流量也会交给它`
        : `本机解析得到 ${show(l.address)}  ← 查询的域名发给了本机的 DNS 服务器`);
  bypass('主进程 Node', 'UDP 直发', r.udp, 'UDP 可以绕过节点');
  bypass('主进程 Node', 'IPv6 直连', r.ipv6, 'IPv6 可以绕过节点');

  routed('子进程（如 MCP）', 'curl（读代理变量）', r.childCurlProxy);
  bypass('子进程（如 MCP）', 'curl（忽略代理变量）', r.childCurlDirect, '不读代理变量的工具直连');
  routed('子进程（如 MCP）', 'Node 子进程（继承代理变量）', r.childNode);
  bypass('子进程（如 MCP）', 'Node 子进程（不读代理变量，如旧版 Node）', r.childNodeLegacy, '不读代理变量的 Node 工具直连');

  add('启动环境', '保护', 'info', PROTECTION[x.protection] || '未启用');
  add('启动环境', '系统级代理', 'info', fakeIp ? '检测到（TUN / fake IP）：直连测试显示的出口取决于它的规则' : r.nodeDnsLookup.ok ? '未检测到' : '无法判断（本机 DNS 查询被拦截）');
  add('启动环境', '版本', 'info', `Electron ${info.electron} · Chromium ${info.chrome} · Node ${info.node} · ${info.platform}`);
  add('启动环境', '数据目录', 'info', local(info.userData));
  add('启动环境', '启动开关', 'info', Object.entries(info.switches).map(([k, v]) => v === true ? `--${k}` : `--${k}=${local(v)}`).join(' ') || '无');
  add('启动环境', '环境变量', 'info', Object.entries(info.env).map(([k, v]) => `${k}=${local(v)}`).join('  ') || '无');
  add('启动环境', '节点出口（Hydra 启动前检测）', 'info', x.exitIp ? show(x.exitIp) : '未配置节点');
  return out;
}

const PROTECTION = {
  seatbelt: 'Seatbelt：只允许连本机中继端口，系统 DNS 服务被拒绝。macOS 不允许沙箱嵌套，探针因此关闭了 Chromium 自身的沙箱',
  sandboxie: 'Sandboxie：在这个环境的沙箱中运行，只放行它的中继，并加载 Hydra 的 DNS 与身份保护（替换机器标识，隐藏网卡上的公网地址）',
};

const LABEL = { pass: '正常', fail: '泄漏', warn: '注意', error: '连不通', info: '' };

function render(list) {
  const groups = [...new Set(list.map((r) => r.group))];
  document.getElementById('report').replaceChildren(...groups.map((g) => {
    const section = document.createElement('section');
    const h = document.createElement('h2');
    h.textContent = g;
    section.append(h);
    for (const r of list.filter((x) => x.group === g)) {
      const row = document.createElement('div');
      row.className = 'row';
      const [label, tag, text] = ['span', 'span', 'span'].map((t) => document.createElement(t));
      label.textContent = r.label;
      tag.className = `tag ${r.status}`;
      tag.textContent = LABEL[r.status];
      text.className = 'text';
      text.textContent = r.text;
      row.append(label, tag, text);
      section.append(row);
    }
    return section;
  }));
}

let shared = null;

async function start() {
  const status = document.getElementById('status');
  const copy = document.getElementById('copy');
  status.textContent = '检测中…（约 15 秒）';
  copy.disabled = true;
  const [data, page] = await Promise.all([window.probe.mainChecks(), pageChecks()]);
  const guard = { seatbelt: '（加 Seatbelt 保护）', sandboxie: '（在 Sandboxie 沙箱中）' }[data.expected.protection] || '（不加保护）';
  if (data.expected.subject) document.getElementById('sub').textContent = `按${data.expected.subject}的启动方式运行${guard}，检查每条网络路径实际从哪里出去。`;
  const list = rows(data, page, false);
  render(list);
  const count = (s) => list.filter((r) => r.status === s).length;
  const [leaks, broken] = [count('fail'), count('error')];
  status.textContent = [leaks ? `发现 ${leaks} 处泄漏` : '没有发现泄漏', broken ? `${broken} 项连不通` : ''].filter(Boolean).join('，');
  shared = { generatedAt: new Date().toISOString(), subject: data.expected.subject, protection: data.expected.protection || null, rows: rows(data, page, true) };
  copy.disabled = false;
}

document.getElementById('rerun').onclick = () => start();
document.getElementById('copy').onclick = async () => {
  if (!shared) return;
  const file = await window.probe.copy(JSON.stringify(shared, null, 2));
  document.getElementById('status').textContent = `已复制，并保存到 ${file}`;
};
start();
