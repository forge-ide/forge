import { defineConfig } from "vite";
import { fileURLToPath } from "node:url";
import solid from "vite-plugin-solid";

// Tauri expects a fixed port and fails if it's taken rather than silently moving.
export default defineConfig(({ mode }) => ({
  plugins: [solid()],
  resolve: {
    alias:
      mode === "e2e-web"
        ? { "./host/platform": fileURLToPath(new URL("./src/host/web.ts", import.meta.url)) }
        : {},
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    target: "es2022",
  },
}));
