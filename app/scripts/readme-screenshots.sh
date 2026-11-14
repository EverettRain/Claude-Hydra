#!/bin/bash
# README screenshots from the installed Claude-Hydra.app, on fictional demo data.
#
#   app/scripts/readme-screenshots.sh [output folder]      (default: docs/images)
#
# macOS only. The app runs with HOME set to a demo folder (/Users/Shared/HydraDemo), so it reads
# demo settings, accounts and sessions and never sees your own. Pages are opened by pressing the
# app's buttons through macOS accessibility; when a button cannot be found the script stops and
# asks you to click it yourself, then carries on. Windows are captured with `screencapture`, which
# gives the native frame, traffic lights and shadow on a transparent background.
#
# Needs, for the terminal app this runs in: Accessibility (to press buttons) and Screen Recording
# (to capture another app's window), under System Settings → Privacy & Security.
# Environment: HYDRA_APP (default /Applications/Claude-Hydra.app), HYDRA_DEMO (demo folder),
# MANUAL=1 to open every page by hand.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
APP=${HYDRA_APP:-/Applications/Claude-Hydra.app}
DEMO=${HYDRA_DEMO:-/Users/Shared/HydraDemo}
OUT=$(mkdir -p "${1:-$REPO/docs/images}" && cd "${1:-$REPO/docs/images}" && pwd)
WORK=$(mktemp -d)
PID=""

say() { printf '\033[1m%s\033[0m\n' "$*"; }
fail() { printf '\033[31m%s\033[0m\n' "$*" >&2; exit 1; }

[ "$(uname)" = Darwin ] || fail "只能在 macOS 上运行。"
[ -d "$APP" ] || fail "找不到 $APP。可以用 HYDRA_APP=/path/to/Claude-Hydra.app 指定。"
command -v python3 >/dev/null || fail "需要 python3（xcode-select --install 会一并装上）。"
EXE="$APP/Contents/MacOS/$(/usr/bin/plutil -extract CFBundleExecutable raw -o - "$APP/Contents/Info.plist")"

