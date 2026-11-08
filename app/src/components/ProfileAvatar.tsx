import defaultIcon from "@/assets/claude-default.png";
import { PALETTE, cn } from "@/lib/utils";

interface Props { color: number; badge: string; isDefault: boolean; className: string }

export function ProfileAvatar({ color, badge, isDefault, className }: Props) {
  return (
    <span className={cn("relative inline-block", className)} style={{ containerType: "inline-size" }} aria-hidden="true">
      <img src={defaultIcon} alt="" draggable={false} className="h-full w-full" />
      {!isDefault && (
        <span className="absolute bottom-0 right-0 grid h-1/2 min-w-[50%] place-items-center rounded-full border border-white font-bold leading-none text-white"
          style={{ backgroundColor: PALETTE[color % PALETTE.length], fontSize: badge.length > 1 ? "29cqw" : "35cqw" }}>
          {badge}
        </span>
      )}
    </span>
  );
}
