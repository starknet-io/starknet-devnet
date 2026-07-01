// ESLint flat config for the Starknet Devnet web UI.
// Uses typescript-eslint for typed lint rules, plus React Hooks and
// React Refresh rules. Prettier is wired in last via `eslint-config-prettier`
// so its formatter is the source of truth and any conflicting stylistic
// rules from upstream configs are disabled.

import js from "@eslint/js";
import globals from "globals";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import prettier from "eslint-config-prettier";

export default tseslint.config(
  {
    ignores: ["dist", "node_modules", "eslint.config.js"],
  },
  {
    files: ["**/*.{ts,tsx}"],
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: "module",
      globals: { ...globals.browser },
    },
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": [
        "warn",
        {
          allowConstantExport: true,
          // Context providers and their types are commonly defined alongside
          // the React Context object. Naming them here lets the rule pass
          // without splitting every provider into its own file.
          allowExportNames: ["DevnetContext", "DevnetContextType"],
        },
      ],
      // The UI is a small SPA; relaxed rules below keep the lint signal
      // focused on real bugs rather than style.
      "@typescript-eslint/no-unused-vars": [
        "warn",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
      "@typescript-eslint/no-explicit-any": "warn",
    },
  },
  prettier,
);
