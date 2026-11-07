import { getCurrentWindow } from "@tauri-apps/api/window";
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ToastProvider } from "./components/ui/toast";
import { applyTheme } from "./lib/theme";
import "./index.css";

applyTheme();

document.addEventListener("contextmenu", (e) => {
  if (!(e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement)) e.preventDefault();
});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ToastProvider>
      <App />
    </ToastProvider>
  </React.StrictMode>,
);

requestAnimationFrame(() => {
  const win = getCurrentWindow();
  win.show().then(() => win.unminimize()).then(() => win.setFocus());
});