cleanup() {
  [ -n "$PID" ] && kill "$PID" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

# ---- the real Hydra has to be out of the way: only one instance runs at a time ----------

if pgrep -f "$EXE" >/dev/null; then
  say "Claude-Hydra 正在运行。请从菜单栏图标选择「退出」（截图结束后再打开）。"
  say "注意：退出期间，通过 Hydra 中继上网的账号会断网。"
  while pgrep -f "$EXE" >/dev/null; do sleep 1; done
fi

# ---- demo data --------------------------------------------------------------------------

say "生成示例数据：$DEMO"
rm -rf "$DEMO"
python3 - "$DEMO" <<'PY'
import json, os, random, sys, time

demo = sys.argv[1]
random.seed(7)
now = int(time.time() * 1000)
HOUR, DAY = 3600_000, 86400_000
support = f"{demo}/Library/Application Support"
root = f"{support}/ClaudeHydra"

def write(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(data if isinstance(data, str) else json.dumps(data, ensure_ascii=False, indent=1))

def iso(ms):
    ms = int(ms)
    return time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(ms / 1000)) + f".{ms % 1000:03d}Z"

def touch(path, ms):
    os.utime(path, (ms / 1000, ms / 1000))

# Folders the containers share, with a few subfolders to browse.
code = f"{demo}/code"
for sub in ["shop-api/src", "shop-api/tests", "shop-api/migrations", "web/src", "web/public", "game/little-lander", "game/sandbox"]:
    os.makedirs(f"{code}/{sub}", exist_ok=True)

exit_jp = {"ip": "203.0.113.24", "country": "JP", "region": "Tokyo", "city": "Tokyo", "timezone": "Asia/Tokyo", "checkedAt": now - 2 * HOUR}
exit_us = {"ip": "198.51.100.77", "country": "US", "region": "California", "city": "Los Angeles", "timezone": "America/Los_Angeles", "checkedAt": now - HOUR}
node = lambda kind, host, port: {"kind": kind, "host": host, "port": port, "ipVersion": "ipv4", "geoChannel": "ipinfo"}
settings = {
    "accounts": {
        "work": {"label": "工作", "environment": "tokyo", "keepRunning": True},
        "research": {"label": "研究", "environment": "uswest"},
        "ctr-1": {"label": "美国容器", "environment": "uswest", "container": {"mounts": [
            {"source": f"{code}/shop-api", "name": "shop-api"}, {"source": f"{code}/web", "name": "web"}]}},
        "ctr-2": {"label": "日本容器", "environment": "tokyo", "container": {"mounts": [{"source": f"{code}/game", "name": "game"}]}},
    },
    "environments": {
        "tokyo": {"name": "东京住宅", "node": node("socks5", "jp-res.example.net", 1080), "timezone": "proxy", "locale": "ja-JP",
                  "lastCheck": {"exit": exit_jp, "quality": "住宅", "risk": "低"}},
        "uswest": {"name": "美西节点", "node": node("https", "us-west.example.net", 443), "timezone": "proxy", "locale": "en-US",
                   "lastCheck": {"exit": exit_us, "quality": "机房", "risk": "低"}},
        "direct": {"name": "本机直连 · 英文界面", "timezone": "system", "locale": "en-US"},
    },
    "requireEnvironment": True,
    "terminal": "iterm",
}
if not os.path.isdir("/Applications/Claude.app"):
    # A stand-in, only so the desktop accounts list; nothing is ever launched from it.
    stub = f"{demo}/Applications/Claude.app"
    write(f"{stub}/Contents/MacOS/Claude", "#!/bin/sh\n")
    write(f"{stub}/Contents/Info.plist", '<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict>'
          '<key>CFBundleIdentifier</key><string>com.anthropic.claudefordesktop</string></dict></plist>')
    settings["claudePath"] = stub
write(f"{root}/settings.json", settings)

# Desktop accounts: the default profile and two of Hydra's, signed in, last used a while ago.
for path, ago in [(f"{support}/Claude", 25 * 60e3), (f"{root}/profiles/work", HOUR), (f"{root}/profiles/research", 2 * HOUR)]:
    write(f"{path}/config.json", {"windowSizeWasSignedIn": True})
    touch(f"{path}/config.json", now - ago)

# ---- Claude Code sessions -----------------------------------------------------------------

def project(cwd):
    return "".join(c if c.isascii() and c.isalnum() else "-" for c in cwd)

class Session:
    def __init__(self, folder, cwd, title):
        self.folder, self.cwd, self.title, self.rows, self.n = folder, cwd, title, [], 0
        self.id = "%08x-%04x-4%03x-a%03x-%012x" % (random.getrandbits(32), random.getrandbits(16), random.getrandbits(12), random.getrandbits(12), random.getrandbits(48))

    def row(self, kind, at, **fields):
        self.n += 1
        self.rows.append({"type": kind, "uuid": f"{self.id[:8]}-{self.n}", "sessionId": self.id, "cwd": self.cwd, "timestamp": iso(at), **fields})

    def ask(self, at, text):
        self.row("user", at, message={"role": "user", "content": text})

    def answer(self, at, content, scale=1.0, model="claude-opus-5-5"):
        usage = {"input_tokens": int(18e3 * scale), "output_tokens": int(42e3 * scale),
                 "cache_creation_input_tokens": int(260e3 * scale), "cache_read_input_tokens": int(3.1e6 * scale)}
        self.row("assistant", at, requestId=f"req_{self.id[:8]}{self.n}", message={"id": f"msg_{self.id[:8]}{self.n}", "role": "assistant", "model": model, "content": content, "usage": usage})

    def result(self, at, tool_id, text):
        self.row("user", at, message={"role": "user", "content": [{"type": "tool_result", "tool_use_id": tool_id, "content": text}]})

    def save(self, last):
        path = f"{self.folder}/projects/{project(self.cwd)}/{self.id}.jsonl"
        lines = [json.dumps(r, ensure_ascii=False) for r in self.rows] + [json.dumps({"type": "ai-title", "aiTitle": self.title, "sessionId": self.id}, ensure_ascii=False)]
        write(path, "\n".join(lines) + "\n")
        touch(path, last)

titles = {
    "/workspace/shop-api": ["排查支付回调重复入账", "补充库存服务的集成测试", "数据库迁移脚本评审", "优惠券过期任务改为批量处理", "订单导出改用流式写入",
                            "接入短信验证码限流", "清理废弃的旧版接口", "商品搜索加上拼音匹配", "退款流程补充状态机", "日志改为结构化输出"],
    "/workspace/web": ["首页加载性能优化", "深色模式与配色变量整理", "表单校验改用 zod", "购物车动画卡顿", "图片懒加载与占位图",
                       "移动端底部导航适配", "国际化文案抽取", "埋点改为统一上报", "错误边界与重试", "组件库升级"],
    "/workspace/game/little-lander": ["沙盒气温与衣物系统", "存档格式迁移", "NPC 对话分支整理", "本地化键值库", "关卡难度曲线调整"],
}

def history(folder, cwds, months, scale):
    """Weekly sessions over `months`, used more in later weeks, with activity in waking hours."""
    start = now - months * 30 * DAY
    weeks = int((now - start) / (7 * DAY))
    for w in range(weeks):
        t0 = start + w * 7 * DAY
        cwd = cwds[w % len(cwds)]
        names = titles[cwd]
        # Titles run back from the newest week, so the sessions seen first are all different.
        k = (weeks - 1 - w) // len(cwds)
        s = Session(folder, cwd, names[k % len(names)] + ("" if k < len(names) else "（续）"))
        ramp = 0.25 + 0.75 * w / max(1, weeks - 1)
        quiet = 0.15 if w % 9 == 4 else 1
        last = t0
        for h in range(7 * 24):
            at = t0 + h * HOUR
            if at > now - 2 * DAY:
                break
            local = time.localtime(at / 1000)
            busy = (0.55 if 10 <= local.tm_hour <= 23 else 0.08) * (0.45 if local.tm_wday >= 5 else 1) * ramp * quiet
            if random.random() > busy:
                continue
            if random.random() < 0.15 or s.n == 0:
                s.ask(at, "继续，按刚才的方案改。" if s.n else s.title)
            s.answer(at + 60e3, [{"type": "text", "text": "好的。"}], scale * (0.4 + random.random() * 1.6),
                     "claude-opus-5-5" if random.random() < 0.82 else "claude-sonnet-5-5")
            last = at
        if s.n:
            s.save(last)

def conversation(folder):
    """The session shown in the conversation view: a question, tools, a table and a question card."""
    s = Session(folder, "/workspace/shop-api", "订单接口改为分页查询")
    t = now - 25 * 60e3
    path = "/workspace/shop-api/src/orders/service.ts"
    s.ask(t, "订单列表接口在数据量大的时候很慢，帮我改成分页查询，前端已经能传 page 和 pageSize 了。")
    s.answer(t + 20e3, [{"type": "thinking", "thinking": "先看现在的查询是怎么写的，再看看有没有现成的分页工具函数。", "signature": "demo"},
                        {"type": "tool_use", "id": "t1", "name": "Grep", "input": {"pattern": "listOrders", "path": "src"}}])
    s.result(t + 25e3, "t1", "src/orders/service.ts:42:export async function listOrders(userId: string) {\nsrc/orders/routes.ts:18:  const orders = await listOrders(req.user.id);")
    s.answer(t + 30e3, [{"type": "tool_use", "id": "t2", "name": "Read", "input": {"file_path": path}}])
    s.result(t + 35e3, "t2", "42  export async function listOrders(userId: string) {\n43    return db.order.findMany({ where: { userId }, orderBy: { createdAt: \"desc\" } });\n44  }")
    s.answer(t + 60e3, [{"type": "text", "text": "现在的 `listOrders` 一次取出用户的全部订单，没有分页，数据一多就慢。我按 `page` / `pageSize` 改成分页，并一起返回总数，前端可以据此显示页码："},
                        {"type": "tool_use", "id": "t3", "name": "Edit", "input": {"file_path": path,
                         "old_string": "export async function listOrders(userId: string) {\n  return db.order.findMany({ where: { userId }, orderBy: { createdAt: \"desc\" } });",
                         "new_string": "export async function listOrders(userId: string, page = 1, pageSize = 20) {\n  const where = { userId };\n  const [items, total] = await Promise.all([\n    db.order.findMany({ where, orderBy: { createdAt: \"desc\" }, skip: (page - 1) * pageSize, take: pageSize }),\n    db.order.count({ where }),\n  ]);\n  return { items, total, page, pageSize };"}}])
    s.result(t + 65e3, "t3", "文件已更新。")
    s.answer(t + 70e3, [{"type": "tool_use", "id": "t4", "name": "Bash", "input": {"command": "npm test -- orders", "description": "运行订单相关测试"}}])
    s.result(t + 90e3, "t4", " PASS  src/orders/service.test.ts\n  ✓ 第一页返回 20 条 (38 ms)\n  ✓ 超出范围的页返回空列表 (12 ms)\n\nTests: 2 passed, 2 total")
    s.answer(t + 100e3, [{"type": "text", "text": "改好了，测试通过。接口的变化如下：\n\n| 参数 | 默认值 | 说明 |\n|---|---|---|\n| `page` | 1 | 页码，从 1 开始 |\n| `pageSize` | 20 | 每页条数，最大 100 |\n\n返回值从数组改成了 `{ items, total, page, pageSize }`。**前端需要相应调整**：列表数据改读 `items`，用 `total` 计算页数。"},
                         {"type": "tool_use", "id": "q1", "name": "AskUserQuestion", "input": {"questions": [{"header": "上限", "question": "pageSize 超过 100 时怎么处理？", "multiSelect": False,
                          "options": [{"label": "自动改为 100", "description": "不报错，按最大值返回"}, {"label": "返回 400 错误", "description": "让调用方明确知道参数不合法"}]}]}}])
    s.result(t + 140e3, "q1", 'Your questions have been answered: "pageSize 超过 100 时怎么处理？"="自动改为 100". You can now continue.')
    s.answer(t + 150e3, [{"type": "text", "text": "好的，超过 100 时按 100 处理，已经补上对应的测试。"}])
    s.save(t + 150e3)

def recent(folder, cwd, title, ago, turns):
    s = Session(folder, cwd, title)
    t = now - ago
    for i in range(turns):
        s.ask(t + i * 600e3, title if i == 0 else "继续。")
        s.answer(t + i * 600e3 + 60e3, [{"type": "text", "text": "好的。"}], 0.6)
    s.save(t + turns * 600e3)

def account(folder, email):
    write(f"{folder}/.claude.json", {"oauthAccount": {"accountUuid": "00000000-0000-4000-8000-000000000000", "emailAddress": email, "planDisplayName": "Max"}})
    write(f"{folder}/CLAUDE.md", "# 全局记忆\n\n- 回复使用中文。\n- 修改代码时跟随项目已有风格。\n")

us, jp = f"{root}/containers/ctr-1/claude", f"{root}/containers/ctr-2/claude"
account(us, "us@example.com")
account(jp, "jp@example.com")
history(us, ["/workspace/shop-api", "/workspace/web"], 10, 1.0)
history(jp, ["/workspace/game/little-lander"], 6, 0.55)
recent(us, "/workspace/web", "首页首屏改为服务端渲染", 3 * HOUR, 9)
recent(us, "/workspace/shop-api", "排查订单状态不同步", 5 * HOUR, 12)
conversation(us)
recent(f"{demo}/.claude", f"{code}/web", "整理组件目录结构", DAY, 6)

PY

# ---- driving the app --------------------------------------------------------------------

cat > "$WORK/ui.js" <<'JS'
// osascript -l JavaScript ui.js <command> <pid> [arguments]
//   window <pid>                         id of the app's main window, "" until it is on screen
//   front <pid>                          bring the app forward
//   press <pid> <text> [index] [exact]   press the index-th control whose label contains (or equals) text
//   label <pid> <text>                   label of the first control whose label contains text
//   dump <pid>                           every control and its label, to find names when a press fails
ObjC.import("CoreGraphics");
function run(argv) {
  const [cmd, pid, text, index, exact] = argv;
  if (cmd === "window") {
    const list = ObjC.deepUnwrap(ObjC.castRefToObject($.CGWindowListCopyWindowInfo($.kCGWindowListOptionOnScreenOnly, 0))) || [];
    const w = list.find((w) => w.kCGWindowOwnerPID == pid && w.kCGWindowLayer == 0 && w.kCGWindowBounds.Width > 400);
    return w ? String(w.kCGWindowNumber) : "";
  }
  const proc = Application("System Events").processes.whose({ unixId: Number(pid) })[0];
  if (cmd === "front") { proc.frontmost = true; return "ok"; }
  const roles = ["AXButton", "AXDisclosureTriangle", "AXLink", "AXRadioButton", "AXTab"];
  const label = (e) => ["title", "name", "description"].map((k) => { try { return String(e[k]() || ""); } catch (x) { return ""; } }).filter(Boolean).join(" ");
  const controls = proc.windows[0].entireContents().filter((e) => { try { return roles.includes(e.role()); } catch (x) { return false; } });
  if (cmd === "dump") return controls.map((e) => e.role() + "  " + label(e)).join("\n");
  const hits = controls.filter((e) => (exact ? label(e).split(" ").includes(text) : label(e).includes(text)));
  if (cmd === "label") return hits.length ? label(hits[0]) : "";
  const hit = hits[Number(index || 0)];
  if (!hit) return "missing";
  hit.actions.byName("AXPress").perform();
  return "ok";
}
JS
ui() { osascript -l JavaScript "$WORK/ui.js" "$@" 2>/dev/null || true; }

# Presses a control; when it is not there, asks for the click instead. $2 says what to click.
press() {
  local text=$1 hint=$2 index=${3:-0} exact=${4:-}
  if [ "${MANUAL:-0}" != 1 ] && [ "$(ui press "$PID" "$text" "$index" "$exact")" = ok ]; then sleep 1.2; return; fi
  ui front "$PID"
  printf '  请在 Claude-Hydra 窗口里%s，然后回到这里按回车… ' "$hint"
  read -r _
}

# Cycles the theme button until it shows the wanted mode (浅色 / 深色 / 跟随系统).
theme() {
  for _ in 1 2 3 4; do
    [ "$(ui label "$PID" "主题：")" = "主题：$1" ] && return
    ui press "$PID" "主题：" >/dev/null; sleep 0.6
  done
  press "主题：" "点右上角的主题按钮，切到「$1」"
}

shot() {
  ui front "$PID"; sleep 1.5
  /usr/sbin/screencapture -x -l "$WID" "$OUT/$1.png"
  say "  已保存 $1.png"
}

say "启动 Claude-Hydra（示例数据）"
HOME="$DEMO" CLAUDE_HYDRA_SUPPORT_DIR="$DEMO/Library/Application Support/ClaudeHydra" "$EXE" >/dev/null 2>&1 &
PID=$!
WID=""
for _ in $(seq 1 60); do WID=$(ui window "$PID"); [ -n "$WID" ] && break; sleep 0.5; done
[ -n "$WID" ] || fail "没等到 Claude-Hydra 的窗口。如果它刚弹出权限请求，授权后重新运行。"
if [ "$(ui press "$PID" "账号" 0 exact)" != ok ]; then
  say "无法操作 Claude-Hydra 的界面：请在「系统设置 → 隐私与安全性 → 辅助功能」里允许当前终端，或用 MANUAL=1 手动切换页面。"
fi
sleep 2
ORIGINAL=$(ui label "$PID" "主题：" | sed 's/^主题：//')

say "开始截图 → $OUT"
theme 浅色
press "账号" "点左侧「账号」" 0 exact;            shot accounts-light
theme 深色;                                        shot accounts-dark
theme 浅色
press "环境配置" "点左侧「环境配置」" 0 exact;     shot environments
press "编辑" "点「东京住宅」右侧的「编辑」"        # environments are listed by name
press "自定义代理" "点对话框里的「自定义代理」，让名称输入框失去焦点"
shot environment-edit
press "取消" "关闭对话框"
press "仪表板" "点左侧「仪表板」" 0 exact;         sleep 2; shot dashboard
press "容器" "点左侧「容器」" 0 exact
press "打开" "点「美国容器」右侧的「打开」" 1      # 日本容器 comes first
press "shop-api" "展开项目「shop-api」";           shot containers
press "配置管理" "点左侧「配置管理」" 0 exact
press "美国容器" "在位置列表里点「美国容器」";     shot config
press "订单接口改为分页查询" "点会话「订单接口改为分页查询」"
press "Edit、Bash" "展开「使用了 2 个工具 Edit、Bash」那一块"; shot chat
press "返回" "点「返回」"
[ -n "$ORIGINAL" ] && theme "$ORIGINAL"

say "完成。截图在 $OUT；示例数据留在 $DEMO，可以删除。"
say "记得重新打开你自己的 Claude-Hydra。"
