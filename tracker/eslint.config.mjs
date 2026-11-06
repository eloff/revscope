import js from "@eslint/js";
import globals from "globals";

export default [
  {
    ignores: [
      "node_modules/**",
      "coverage/**",
      "example/**",
      "tracker-min.js",
      "scripts/benchmark-output/**",
      "scripts/minify-benchmark.mjs",
    ],
  },
  js.configs.recommended,
  {
    files: ["scripts/**/*.mjs", "vitest.config.mjs"],
    languageOptions: {
      ecmaVersion: "latest",
      sourceType: "module",
      globals: {
        ...globals.node,
      },
    },
  },
  {
    files: ["test/**/*.mjs"],
    languageOptions: {
      ecmaVersion: "latest",
      sourceType: "module",
      globals: {
        ...globals.node,
        ...globals.vitest,
      },
    },
  },
  {
    files: ["**/*.js"],
    languageOptions: {
      ecmaVersion: 5,
      sourceType: "script",
      globals: {
        ...globals.browser,
        RR_TEST: "readonly",
      },
    },
    rules: {
      "no-unused-vars": ["error", { caughtErrorsIgnorePattern: "^_" }],
    },
  },
];
