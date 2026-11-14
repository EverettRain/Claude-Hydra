import { open as pickFolder } from "@tauri-apps/plugin-dialog";
import { FolderPlus, X } from "lucide-react";
import { useEffect, useState } from "react";
import { api, errorText, type ContainerInput, type ContainerRuntime } from "@/lib/api";
import { EnvironmentPicker } from "./EnvironmentPicker";
import { Button } from "./ui/button";
import { Dialog } from "./ui/dialog";
import { Input, Select } from "./ui/input";
import { Switch } from "./ui/switch";

interface Props {
  open: boolean;
  /** Existing container account id, or null to create one. */
  id: string | null;
  initial?: ContainerInput;
  onClose: () => void;
  onSaved: (id: string) => void;
  runtime: ContainerRuntime | null;
}

const empty = (): ContainerInput => ({ name: "", environment: "", image: "", mounts: [], sshAgent: false, claudeDir: "" });

const baseName = (path: string) => path.replace(/[\\/]+$/, "").split(/[\\/]/).pop()?.replace(/:$/, "") || "folder";

function Line({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[88px_1fr] items-center gap-3 px-4 py-2">
      <span className="text-[12.5px] text-fg-2">{label}</span>
      <div className="flex min-w-0 items-center gap-2">{children}</div>
    </div>
  );
}

