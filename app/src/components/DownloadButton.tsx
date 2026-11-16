import { ExternalLink } from "lucide-react";
import { api, errorText, type DownloadTool } from "@/lib/api";
import { Button } from "./ui/button";
import { useToast } from "./ui/toast";

const NAMES: Record<DownloadTool, string> = { claude: "Claude", orbstack: "OrbStack", docker: "Docker Desktop", sandboxie: "Sandboxie-Plus" };

/** Opens the official download page of a program Hydra needs but does not install itself. */
export function DownloadButton({ tool, className, variant = "outline" }: { tool: DownloadTool; className?: string; variant?: "outline" | "default" }) {
  const toast = useToast();
  return (
    <Button variant={variant} size={variant === "outline" ? "sm" : undefined} className={className} onClick={() => void api.openDownloadPage(tool).catch((e) => toast(errorText(e)))}>
      <ExternalLink />
      前往 {NAMES[tool]} 官网下载
    </Button>
  );
}
