import path from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  root: "src",
  plugins: [react()],
  base: "./",
  build: { outDir: "../dist", emptyOutDir: true, target: "es2021" },
  server: { port: 3017, strictPort: true },
  resolve: { alias: { "@": path.resolve(__dirname, "./src") } },
  clearScreen: false,
  envPrefix: ["VITE_", "TAURI_"],
});
