#!/usr/bin/env node
//! 检查「在 Provider 所在的文件里调它自己的 hook」。
//!
//! # 为什么需要（真踩过，而且**表现为整个界面白屏**）
//!
//! Naive UI 的 `useMessage()` / `useDialog()` 只能用在 **Provider 内部的后代组件**里。
//! 而 `NMessageProvider` 是在 `App.vue` 里渲染的 —— 所以 `App.vue` 自己是
//! Provider 的**父级**，在那儿调 `useMessage()` 会直接抛错，
//! **整个应用崩掉、打开就是白屏**。
//!
//! 这种错误类型检查（vue-tsc）和构建（vite）都**不会报**，只在运行时炸。
//! 实测踩过：给 App.vue 加了个改名提示用 `useMessage()`，装出来的包一开就白屏。
//!
//! 判定：文件里**渲染**了 `<NMessageProvider>`（等等）**并且**调用了对应的 hook。

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const ROOT = "src";
/** hook → 它必须身处的 Provider。 */
const PAIRS = [
  ["useMessage", "NMessageProvider"],
  ["useDialog", "NDialogProvider"],
  ["useNotification", "NNotificationProvider"],
  ["useLoadingBar", "NLoadingBarProvider"],
];

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (p.endsWith(".vue")) out.push(p);
  }
  return out;
}

/** 去掉注释，避免「注释里提到 Provider」被误判。 */
function stripComments(src) {
  return src.replace(/<!--[\s\S]*?-->/g, "").replace(/\/\/[^\n]*/g, "").replace(/\/\*[\s\S]*?\*\//g, "");
}

const problems = [];
for (const file of walk(ROOT)) {
  const src = stripComments(readFileSync(file, "utf8"));
  for (const [hook, provider] of PAIRS) {
    const rendersIt = new RegExp(`<${provider}[\\s>/]`).test(src);
    const callsIt = new RegExp(`\\b${hook}\\s*\\(`).test(src);
    if (rendersIt && callsIt) problems.push({ file, hook, provider });
  }
}

if (problems.length) {
  console.error("✗ 在 Provider 所在的文件里调用了它自己的 hook（会在运行时崩成白屏）\n");
  for (const p of problems) {
    console.error(`  ${relative(".", p.file)}：渲染了 <${p.provider}>，同时调了 ${p.hook}()`);
  }
  console.error("\n  该 hook 只能用在 Provider **内部的后代组件**里。");
  console.error("  把逻辑挪到一个子组件（像 NotifierBridge 那样），或用 lib/notify 的出口。");
  process.exit(1);
}
console.log("✓ Provider hook 检查通过");
