import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    globals: true,
    include: ["packages/browser-adapter/test/**/*.test.ts"],
    testTimeout: 60000,
  },
});
