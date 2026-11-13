import hydra from "@/assets/app-icons/hydra.png";
import maidIndigo from "@/assets/app-icons/maid-indigo.png";
import maidSky from "@/assets/app-icons/maid-sky.png";
import maidTeal from "@/assets/app-icons/maid-teal.png";

/** Artwork of the selectable app icons, by the id the backend uses (see `app_icon.rs`). */
const ICONS: Record<string, string> = { "maid-sky": maidSky, "maid-teal": maidTeal, "maid-indigo": maidIndigo, hydra };

export const DEFAULT_ICON = "maid-sky";

export const appIconSrc = (id?: string | null) => ICONS[id ?? DEFAULT_ICON] ?? ICONS[DEFAULT_ICON];
