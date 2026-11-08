import { useState } from "react";
import { api, errorText } from "@/lib/api";
import { Button } from "./ui/button";
import { Input, Label } from "./ui/input";

export function ProxyField({ value, onChange }: { value: string; onChange: (value: string) => void }) {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ endpoint: string; message: string } | null>(null);
  const test = async () => {
    const endpoint = value.trim();
    setBusy(true);
    try {
      const ip = await api.checkProxy(endpoint);
      setResult({ endpoint, message: `出口 IP：${ip}` });
    } catch (e) {
      setResult({ endpoint, message: errorText(e) });
    } finally {
      setBusy(false);
    }
  };
  return <div className="space-y-2">
    <Label htmlFor="proxy-endpoint">独立代理入口（可选）</Label>
    <div className="flex gap-2">
      <Input id="proxy-endpoint" value={value} onChange={e => onChange(e.target.value)}
        placeholder="socks5://127.0.0.1:1080" className="font-mono text-xs" />
      <Button variant="outline" disabled={!value.trim() || busy} onClick={test}>{busy ? "检测中…" : "检测出口"}</Button>
    </div>
    <p className="text-xs text-muted-foreground">支持免密 HTTP、HTTPS 和 SOCKS5 入口。启动前通过此入口向 ipify 查询出口 IP，检测失败时阻止启动。子进程直连仍可能发生。</p>
    {result?.endpoint === value.trim() && <p role="status" className="text-xs">{result.message}</p>}
  </div>;
}
