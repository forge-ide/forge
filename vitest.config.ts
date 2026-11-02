import { defineConfig } from "vitest/config";
import solid from "vite-plugin-solid";
export default defineConfig({
  plugins: [solid({ hot: false })],
  resolve: { conditions: ["browser"] },
  test: {
    include: ["packages/*/src/**/*.test.{ts,tsx}"],
    environment: "jsdom",
    setupFiles: ["./packages/ui/src/testing/setup.ts"],
  },
});
