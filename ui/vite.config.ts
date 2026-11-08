/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";

// Character packs live at the repo root (`characters/<id>/`) and are served as static files.
const charactersDir = fileURLToPath(new URL("../characters", import.meta.url));

export default defineConfig({
  plugins: [react()],
  publicDir: charactersDir,
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: { target: "es2022", outDir: "dist", emptyOutDir: true },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
  },
});
