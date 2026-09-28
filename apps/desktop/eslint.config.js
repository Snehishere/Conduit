import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactPlugin from "eslint-plugin-react";
import reactHooksPlugin from "eslint-plugin-react-hooks";

export default tseslint.config(
  // Global ignores
  {
    ignores: ["dist/", "node_modules/", "src-tauri/", "**/*.test.*", "**/*.spec.*"],
  },

  // Base recommended rules
  js.configs.recommended,

  // TypeScript strict type-checked rules (uses TS 6 API via npm alias)
  ...tseslint.configs.strictTypeChecked,

  // React plugin
  {
    plugins: {
      react: reactPlugin,
      "react-hooks": reactHooksPlugin,
    },
    rules: {
      // React 19+ uses automatic JSX transform — React import not required
      "react/jsx-uses-react": "off",
      "react/jsx-uses-vars": "error",
      "react/no-deprecated": "warn",
      "react/react-in-jsx-scope": "off",

      // React hooks — warn to catch missing dependency arrays without blocking
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "warn",

      // Core rules
      "no-unused-vars": "off", // handled by @typescript-eslint/no-unused-vars
      "eqeqeq": "error",
      // Narrow (not disable): warn/error are legitimate feedback channels in
      // error boundaries and catch paths; log/info/debug noise stays banned.
      "no-console": ["warn", { allow: ["warn", "error"] }],
      "no-empty": "warn",

      // TypeScript rules — warn on existing patterns, error on new violations
      "@typescript-eslint/no-unused-vars": ["warn", { argsIgnorePattern: "^_", varsIgnorePattern: "^(React|_)$" }],
      "@typescript-eslint/no-explicit-any": "warn",
      "@typescript-eslint/no-unsafe-assignment": "warn",
      "@typescript-eslint/no-unsafe-member-access": "warn",
      "@typescript-eslint/no-unsafe-argument": "warn",
      "@typescript-eslint/no-unsafe-return": "warn",
      "@typescript-eslint/no-floating-promises": ["warn", { ignoreVoid: true }],
      "@typescript-eslint/no-misused-promises": ["warn", { checksVoidReturn: false }],
      "@typescript-eslint/no-confusing-void-expression": "warn",
      "@typescript-eslint/restrict-template-expressions": ["warn", { allowNumber: true }],
      "@typescript-eslint/no-unnecessary-type-assertion": "warn",
      "@typescript-eslint/no-unnecessary-condition": "warn",
      "@typescript-eslint/no-non-null-assertion": "warn",
      "@typescript-eslint/require-await": "warn",
      "@typescript-eslint/use-unknown-in-catch-callback-variable": "warn",
    },
    languageOptions: {
      parser: tseslint.parser,
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    settings: {
      react: {
        version: "detect",
      },
    },
  },
);
