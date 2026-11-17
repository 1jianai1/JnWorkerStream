import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";

export default tseslint.config(
  {
    ignores: ["dist/**", "node_modules/**", "coverage/**", "playwright-report/**", "test-results/**"],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}", "tests/**/*.{ts,tsx}", "e2e/**/*.mjs", "*.{ts,js,mjs}"],
    languageOptions: {
      globals: { ...globals.browser, ...globals.node },
      parserOptions: { ecmaFeatures: { jsx: true } },
    },
    plugins: { "react-hooks": reactHooks },
    rules: {
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "warn",
      "@typescript-eslint/no-explicit-any": "error",
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_", varsIgnorePattern: "^_" }],
      "no-console": ["warn", { allow: ["warn", "error"] }],
    },
  },
  {
    files: ["e2e/**/*.mjs"],
    rules: { "no-console": "off" },
  },
  {
    files: ["src/**/*.{ts,tsx}"],
    // 后端实现（ipc.ts 与 tauri-backend.ts）可以碰 mock：它们负责把 Mock 或真实 Rust
    // 适配成同一个 Backend 接口；组件仍然只能通过 lib/ipc.ts 拿 backend。
    ignores: ["src/mock/**", "src/lib/ipc.ts", "src/lib/tauri-backend.ts"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              group: ["**/mock/**", "../mock/*", "../../mock/*", "../../../mock/*"],
              message: "Components must reach the backend through lib/ipc.ts; only ipc.ts may import src/mock/**.",
            },
          ],
        },
      ],
    },
  },
);
