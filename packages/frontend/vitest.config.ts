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
});
