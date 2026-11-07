import { useEffect, useState } from "react";

export type ThemeMode = "system" | "light" | "dark";
const KEY = "hydra-theme";

function read(): ThemeMode {
  try {
    const v = localStorage.getItem(KEY);
    if (v === "light" || v === "dark" || v === "system") return v;
  } catch {
    /* storage unavailable */
  }
  return "system";
}

const media = window.matchMedia("(prefers-color-scheme: dark)");

export function applyTheme(mode: ThemeMode = read()) {
  const dark = mode === "dark" || (mode === "system" && media.matches);
  document.documentElement.classList.toggle("dark", dark);
}

export function useTheme() {
  const [mode, setMode] = useState<ThemeMode>(read);
  useEffect(() => {
    applyTheme(mode);
    try {
      localStorage.setItem(KEY, mode);
    } catch {
      /* ignore */
    }
    const onChange = () => applyTheme(mode);
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, [mode]);
  return [mode, setMode] as const;
}
