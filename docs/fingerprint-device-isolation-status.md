# 指纹 / 设备隔离：当前状态

日期：2026-11-12（第二轮更新）
状态：**部分实现**（理解 A 已做，理解 B 未做）

## 结论

"指纹 / 设备隔离"按理解 A 实现了：开启 Sandboxie 隔离的账号，在沙箱里看到的机器 ID、硬件配置 ID、卷序列号、MAC 地址和主机名都换成了这个账号自己的值，与主机和其他账号都不同。第二轮把这几项从"常见 API 生效"推进到"同一个值无论从哪条路读都一致"：注册表与卷序列号改在 NT 层改写，Node（即 Claude Code）读到的主机名、卷号、MAC 都已实测为派生值，账号的 Claude Code 设备 ID 也不再在沙箱之间共用。

理解 B（虚拟机级隔离）没有做，也不打算用 Sandboxie 冒充。Sandboxie 是进程沙箱，不是虚拟机：它不虚拟化内核，WMI 由盒外进程应答，因此仍有一批系统标识会漏出主机的值，见「没有覆盖到的部分」。

## 这项任务指什么

需求原话是"加入指纹 / 设备隔离"，对标 [Claude-Cage](https://github.com/Lee-xeo/Claude-Cage)。它可以有两种理解：

| 理解 | 内容 | 状态 |
|---|---|---|
| A. 生成设备标识 | 给每个虚拟沙箱生成机器 ID、设备 ID、硬件标识、MAC、主机名等，减少平台风控在多个账号在同一台设备上 | 已实现，见下文 |
| B. 加强环境隔离 | 让账号运行在与主机隔离的环境里（虚拟机一类），自然拥有独立的系统环境 | 未做；Sandboxie 不能替代虚拟机 |

## Claude-Cage 实际做了什么

依据它的 README（没有查看它的 `docs/` 目录）：

- 它的"指纹设置"只包含**时区和语言**，并支持随代理 IP 自动同步。
- 它的设备隔离靠**虚拟机**：Claude 跑在虚拟机里，Cage 接管虚拟机内的全部网络流量。
- README 没有伪造机器 ID、设备 ID、硬件标识、MAC 或主机名。

也就是说，Cage 做的是理解 B，Hydra 现在做的是 A。

## Hydra 目前已有的相关能力

| 项目 | Claude-Cage | Claude-Hydra |
|---|---|---|
| 时区随代理 IP | 有 | 有（环境配置里"跟随代理 IP"） |
| 窗口语言 | 有 | 有（环境配置里"窗口语言"） |
| 网络全部走代理 | 虚拟机级接管 | 本地中继；开启 Sandboxie 时绕过代理的进程会断网，不开沙箱时忽略代理设置的程序仍可能直连 |
| 与主机的数据隔离 | 整个虚拟机 | 账号数据目录、可选 Sandboxie 沙箱、"独立 Code"目录（开沙箱时强制） |
| 独立的系统标识 | 虚拟机自带 | 开启 Sandboxie 时有，未开沙箱时与主机相同 |

## 实现方式

每个账号的标识由账号 ID 确定性地派生：`Sha256("hydra identity" || 标签 || 账号散列)`。同一个账号每次启动看到的都是同一台"设备"，不同账号之间互不相同，也不随重启漂移。

| 标识 | 取值形态 | 改写位置 |
|---|---|---|
| 机器 ID（MachineGuid） | 裸小写 GUID | `NtQueryValueKey` / `NtEnumerateValueKey`，键尾 `\Microsoft\Cryptography` |
| 硬件配置 ID（HwProfileGuid） | 带花括号大写 GUID | 同上，键尾 `\Hardware Profiles\0001` |
| 主机名 | `DESKTOP-` 加 7 位大写字母数字 | `GetComputerName[Ex]W/A`；注册表 `Tcpip\Parameters\Hostname`（同上两个 NT 调用）；账号域名 `LookupAccountSidW/A`、`LookupAccountNameW/A`、`GetUserNameExW/A(NameSamCompatible)`；环境变量 `COMPUTERNAME`、`USERDOMAIN`、`USERDOMAIN_ROAMINGPROFILE`、`LOGONSERVER` |
| 卷序列号 | 8 位十六进制，每个卷各一个 | `NtQueryVolumeInformationFile(FileFsVolumeInformation)` 与 `NtQueryInformationFile(FileIdInformation)` |
| MAC 地址 | 本地管理、单播；前 3 字节按账号固定，后 3 字节按网卡 GUID 派生 | `GetAdaptersAddresses`、`GetAdaptersInfo`、`GetIfTable`、`GetIfEntry`、`GetIfTable2[Ex]`、`GetIfEntry2[Ex]` |

### 为什么改在 NT 层

第一轮挂的是 `RegQueryValueExW`、`GetVolumeInformationW` 这类 Win32 入口。审阅发现同一个值还有别的路：

- `reg query <键>`（不带 `/v`）走 `RegEnumValueW → NtEnumerateValueKey`，原来会把主机的 MachineGuid 原样列出来；`RegQueryValueExA`、`RegGetValueA` 也没挂。
- Node 的 `fs.stat().dev`、`GetFileInformationByHandle`、`GetVolumeInformationByHandleW` 都不经过 `GetVolumeInformationW`，原来拿到的是主机卷号。Claude Code 就是 Node 程序。

这些入口最终都落到同一个 NT 调用，所以现在只在那一层改写：注册表值在 `NtQueryValueKey` / `NtEnumerateValueKey` 里按键名 + 值名应答（按调用方请求的信息类别和缓冲区大小返回系统同样的状态码与长度，"先问长度再取值"的两段式调用照常工作）；卷号在 `NtQueryVolumeInformationFile` 里改写。`GetVolumeInformationW/A` 的单独挂钩已删除，避免同一个值被改两次。

### 卷序列号：按真实卷号派生

NT 层只知道"是哪个卷"，不知道调用方用的路径，所以派生值以该卷的**真实序列号**为键：`派生 = derive(账号键, 'V', 真实序列号)`。好处是所有通往同一卷的路都一致——盘符、`NULL`（当前盘）、挂在 `C:\mnt\data` 上的卷、打开的文件句柄——而且挂载点卷与 `C:` 自然分开。第一轮"按盘符归一，挂载点折成 `C:`"的限制和"传 `NULL` 得到另一个值"的不一致都随之消失。

64 位卷号（`FileIdInformation`）的低 32 位与 `vol` 显示的值一致，高 32 位另行派生，与 NTFS 的实际关系相同。

`derive` 是带账号键的 FNV-1a 加 murmur3 收尾混淆，Rust（`identity.rs::derive`）与 C（`idguard.c::derive`）逐字节一致。用带键的非线性混淆而不是"基值异或"，是因为异或方案下同一台机器上任何账号的 `serial(C) ^ serial(D)` 都相等，可以据此把账号串起来，甚至认出 Hydra 本身；单元测试 `volume_relations_do_not_link_accounts` 钉住这一点，`derive_matches_the_guard` 钉住一组在真实沙箱里观察到的值（主机 `04EF-9568` → 账号 `hydra-identity-test` 的 `89A3-60FA`），任何一侧改动都会在离线单元测试里失败。

### MAC：按网卡 GUID 派生

第一轮按网卡在列表中的**序号**派生，有两个问题：`GetAdaptersAddresses` 与 `GetAdaptersInfo` 列出的网卡集合不同，同一块网卡在两个接口里序号不同、MAC 也不同；插拔 VPN / 虚拟网卡后序号整体错位，所有账号的 MAC 一起变。现在后 3 字节由 `derive(账号 MAC, 'M', 网卡 GUID)` 得出：同一块网卡在所有接口、所有重启之间都是同一个地址。实测 `ipconfig` 与 Node 对 VMnet1 报出同一个 `AA-6C-7B-7F-CD-F7`。只改长度为 6 的物理地址，隧道等其他长度的地址保持原样。

### 主机名：连带的几个出口

主机名不只出现在 `GetComputerName` 里：

- 不在域里的机器上，`USERDOMAIN`、`USERDOMAIN_ROAMINGPROFILE` 等于计算机名，`LOGONSERVER` 是 `\\计算机名`。启动器现在把它们和 `COMPUTERNAME` 一起换掉（`identity::host_name_env`）；在域里时它们是域与域控，所有账号本来就共用，保持不变。
- 本地账号的"域"就是计算机名：`whoami` 打印 `主机名\用户`，.NET 的 `UserDomainName` 也这样取。`LookupAccountSid/Name`、`GetUserNameEx(NameSamCompatible)` 现在把等于主机名的域换成派生名，缓冲区不够时按系统的方式报 `ERROR_INSUFFICIENT_BUFFER` / `ERROR_MORE_DATA` 和所需长度。

### Claude Code 自己的设备 ID

Claude Code 的 `userID` 和遥测标识存在 `~/.claude.json`。第一轮里"独立 Code"默认关闭，沙箱会把主机的 `~/.claude`、`~/.claude.json*` 开放进去，于是每个沙箱的 MachineGuid 各不相同，Claude Code 设备 ID 却是同一个——这比完全不伪装更显眼。现在只要环境开了沙箱隔离，"独立 Code"就强制开启（`Environment::keeps_code_separate`），界面上的开关同时锁定并说明原因。已有账号下次启动时会改用自己的 Claude Code 目录，需要在该账号里重新登录一次 Claude Code。

### 其他配套

- 防护 DLL 对存在转发关系的函数在两个模块上都挂（`kernel32`/`kernelbase`、`advapi32`/`sechost`、`secur32`/`sspicli`），但同一地址只挂一次：转发导出经 `GetProcAddress` 会解析到同一个地址，重复挂钩会叠在自己身上。
- 五项 `Hydra*` 值由 Rust 侧一次写全；缺 `HydraMachineGuid` 时防护 DLL 整体不启用。
- `InjectDll` / `InjectDll64` 每个值只当一个 DLL 路径，多个 DLL 要重复写多行；`set_injection` 先清空再逐个 `append`，回归测试 `every_guard_is_its_own_injection_line` 守着。DLL 文件名带内容哈希，手工改 ini 追加新路径而不清空旧项，会继续加载旧一代 DLL——判据是 `SbieIni.exe query <box> InjectDll64` 里的文件名必须是 `sandbox\` 目录里当前那一代。
- **发布版必须带 32 位防护**：缺 x86 MSVC 工具链时，`build.rs` 在 release 构建里直接失败（debug 构建仍只警告）。否则盒内所有 32 位程序会报主机标识，也不受中继端口限制。

代码位置：`app/src-tauri/src/identity.rs`（派生）、`app/src-tauri/src/idguard.c`（改写）、`app/src-tauri/src/sandbox.rs`（写入沙箱配置与注入）、`app/src-tauri/src/launcher.rs`（环境变量）、`app/src-tauri/src/environment.rs`（强制独立 Code）。

## 已验证

2026-11-12 第二轮，本机 Windows + Sandboxie-Plus，账号 `hydra-identity-test`（沙箱 `Hydra_e32311b88f`）。主机同一时刻：MachineGuid `9ce867b7-…`、主机名 `LAPTOP-UUCIGDOF`、C: 卷号 `04EF-9568`、首块网卡 `90-2E-16-6A-EF-19`。

集成测试 `CLAUDE_HYDRA_ROOT=<scratch> cargo test --lib sandboxie_identity -- --ignored --nocapture` 在盒内执行以下命令并逐条断言，全部通过（`<scratch>` 必须用反斜杠写成 Windows 路径，否则 `OpenFilePath` 匹配不上，探测输出会留在盒内，测试报 "boxed probe did not finish"）：

| 读法 | 盒内结果 |
|---|---|
| `reg query …\Cryptography /v MachineGuid` | `70c0f527-4f99-4023-80aa-87230068f939` |
| `reg query …\Cryptography`（**枚举**，不带 `/v`） | 同上；输出中不含主机值（第一轮会泄漏） |
| `reg query …\Hardware Profiles\0001 /v HwProfileGuid` | 派生值 |
| `hostname` | `DESKTOP-F1JX9W6` |
| `whoami` | `desktop-f1jx9w6\24779`（第一轮是主机名） |
| `vol C:` / `vol D:` | 各自的派生值，互不相同，不等于主机值 |
| `ipconfig /all` | 全部网卡 `AA-6C-7B-…`，互不相同 |
| Node `os.hostname()` | `DESKTOP-F1JX9W6` |
| Node `fs.statSync('C:/').dev` / `'D:/'` | `2309185786`（= `0x89A360FA`）/ D: 的派生值，与 Rust 侧计算一致 |
| Node `os.networkInterfaces()` 的 MAC | 全部 `aa:6c:7b:…`；同一网卡与 `ipconfig` 报的相同 |

另在盒内用 **32 位** `C:\Windows\SysWOW64\cmd.exe` 手工复核：MachineGuid（含枚举）、`hostname`、`whoami` 均为派生值，`vol C:` 为 `89A3-60FA`，与 64 位 Node 报的同一个值一致，说明 32 位防护已加载且两种位数的规则相同。

同一次运行在盒外读 MachineGuid 仍是主机原值：防护只作用于盒内进程。全部 64 个单元测试与 `every_guard_is_its_own_injection_line` 同样通过。

启动器在账号进程没进沙箱、或防护 DLL 没加载时会关掉它并报错（`launcher.rs` 的 `confirm_boxed`），所以沙箱配置写错不会静默失效。

## 没有覆盖到的部分

先区分两类：**盒外应答**的（防护 DLL 进不去，只能上虚拟机）和**能改但有意不改**的（改了会与盒外应答的值互相矛盾）。

盒外应答，值与主机一致：

- **WMI**：`wmic csproduct get uuid`、`wmic baseboard/bios get serialnumber`、`wmic computersystem get name`、`Get-CimInstance` 都由盒外的 `WmiPrvSE.exe` 应答。`getmac.exe` 也属此类——它查询 `Win32_NetworkAdapter`，所以在盒内仍打印主机 MAC（已实测），这不是漏挂的 API。
- **DHCPv6 DUID**：`ipconfig /all` 的 DUID 行由盒外提供，尾部是主机首块网卡的地址。

能在进程内改，但有意不改：

- **SMBIOS 固件表**（`GetSystemFirmwareTable`）、**磁盘序列号**（`IOCTL_STORAGE_QUERY_PROPERTY`）、`GetSystemInfo` / `NtQuerySystemInformation` 里的硬件信息。这些调用在进程内执行，技术上可以挂钩改写（第一轮文档把它们归为"Sandboxie 的能力边界"，这个说法不准确）。不改的原因是：同样的主板 UUID、序列号、磁盘序列号也能经 WMI 读到，而 WMI 改不了；进程内报派生值、WMI 报主机值，同一台"机器"给出两套答案，比统一报主机值更可疑。DUID 当初也是这样试过后撤回的。要改它们，前提是先让盒内的 WMI 失效或同样被改写。
- **用户名与 SID**：`USERNAME`、`USERPROFILE`、`GetUserNameW`、`whoami /user` 的 SID（含机器 SID `S-1-5-21-…`）在每个沙箱里都一样。用户名出现在几乎所有路径里（`C:\Users\<名>\…`，包括账号数据目录和当前目录），只改 API 而路径不变，会造出明显自相矛盾的环境；SID 遍布令牌、注册表 `HKU\<SID>` 与文件 ACL，无法在进程内一致地替换。这是同一 Windows 用户下运行多账号的固有边界。
- **卷创建时间、卷对象 ID**（`FileFsVolumeInformation.VolumeCreationTime`、`FileFsObjectIdInformation`）、`NtQueryMultipleValueKey`：较少被用作指纹，本轮没有改。
- **理解 B 本身**：共用内核，CPU 型号、核心数、内存、显卡、驱动版本完全一致。

另外，账号浏览器（WebView2）的 Canvas、WebGL、GPU、字体、屏幕等**网页指纹**在所有账号之间相同。对 claude.ai 的登录风控来说，这比 MachineGuid 是更强的信号。它不属于本文的"系统标识"范畴，也没有在本轮处理，需要单独评估。

## 尚未完成的内容

1. **虚拟机级隔离（理解 B）**：没有。
2. **WMI / 固件 / 磁盘序列号 / DUID 层的标识**：没有，原因见上一节。
3. **网页指纹**：没有，见上一节末段。
4. **不开沙箱时的强制代理**：没有。不开沙箱时只有启动前的连通性核对和代理环境变量两条约束，自带网络栈或忽略代理变量的进程仍能直连；要做到与沙箱等价需要管理员权限下的 WFP 过滤或等价的内核层拦截。
5. **用户名与 SID**：不打算在 Sandboxie 方案内处理，见上一节。

## 本轮改动（第二轮）

- 开沙箱的环境强制"独立 Code"，消除各沙箱共用 Claude Code 设备 ID 的问题；界面开关锁定并说明。
- 注册表标识改在 `NtQueryValueKey` / `NtEnumerateValueKey` 应答，覆盖枚举、ANSI 版本和所有 `RegGetValue` 变体；修正了 `RegQueryValueExW(…, NULL, NULL)` 存在性探测返回 `ERROR_INVALID_PARAMETER` 的偏差。
- 卷序列号改在 `NtQueryVolumeInformationFile` / `NtQueryInformationFile(FileIdInformation)` 改写，以真实卷号为键、带账号键的非线性派生；Node `fs.stat().dev`、句柄类 API、`NULL` 根、挂载点卷全部一致。
- MAC 改按网卡 GUID 派生，并新增 `GetIfTable`、`GetIfEntry`、`GetIfTable2[Ex]`、`GetIfEntry2[Ex]`。
- 主机名补齐 `USERDOMAIN`、`USERDOMAIN_ROAMINGPROFILE`、`LOGONSERVER` 环境变量，以及 `LookupAccountSid/Name`、`GetUserNameEx` 返回的账号域。
- release 构建缺 32 位防护时直接失败。
- 集成测试新增枚举、`whoami`、Node 三组断言；单元测试新增跨账号关联、环境变量替换和一组真实沙箱观测值的钉住测试。
