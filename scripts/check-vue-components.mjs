#!/usr/bin/env node
// 静态检查：模板里用到、但**既没 import 也没全局注册**的组件。
//
// # 为什么需要它
//
// 这类 bug 已经犯过两次（`NRadioGroup`、`NSelect`），而且两次都逃过了所有检查：
//   - `vue-tsc` 不报：script-setup 里未解析的组件会被当成普通标签，不报错
//   - `cargo build` 不关前端的事
//   - 只有**跑起来看渲染结果**才会发现：Vue 把未知标签渲染成空元素，
//     子内容退化成纯文本（"测试环境1（dev）灰度环境（pre）"就是这么来的）
//
// 而这一类是**可以静态查出来的** —— 模板里的 PascalCase 标签名，要么在
// `<script setup>` 里 import 过，要么是内建的（Transition/KeepAlive…）。
// 所以就写成脚本，放进 CI/自测里，一劳永逸。
//
// 用法：node scripts/check-vue-components.mjs

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

/** Vue 内建组件（模板里可以直接用，不需要 import）。 */
const BUILTIN = new Set([
  "Transition",
  "TransitionGroup",
  "KeepAlive",
  "Teleport",
  "Suspense",
  "Component",
]);

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) {
      if (name === "node_modules" || name === "dist") continue;
      walk(p, out);
    } else if (name.endsWith(".vue")) {
      out.push(p);
    }
  }
  return out;
}

/**
 * 文件自己的组件名（`Foo.vue` → `Foo`）。
 *
 * **递归组件靠文件名自引用**：`<script setup>` 里 `<DocTree>` 出现在
 * `DocTree.vue` 内部是合法的，不需要 import。忘了这条会把正常的递归组件
 * 报成错误。
 */
function selfName(file) {
  const base = file.split("/").pop() ?? "";
  return base.replace(/\.vue$/, "");
}

/** 模板里出现的 PascalCase 标签（含单文件内自定义的）。 */
function usedComponents(template) {
  const out = new Set();
  for (const m of template.matchAll(/<([A-Z][A-Za-z0-9]*)[\s/>]/g)) {
    out.add(m[1]);
  }
  return out;
}

/** `<script setup>` 里 import 进来的名字。 */
function importedNames(script) {
  const out = new Set();
  // import X from "..." / import { X, Y as Z } from "..."
  for (const m of script.matchAll(/import\s+([^;]+?)\s+from\s+["'][^"']+["']/g)) {
    const clause = m[1];
    const braced = clause.match(/\{([^}]*)\}/);
    if (braced) {
      for (const part of braced[1].split(",")) {
        const name = part.trim().split(/\s+as\s+/).pop()?.trim();
        if (name) out.add(name);
      }
    }
    const def = clause.replace(/\{[^}]*\}/, "").replace(/,/g, "").trim();
    if (def && !def.startsWith("*")) out.add(def);
  }
  return out;
}

const problems = [];
for (const file of walk("src")) {
  const text = readFileSync(file, "utf8");
  const s = text.indexOf("<script");
  const e = text.indexOf("</script>");
  const t = text.indexOf("<template>");
  if (t < 0) continue;
  const script = s >= 0 ? text.slice(s, e) : "";
  const template = text.slice(t);
  const imported = importedNames(script);

  for (const c of usedComponents(template)) {
    // 在当前文件里自己定义的（script 里出现过这个名字 + defineComponent/const X =）
    const definedLocally = new RegExp(
      `(const|function|class)\\s+${c}\\b|defineComponent|${c}\\s*:`,
    ).test(script);
    if (imported.has(c) || BUILTIN.has(c) || definedLocally || c === selfName(file)) continue;
    problems.push(`${relative(process.cwd(), file)}: 模板用了 <${c}>，但没 import`);
  }
}

if (problems.length) {
  console.error("✗ 模板里有未导入的组件：\n");
  for (const p of problems) console.error("   " + p);
  console.error(
    "\n  这类问题 vue-tsc 不报，但运行时会渲染成空元素、子内容变纯文本。",
  );
  process.exit(1);
}
console.log("✓ 模板组件导入检查通过");