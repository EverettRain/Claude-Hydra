<div align="center">

<img src="app/src/assets/claude-hydra-icon-256.png" alt="Claude-Hydra" width="140">

# Claude-Hydra

**同时运行多个 Claude 桌面端账号，每个账号拥有独立的数据目录、代理节点与时区。**

基于 Tauri 2（Rust + React）构建，支持 Windows 与 macOS。

![Windows 10 | 11](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D4?style=flat-square&logo=windows)
![macOS](https://img.shields.io/badge/macOS-supported-000000?style=flat-square&logo=apple)
![Tauri 2](https://img.shields.io/badge/Tauri-2.x-24C8DB?style=flat-square&logo=tauri)
![License: MIT](https://img.shields.io/badge/License-MIT-2e7d4a?style=flat-square)

![Claude-Hydra 主界面](docs/images/hydra3-light.png)

</div>

## 目录

- [功能概览](#功能概览)
- [快速开始](#快速开始)
- [代理节点](#代理节点)
- [防泄漏机制](#防泄漏机制)
- [避免直连的使用建议](#避免直连的使用建议)
- [时区同步](#时区同步)
- [用量统计](#用量统计)
- [会话迁移](#会话迁移)
- [平台差异](#平台差异)
- [数据存储与安全](#数据存储与安全)
- [命令行](#命令行)
- [开发与构建](#开发与构建)
- [已知限制](#已知限制)

## 功能概览

| 功能 | 说明 |
| --- | --- |
| 多账号并行 | 通过 `--user-data-dir` 为每个账号提供独立的配置与登录态，可同时打开多个 Claude 窗口 |
| 代理节点 | 每个账号单独配置 HTTP / HTTPS / SOCKS5 节点，支持账号密码、出口检测与换 IP 链接 |
| 防泄漏 | 内置本机中继，DNS、WebRTC、UDP/QUIC、IPv6 全部经节点或被阻止；节点失效时断网而非直连 |
| 时区同步 | 账号时区可跟随代理出口地区、跟随系统或手动指定 |
| 用量统计 | 按账号查看订阅的 5 小时与每周额度 |
| 会话迁移 | 在账号之间复制本地 Claude Code 会话；普通聊天可导出为文本续聊 |
| 系统托盘 | 关闭主窗口后仅保留轻量后台进程，实时显示各账号运行状态 |
| 保持运行 | Claude 因自动更新或注销退出后自动重新打开，手动退出不触发 |
| 进程树回收 | 关闭账号时一并结束其渲染进程及会话创建的 MCP 服务进程 |
| 桌面快捷方式 | 为账号生成快捷方式，毫秒级启动或唤醒对应窗口 |
| 任务栏分组 | Windows 上每个账号使用独立的任务栏分组与配色图标 |

## 快速开始

1. 打开 **Claude-Hydra**，点击右上角 **+ 添加账号**。
2. 在侧边栏填写窗口名称，按需配置代理节点、时区和窗口语言，点击 **确定**。
3. 在弹出的全新 Claude 窗口中登录，即可与其他账号同时在线。
4. 点击账号卡片上的 **启动 / 切换到**（或双击卡片）启动或置顶窗口；右侧菜单可编辑配置、重启、打开数据目录或创建快捷方式。

## 代理节点

在 **添加账号** 或 **编辑账号** 中选择“自定义代理”：

| 字段 | 说明 |
| --- | --- |
| IP 查询渠道 | 检测出口地区与时区所用的服务：ipinfo.io / ipapi.co / ip-api.com，请求经节点发出 |
| 代理检测 | 立即经节点检测出口 IP、国家 / 地区 / 城市与时区 |
| 代理类型 | Socks5、HTTP、HTTPS |
| IP 协议 | 要求的出口 IP 版本（IPv4 / IPv6），启动前校验，不符则不启动 |
| 代理主机 / 端口 | 主机栏支持粘贴 `socks5://主机:端口:账号:密码`、`账号:密码@主机:端口`、`主机:端口:账号:密码`，自动拆分 |
| 代理账号 / 密码 | 选填，最多 200 个字符 |
| 刷新 URL | 服务商提供的换 IP 链接，点击右侧按钮直接请求（不经过节点） |

保存后，新建的连接立即使用新节点；时区与启动参数在重启该账号后生效。

## 防泄漏机制

配置了节点的账号，网络路径为：

```text
Claude 窗口 / 子进程 ──► 127.0.0.1 本机中继（Hydra 后台）──► 代理节点 ──► 互联网
```

Claude 只看得到本机中继地址；节点的账号密码保存在 Hydra 内部，不会出现在 Claude 的启动参数或环境变量中。

| 泄漏途径 | 处理方式 |
| --- | --- |
| DNS | `--host-resolver-rules` 禁止本地解析，域名原样交给节点解析 |
| WebRTC | `--force-webrtc-ip-handling-policy=disable_non_proxied_udp`，不暴露本机或 STUN 地址 |
| UDP / QUIC | `--disable-quic`；中继拒绝 SOCKS5 UDP ASSOCIATE |
| IPv6 | Claude 只连接 127.0.0.1 中继，不直接使用本机 IPv6 |
| 子进程 | Claude Code、MCP 等通过 `HTTP(S)_PROXY`、`ALL_PROXY`、`NODE_USE_ENV_PROXY` 指向同一中继 |
| 节点失效 | 每次启动前经“中继 → 节点”检测出口，失败不启动；运行中节点断开或 Hydra 退出，该账号断网，不回退直连 |

上述开关已用无头 Chrome 实测：WebRTC 候选地址由 3 个降为 0；中继关闭时页面无法加载；禁止本地 DNS 后仍可按域名经节点访问。

> 账号运行期间请保持 Claude-Hydra 在托盘中运行。通过命令行或快捷方式启动时，若后台未运行会自动拉起。

## 避免直连的使用建议

代理节点只在通过 Claude-Hydra 启动账号时生效（主界面、托盘、Hydra 生成的快捷方式、命令行）。直接打开 Claude 时，它会进入**默认账号**，不经过任何节点。Hydra 创建的额外账号无法被直接打开，因此按以下方式使用即可避免账号直连：

1. **默认账号保持未登录。** 如果默认账号曾经登录过，请先在 Claude 中退出登录。之后无论是直接打开 Claude，还是 Claude 自动更新后自行重启，看到的都只是一个未登录的空白窗口，不会带着你的账号用真实 IP 联网。
2. **所有需要代理的账号都在 Hydra 中创建并登录。** 默认账号不建议配置代理节点。
3. **登录时让系统浏览器也走同一节点。** 在账号窗口中选择“使用 Google 登录”等方式时，登录页会在系统浏览器中打开，而系统浏览器不经过 Hydra 的节点。登录前请让浏览器临时使用同一节点（例如浏览器代理扩展，或为该节点单独准备一个浏览器配置文件），登录完成后再恢复。
4. **账号运行期间不要退出 Claude-Hydra。** 退出后已配置节点的账号会断网，不会回退直连，重新打开 Hydra 即可恢复。
5. **留意自行联网的工具。** 不读取代理环境变量、自行直连网络的 MCP 服务或命令行工具可能绕过节点，请优先选用支持 `HTTPS_PROXY` 的工具。

> macOS 上点击 Dock 中的 Claude 时，若已有 Hydra 账号在运行，系统通常会切换到该窗口而非新开默认账号；需要打开默认账号时请从 Hydra 启动。

## 时区同步

| 选项 | 行为 |
| --- | --- |
| 跟随代理 IP（默认） | 每次启动时经节点查询出口时区并应用；查询不到则不启动 |
| 跟随系统 | 不修改时区 |
| 自定义 | 使用指定的 IANA 时区，如 `Asia/Tokyo` |

时区通过 `TZ` 环境变量传给 Claude。macOS 上 Claude 窗口与子进程均生效；Windows 上 Claude 窗口可能仍显示系统时区，仅子进程确定生效。

## 用量统计

在 **用量统计** 页点击 **刷新额度**，按账号读取 Claude 返回的 5 小时与每周额度（包含同账号在其他设备上的用量）。只在点击时联网，同一账号 60 秒内不重复请求；配置了节点的账号经节点查询。

- Windows：使用当前 Windows 用户解密本机登录凭据。
- macOS：首次读取时系统会询问是否允许访问钥匙串中的“Claude Safe Storage”，请选择“允许”。

此接口未公开，可能随 Claude 更新而变化。

## 会话迁移

**Claude Code 会话**：选择来源和目标账号后预览会话（标题、工作目录、最后活动时间、目标中是否已存在），勾选后复制。

1. 迁移前请自行退出来源和目标账号，Hydra 不会自动关闭它们。
2. 目标账号必须已至少有一个 Claude Code 会话，且只登录了一个身份。
3. 只复制会话索引（`claude-code-sessions` 下的 `local_*.json`），原文件保留，同名会话跳过不覆盖。

**普通聊天续聊**：在来源账号中通过“设置 → 隐私 → 导出数据”导出，选择解压后的 `conversations.json`，将所选聊天保存为文本附件，在目标账号的新聊天中附上继续对话。

## 平台差异

| 功能 | Windows | macOS |
| --- | --- | --- |
| 多账号、托盘、保持运行 | ✅ | ✅ |
| 桌面快捷方式 | ✅ `.lnk` | ✅ `.command` |
| 代理节点与防泄漏中继 | ✅（商店版默认账号除外） | ✅ |
| 时区同步 | ⚠️ 子进程生效，窗口待实测 | ✅ |
| 用量统计 | ✅ | ✅ |
| 会话迁移 / 聊天续聊 | ✅ | ✅ |
| 任务栏独立分组 | ✅ | — Dock 按应用分组 |
| 浏览器登录回调 | ✅ | — 请在对应账号窗口内直接登录 |

## 数据存储与安全

| 平台 | 根目录 |
| --- | --- |
| Windows | 存在 D 盘时为 `D:\ClaudeHydra`，否则为 `%LOCALAPPDATA%\ClaudeHydra`；可用环境变量 `CLAUDE_HYDRA_ROOT` 自定义 |
| macOS | `~/Library/Application Support/ClaudeHydra` |

- `settings.json`：账号配置。节点密码在 Windows 上以 DPAPI 加密保存；macOS 上设置文件权限为仅本人可读。
- `profiles\<账号ID>`：额外账号的 Claude 数据目录。
- 默认账号继续使用 Claude 原有的数据目录。

## 命令行

```text
claude-hydra                   打开主窗口
claude-hydra --tray            仅启动到系统托盘
claude-hydra --launch <账号>   启动指定账号；若已在运行则将其窗口置顶
claude-hydra --close <账号>    关闭指定账号的 Claude 进程树
claude-hydra --restart <账号>  重启指定账号的 Claude
claude-hydra --add-account     创建新账号并打开登录窗口
claude-hydra --list            列出所有账号及运行状态
claude-hydra --watch           执行一次“保持运行”检查
claude-hydra --version         打印版本号
```

## 开发与构建

环境要求：Node.js 22+、pnpm 11+、Rust 1.95（见 `app/rust-toolchain.toml`）及对应平台的 Tauri 2 构建依赖。

在 `app/` 目录下执行：

```bash
pnpm install --frozen-lockfile
pnpm tauri dev          # 启动开发调试
pnpm tauri build        # 构建安装包
pnpm typecheck          # TypeScript 类型检查
pnpm build:renderer     # 构建前端资源
pnpm test:rust          # 运行 Rust 单元测试
```

需要网络或本机 Chrome 的集成测试默认跳过，可手动运行：

```bash
cd app/src-tauri && HYDRA_TEST_CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" cargo test -- --ignored
```

设计与验证记录见 [`docs/`](docs/)：[环境设计评估](docs/environment-design.md)、[会话迁移验证](docs/desktop-session-migration.md)、[计划任务迁移](docs/scheduled-task-migration.md)。

## 已知限制

- 防泄漏覆盖 Claude 的 Chromium 网络栈与读取代理环境变量的子进程；不读取代理变量、自行直连的子进程，以及在系统浏览器中打开的登录页无法拦截。需要系统级保证时，请配合防火墙或独立虚拟机。
- 独立数据目录只隔离本地登录数据，不修改设备指纹，也不保证账号之间不被关联。
- 使用代理节点时，额外启动参数中不能再包含 `--proxy-server`、`--host-resolver-rules` 等代理开关。
- 商店版（MSIX）的默认账号由系统启动，无法应用代理节点、时区与额外参数。
- 用量统计与会话迁移依赖 Claude 桌面端未公开的数据格式，可能随其更新失效。

## 许可证

本项目基于 [MIT License](LICENSE) 开源。
