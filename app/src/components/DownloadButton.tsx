import { ExternalLink } from "lucide-react";
import { api, errorText, type DownloadTool } from "@/lib/api";
import { Button } from "./ui/button";
import { useToast } from "./ui/toast";

const NAMES: Record<DownloadTool, string> = { claude: "Claude", orbstack: "OrbStack", docker: "Docker Desktop", sandboxie: "Sandboxie-Plus" };

/** Opens the official download page of a program Hydra needs but does not install itself. */
export function DownloadButton({ tool, className }: { tool: DownloadTool; className?: string }) {
  const toast = useToast();
  return (
    <Button variant="outline" size="sm" className={className} onClick={() => void api.openDownloadPage(tool).catch((e) => toast(errorText(e)))}>
      <ExternalLink />
      前往 {NAMES[tool]} 官网下载
    </Button>
  );
}
