/* eslint-env node */
/**
 * ESLint 配置（G5 / L6 前端门禁）。
 *
 * 设计原则（计划 §2.1）：先让现有代码过，不爆大量 error 卡死。
 * - 仅启用少量「硬错误」规则（hooks 规则 / 禁用 debugger / 禁 var），
 *   其余以 warn 形式给出渐进改进信号，不阻断 gate。
 * - 关闭与 TypeScript 冲突或噪音较大的核心规则，由 tsc 负责类型正确性。
 * - 不继承 eslint:recommended，避免历史代码触发大量 error。
 */
module.exports = {
  root: true,
  env: {
    browser: true,
    es2021: true,
    node: true,
  },
  parser: "@typescript-eslint/parser",
  parserOptions: {
    ecmaVersion: 2021,
    sourceType: "module",
    ecmaFeatures: { jsx: true },
  },
  plugins: ["@typescript-eslint", "react-hooks"],
  ignorePatterns: [
    "dist",
    "node_modules",
    "*.config.ts",
    "*.config.js",
    "scripts",
    "public",
    "src/_parked",
  ],
  rules: {
    // —— 硬错误（阻断 gate）——
    "react-hooks/rules-of-hooks": "error",
    "no-debugger": "error",
    "no-var": "error",

    // —— 警告（不阻断，渐进改进）——
    "react-hooks/exhaustive-deps": "warn",
    "@typescript-eslint/no-unused-vars": [
      "warn",
      { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
    ],

    // —— 关闭与 TS / 现有代码冲突的规则 ——
    "no-undef": "off",
    "no-unused-vars": "off",
    "no-empty": "off",
    "no-empty-function": "off",
    "no-console": "off",
    "no-prototype-builtins": "off",
    "no-case-declarations": "off",
    "no-fallthrough": "off",
    "no-cond-assign": "off",
    "no-constant-condition": ["error", { checkLoops: false }],
    "no-control-regex": "off",
    "no-misleading-character-class": "off",
    "no-unsafe-finally": "off",
    "no-unsafe-negation": "off",
    "no-useless-escape": "off",
    "require-yield": "off",
    "no-async-promise-executor": "off",
    "no-redeclare": "off",
    "no-dupe-class-members": "off",
    "no-unreachable": "off",
    "no-use-before-define": "off",
    "no-shadow": "off",
    "no-extra-boolean-cast": "off",
    "no-return-assign": "off",
    "no-throw-literal": "off",
    "no-sequences": "off",
    "no-with": "off",
    "no-bitwise": "off",
    "no-label-var": "off",
    "no-restricted-syntax": "off",
    "@typescript-eslint/no-explicit-any": "off",
    "@typescript-eslint/ban-ts-comment": "off",
    "@typescript-eslint/no-non-null-assertion": "off",
    "@typescript-eslint/explicit-module-boundary-types": "off",
    "@typescript-eslint/no-inferrable-types": "off",
    "@typescript-eslint/no-namespace": "off",
    "@typescript-eslint/no-empty-interface": "off",
    "@typescript-eslint/no-empty-function": "off",
    "no-unexpected-multiline": "off",
    "no-tabs": "off",
    "no-mixed-spaces-and-tabs": "off",
    "constructor-super": "off",
    "use-isnan": "off",
    "valid-typeof": "off",
  },
  overrides: [
    {
      files: ["*.ts", "*.tsx"],
      rules: {
        "@typescript-eslint/no-unused-vars": [
          "warn",
          { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
        ],
      },
    },
  ],
};
