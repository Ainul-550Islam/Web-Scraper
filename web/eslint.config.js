import js from "@eslint/js";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["dist", "coverage", "node_modules"] },
  {
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      ecmaVersion: 2022,
      globals: globals.browser,
    },
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": [
        "warn",
        { allowConstantExport: true },
      ],
      "@typescript-eslint/no-explicit-any": "error",
      eqeqeq: ["error", "smart"],
      "prefer-const": "error",
      "@typescript-eslint/consistent-type-imports": "warn",
    },
  },
  {
    files: ["src/test/**/*", "**/*.test.{ts,tsx}"],
    languageOptions: { globals: globals.node },
  },
  {
    // Barrel modules aggregate split units by design; react-refresh's
    // component-granularity rule targets definition files, not pure re-export modules.
    files: ["src/**/index.ts", "src/**/index.tsx"],
    rules: { "react-refresh/only-export-components": "off" },
  },
);
