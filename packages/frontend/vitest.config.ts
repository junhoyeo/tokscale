import { defineConfig } from "vitest/config";
import { resolve } from "path";

// Mirrors the `@/*` -> `./src/*` mapping in tsconfig.json so test files (and
// the production sources they import) can resolve aliased paths under
// `npx vitest run`. Without this, vitest fails on `import "@/lib/..."`
// outside of mocked imports because vitest does not honor tsconfig path
// aliases automatically.
export default defineConfig({
  resolve: {
    alias: {
      "@": resolve(__dirname, "./src"),
    },
  },
  test: {
    coverage: {
      provider: "v8",
      reporter: ["text", "json-summary", "html"],
      include: ["src/**/*.{ts,tsx}"],
      exclude: [
        "src/**/*.d.ts",
        "src/**/*.test.{ts,tsx}",
        "src/**/__tests__/**",
        "src/**/migrations/**",
        "src/**/*.stories.{ts,tsx}",
      ],
      // Thresholds intentionally set to a conservative floor; raise as the
      // suite grows. CI fails when any metric drops below these.
      thresholds: {
        lines: 40,
        functions: 40,
        branches: 40,
        statements: 40,
      },
    },
  },
});
