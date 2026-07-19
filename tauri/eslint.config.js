// Minimal lint gate. Exists first and foremost for no-undef: a missing React
// import (e.g. useRef) is not caught by the vite build and renders the whole
// app as a blank window at runtime (v1.3.13 regression).
import js from "@eslint/js";
import globals from "globals";

export default [
  js.configs.recommended,
  {
    files: ["src/**/*.js", "src/**/*.jsx"],
    languageOptions: {
      ecmaVersion: 2024,
      sourceType: "module",
      globals: globals.browser,
      parserOptions: { ecmaFeatures: { jsx: true } },
    },
    rules: {
      // Component definitions register as "unused" for plain ESLint; JSX usage
      // is what matters here, so keep this advisory.
      "no-unused-vars": ["warn", { varsIgnorePattern: "^[A-Z_]" }],
    },
  },
];
