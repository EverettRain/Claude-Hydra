import { useState } from "react";
import { errorText } from "@/lib/api";
import { DownloadButton } from "./DownloadButton";
import { Button } from "./ui/button";
import { Dialog } from "./ui/dialog";

/**
 * Asked before an account starts in an environment whose sandbox cannot run here (Sandboxie-Plus
 * missing): install it, or turn the environment's sandbox off and carry on. Never silently.
 */
export function SandboxPrompt({ environment, onClose, onWithout }: { environment: string | null; onClose: () => void; onWithout: () => Promise<void> }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const without = async () => {
    setBusy(true); setError("");
    try { await onWithout(); }
    catch (e) { setError(errorText(e)); }
    finally { setBusy(false); }
  };
  return (
    <Dialog
      open={environment !== null}
      onClose={() => !busy && onClose()}
      title="需要 Sandboxie-Plus"
      className="max-w-[560px]"
      description={`环境「${environment ?? ""}」开启了沙箱隔离，但这台电脑上没有检测到 Sandboxie-Plus。`}
      footer={
        <>
          <Button variant="ghost" onClick={onClose} disabled={busy}>取消</Button>
          <Button variant="outline" onClick={without} disabled={busy}>{busy ? "正在保存…" : "不使用沙箱隔离并继续"}</Button>
          <DownloadButton tool="sandboxie" variant="default" />
        </>
      }
    >
      <div className="space-y-2.5 text-[12.5px] leading-[1.65] text-fg-2">
        <p>沙箱隔离让 Claude 和它启动的程序只能经本账号的中继联网，忽略代理设置的程序会断网而不是直连；同时让它们看到环境的时区和系统语言，按账号替换机器标识，并隐藏网卡上的公网地址。</p>
        <p>Sandboxie-Plus 是免费的开源软件，安装时需要管理员权限并安装驱动。装好后回到这里再启动即可。</p>
        <p className="text-muted">选择不使用：这个环境会保存为不使用沙箱隔离，之后不再询问，可以随时在环境配置中重新开启。不使用时，忽略代理设置的程序可能绕过节点直连，Claude 窗口可能显示系统时区，网页能读到网卡上的公网地址。各账号仍使用各自独立的 Claude Code 登录。</p>
        {error && <p role="alert" className="selectable text-danger">{error}</p>}
      </div>
    </Dialog>
  );
}