/** Create or edit a Claude Code container account: environment, shared folders and image. */
export function ContainerDialog({ open, id, initial, onClose, onSaved, runtime }: Props) {
  const runtimeName = runtime?.name ?? "容器运行时";
  const [value, setValue] = useState<ContainerInput>(empty);
  const [customImage, setCustomImage] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!open) return;
    const v = initial ? structuredClone(initial) : empty();
    setValue(v);
    setCustomImage(!!v.image);
    setError("");
  }, [open, initial]);

  const addFolders = async () => {
    try {
      const picked = await pickFolder({ directory: true, multiple: true, title: "选择要共享到容器的文件夹" });
      const paths = (Array.isArray(picked) ? picked : picked ? [picked] : []).filter((p) => !value.mounts.some((m) => m.source === p));
      if (paths.length) setValue((v) => ({ ...v, mounts: [...v.mounts, ...paths.map((source) => ({ source, name: baseName(source) }))] }));
    } catch (e) {
      setError(errorText(e));
    }
  };

  const chooseClaudeDir = async () => {
    try {
      const picked = await pickFolder({ directory: true, title: "选择保存 Claude Code 数据的文件夹" });
      if (typeof picked === "string") setValue((v) => ({ ...v, claudeDir: picked }));
    } catch (e) {
      setError(errorText(e));
    }
  };

  const save = async () => {
    setSaving(true);
    setError("");
    try {
      onSaved(await api.saveContainer(id, { ...value, image: customImage ? value.image.trim() : "" }));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={id ? "编辑容器账号" : "新建容器账号"}
      description={`在 ${runtimeName} 容器中运行 Claude Code。容器只经所选环境的代理节点联网，时区与语言跟随环境。`}
      className="max-w-[600px] bg-bg"
      footer={
        <>
          <Button variant="outline" onClick={onClose}>取消</Button>
          <Button onClick={save} disabled={saving || !value.name.trim() || !value.environment}>{saving ? "保存中…" : "保存"}</Button>
        </>
      }
    >
      <div className="space-y-5">
        <section>
          <div className="group-label">基础</div>
          <div className="list-card">
            <Line label="名称">
              <Input aria-label="容器账号名称" value={value.name} maxLength={40} autoFocus placeholder="例如：美国工作站" onChange={(e) => setValue({ ...value, name: e.target.value })} />
            </Line>
          </div>
        </section>

        <section>
          <div className="group-label">环境（必须带代理节点）</div>
          <EnvironmentPicker requireNode value={value.environment} onChange={(environment) => setValue((v) => ({ ...v, environment }))} />
        </section>

        <section>
          <div className="group-label">共享文件夹</div>
          <div className="list-card">
            {value.mounts.map((m, i) => (
              <div key={m.source} className="flex items-center gap-2 px-4 py-2">
                <span className="selectable min-w-0 flex-1 truncate font-mono text-[11.5px] text-fg-2" title={m.source}>{m.source}</span>
                <span className="shrink-0 font-mono text-[11.5px] text-muted">→ /workspace/</span>
                <Input
                  aria-label="容器内的文件夹名称"
                  className="w-40 font-mono text-xs"
                  value={m.name}
                  onChange={(e) => setValue((v) => ({ ...v, mounts: v.mounts.map((x, j) => (j === i ? { ...x, name: e.target.value } : x)) }))}
                />
                <Button variant="ghost" size="iconSm" aria-label="移除" onClick={() => setValue((v) => ({ ...v, mounts: v.mounts.filter((_, j) => j !== i) }))}>
                  <X />
                </Button>
              </div>
            ))}
            <button onClick={addFolders} className="flex w-full items-center gap-2 px-4 py-2.5 text-[12.5px] text-muted hover:bg-card-2 hover:text-fg active:!scale-100">
              <FolderPlus className="h-3.5 w-3.5" />
              添加文件夹…
            </button>
          </div>
          <p className="mt-1.5 px-1 text-[11.5px] leading-relaxed text-muted">
            只共享具体的项目文件夹；不能共享整个用户目录或包含 Claude-Hydra 数据的目录。{id && "修改共享文件夹后需要重建容器才能生效（登录数据保留）。"}
            {!runtime?.sshAgent && "共享的文件夹经 Docker Desktop 挂进 Linux 虚拟机，读写比在本机慢，大型项目的安装和构建会更久。"}
          </p>
        </section>

        <section>
          <div className="group-label">Claude 数据</div>
          <div className="list-card">
            <div className="flex items-center gap-2 px-4 py-2.5">
              <span className="selectable min-w-0 flex-1 truncate font-mono text-[11.5px] text-fg-2" title={value.claudeDir || undefined}>
                {value.claudeDir || "Claude-Hydra 数据目录中的默认文件夹"}
              </span>
              {value.claudeDir && <Button variant="ghost" size="sm" onClick={() => setValue({ ...value, claudeDir: "" })}>恢复默认</Button>}
              <Button variant="outline" size="sm" onClick={chooseClaudeDir}>选择文件夹…</Button>
            </div>
            <p className="px-4 pb-3 text-[11.5px] leading-relaxed text-muted">
              Claude Code 的登录凭据（明文）、设置、记忆与 .claude.json 都保存在这个本机文件夹，挂载为容器的 /root/.claude。可以选择已有的 .claude 文件夹继续使用其中的登录与记忆；不能使用本机自己的 .claude，每个容器账号需要各自的文件夹。
              {id && "更改后需要重建容器，原有数据不会自动搬移。"}
            </p>
          </div>
        </section>

        <section>
          <div className="group-label">{runtime?.sshAgent ? "镜像与 SSH" : "镜像"}</div>
          <div className="list-card">
            <Line label="镜像">
              <Select aria-label="镜像" value={customImage ? "custom" : ""} onChange={(e) => setCustomImage(e.target.value === "custom")}>
                <option value="">内置镜像（Debian、Node 22、Claude Code、Python、编译工具）</option>
                <option value="custom">使用 {runtimeName} 中已有的镜像</option>
              </Select>
            </Line>
            {customImage && (
              <Line label="镜像名称">
                <Input aria-label="镜像名称" className="font-mono text-xs" placeholder="例如 cloaked-claude:latest" value={value.image} onChange={(e) => setValue({ ...value, image: e.target.value })} />
              </Line>
            )}
            {runtime?.sshAgent && (
              <Line label="SSH agent">
                <Switch checked={value.sshAgent} onCheckedChange={(sshAgent) => setValue({ ...value, sshAgent })} label="转发本机的 SSH agent" />
                <span className="text-[11.5px] text-muted">{value.sshAgent ? "转发本机的 SSH agent（私钥不进入容器）" : "不转发"}</span>
              </Line>
            )}
            <p className="px-4 pb-3 text-[11.5px] leading-relaxed text-muted">
              内置镜像在第一次打开时构建，需要几分钟。自定义镜像需要包含 claude 命令；镜像的修改在重建容器后生效。
            </p>
          </div>
        </section>

        {error && <p className="selectable whitespace-pre-wrap rounded-xl bg-danger-soft px-4 py-3 text-[12.5px] text-danger">{error}</p>}
      </div>
    </Dialog>
  );
}
