#!/usr/bin/env node
//! 检查 Vue 单文件组件里的 **CSS 嵌套**。
//!
//! # 为什么需要
//!
//! `<style scoped>` 里是**普通 CSS**（不是 SCSS），它**不支持嵌套**。写成
//! ```css
//! .ws-row { ... .group-head { ... } }
//! ```
//! 浏览器/Vite **不会报错**，而是把嵌套的那几块**静默丢掉** —— 于是「样式写了
//! 但没生效」，看界面只会觉得「怎么这么丑」，极难联想到是 CSS 的问题。
//!
//! 这个坑真踩过：给需求列表加分组头样式时插错了层级，分组头一点样式都没有
//! （实测反馈「很丑 很挤 很不明显」），排查花了好几轮。
//!
//! `@media` / `@supports` / `@layer` 里嵌套是**合法**的，不算问题。

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const ROOT = "src";
const AT_RULE_OK = /^@(media|supports|layer|container|scope)\b/;

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (p.endsWith(".vue")) out.push(p);
  }
  return out;
}

/** 取出所有 `<style>` 块（含 scoped / module）。 */
function styleBlocks(src) {
  const out = [];
  const re = /<style[^>]*>([\s\S]*?)<\/style>/g;
  let m;
  while ((m = re.exec(src))) {
    const startLine = src.slice(0, m.index).split("\n").length;
    out.push({ css: m[1], startLine });
  }
  return out;
}

const problems = [];
for (const file of walk(ROOT)) {
  const src = readFileSync(file, "utf8");
  for (const { css, startLine } of styleBlocks(src)) {
    // 逐行扫：维护「当前是不是在合法的 at-rule 里」
    let depth = 0;
    const atRuleDepths = [];
    const lines = css.split("\n");
    for (let i = 0; i < lines.length; i++) {
      const raw = lines[i];
      const line = raw.trim();
      if (!line || line.startsWith("/*") || line.startsWith("*") || line.startsWith("//")) {
        // 注释行里的大括号不参与计数（简化处理：单行注释）
        continue;
      }
      // 这一行是不是「一个选择器」的开头（有缩进且不是声明）
      const looksLikeSelector =
        /^\s+[.&#*:[a-zA-Z]/.test(raw) &&
        !/[:;]\s*[^;{]*;?\s*$/.test(line.replace(/\{.*$/, "")) &&
        !line.startsWith("--") &&
        !/^[a-z-]+\s*:/.test(line);
      const insideAtRule = depth > 0 && atRuleDepths.some((d) => depth > d);
      if (looksLikeSelector && line.includes("{") && !insideAtRule) {
        problems.push({
          file,
          line: startLine + i,
          text: line,
        });
      }
      if (AT_RULE_OK.test(line)) atRuleDepths.push(depth);
      depth += (raw.match(/\{/g) || []).length;
      depth -= (raw.match(/\}/g) || []).length;
      while (atRuleDepths.length && depth <= atRuleDepths[atRuleDepths.length - 1]) {
        atRuleDepths.pop();
      }
    }
  }
}

if (problems.length) {
  console.error("✗ 发现疑似 CSS 嵌套（普通 CSS 不支持嵌套，会被静默丢掉）\n");
  for (const p of problems) {
    console.error(`  ${relative(".", p.file)}:${p.line}`);
    console.error(`      ${p.text}`);
  }
  console.error("\n  这些样式很可能**根本没有生效**。请把选择器挪到 <style> 的顶层。");
  process.exit(1);
}
console.log("✓ CSS 嵌套检查通过");
