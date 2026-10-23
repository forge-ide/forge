import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

// Tauri expects a fixed port and fails if it's taken rather than silently moving.
export default defineConfig({
  plugins: [solid()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    target: "es2022",
  },
});
