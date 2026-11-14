import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";

/**
 * What Hydra cannot protect in Claude's desktop app, shown on the Accounts page: on macOS the
 * advice not to use it when privacy matters most, on Windows to turn on the Sandboxie box.
 * Either way the user may go on; the test in Settings lets them see it for themselves.
 */
const TEXT: Record<string, string[]> = {
  macos: [
    "Claude 桌面版是 Electron 应用。它的网页可以通过 WebRTC 读到这台电脑网卡上的地址，包括网卡上的公网 IPv4 或公网 IPv6，再经正常的请求发回服务器。只有 Claude 自己能关掉这一点：Hydra 的启动参数对它无效，中继和出口保护也管不到。",
    "macOS 不允许在沙箱里再建沙箱，所以 Hydra 没法把整个 Claude 关进「只能经节点联网」的 Seatbelt 沙箱。Claude 里不读代理设置的部分（自己建连接的库、UDP、本机 DNS 查询）仍可能绕过节点。",
    "如果你对隐私有极高要求，不建议在 macOS 上使用 Claude 桌面版：网页版可以在「网页端」用账号浏览器打开（Chrome 的 WebRTC 限制有效），Claude Code 可以用「容器」（网关全程阻断直连）。",
  ],
  windows: [
    "Claude 桌面版是 Electron 应用。它的网页可以通过 WebRTC 读到这台电脑网卡上的地址，包括网卡上的公网地址。不开沙箱时，Claude 里不读代理设置的部分（自己建连接的库、UDP、本机 DNS 查询）也可能绕过节点。",
    "如果你对隐私要求高，建议在「环境配置」中为账号所用的环境打开「沙箱隔离」（需要安装 Sandboxie-Plus）：沙箱里只放行这个账号的中继，本机 DNS 查询会被拒绝，机器标识按账号替换。网卡地址目前在沙箱中仍可能经 WebRTC 读到。",
    "不开沙箱也可以继续使用，这条提示不会阻止你。",
  ],
};

export function PrivacyNotice({ platform, onTest, onDismiss }: {
  platform: string;
  /** Opens Settings at the test. */
  onTest: () => void;
  /** Closes the notice; `forever` = the user asked not to see it again. */
  onDismiss: (forever: boolean) => void;
}) {
  const [never, setNever] = useState(false);
  const text = TEXT[platform];
  if (!text) return null;
  return (
    <Dialog
      open
      onClose={() => onDismiss(never)}
      className="max-w-lg"
      title="隐私提示"
      footer={
        <>
          <label className="mr-auto flex cursor-pointer items-center gap-2 text-[12.5px] text-fg-2">
            <input type="checkbox" className="h-3.5 w-3.5 accent-[rgb(var(--brand))]" checked={never} onChange={(e) => setNever(e.target.checked)} />
            不再显示
          </label>
          <Button variant="outline" onClick={() => { onDismiss(never); onTest(); }}>去模拟测试</Button>
          <Button onClick={() => onDismiss(never)}>我知道了</Button>
        </>
      }
    >
      <div className="space-y-2.5 text-[12.5px] leading-[1.65] text-fg-2">
        {text.map((p) => <p key={p}>{p}</p>)}
        <p className="text-muted">想亲自验证，可以在「设置 → Claude 桌面端 → 模拟测试」用一个按同样方式启动的 Electron 程序，逐项检查每条网络路径实际从哪里出去。不再显示后，可以在同一处重新打开这条提示。</p>
      </div>
    </Dialog>
  );
}
