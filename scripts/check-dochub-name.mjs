//! 检查界面里有没有**写死的文档库目录名**。
//!
//! # 为什么需要它
//!
//! 文档库在 hub 里的目录名是**从用户填的 URL 推导**出来的
//! （`x-doc-hub.git` → `x-doc-hub`，见 `Hub::dochub_name`），不是固定的
//! `doc-hub`。界面里任何写死这个名字的地方都会当场出错：
//!
//! - `infoOf('doc-hub')` → 查不到，那一行渲染成空
//! - `` `${wsPath}/doc-hub` `` → 路径错，「打开目录」打不开
//! - `<span>doc-hub</span>` → 显示成用户从没填过的名字
//!
//! 这个坑踩过**三次**：Rust 的常量、Vue 模板里的裸字面量、还有脚本区里
//! `'doc-hub'` 这种带引号的形式（前两次的批量替换各漏了一种）。
//! 所以做成检查：一律改用 `dochub_name`（来自 `HubInfo`）或 store 里的
//! computed（`docName` / `DOCHUB`）。

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const ROOT = new URL("..", import.meta.url).pathname;
const SKIP_DIRS = new Set(["node_modules", "dist", "gen", "target"]);

/** 收集 src 下的 .vue / .ts（跳过测试：那里用字面量是刻意的）。 */
function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    if (SKIP_DIRS.has(name)) continue;
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (/\.(vue|ts)$/.test(name) && !/\.test\.ts$/.test(name)) out.push(p);
  }
  return out;
}

/**
 * 这一行是不是「允许的用法」。
 *
 * 只有三类例外，每一类都要能说清为什么：
 * 1. **注释里**（用户看不到，且注释里提历史名字是必要的）；
 * 2. **兜底值** `?? "doc-hub"` —— hub 还没打开时总得有个名字可用；
 * 3. 明确写了「旧名 / 历史上 / legacy」的说明句。
 */
function allowed(line, inComment) {
  if (inComment) return true;
  const t = line.trim();
  // 行首是注释：`// …`、多行注释的续行 `* …`、单行块注释 `/** … */`、`<!-- … -->`
  if (/^(\/\/|\*|\/\*|<!--)/.test(t)) return true;
  // 单行 HTML 注释出现在行中（如 `<p>…</p><!-- 说明 -->`）
  if (line.includes("<!--") && line.includes("-->")) return true;
  if (/\?\?\s*"doc-hub"/.test(line)) return true;
  if (/旧名|历史上|legacy/i.test(line)) return true;
  return false;
}

/**
 * 处理完这一行之后，是否还在 HTML 注释里。
 *
 * HTML 注释不嵌套，所以「进去之后一直算注释内，直到遇到 `-->`」。
 * （这里踩过一次：写成「在注释内且本行没有 --> 就置回 false」，
 *  于是注释的第二行起被当成代码，满屏误报。）
 */
function stillInComment(line, inComment) {
  const open = line.indexOf("<!--");
  const close = line.indexOf("-->");
  if (open >= 0) return close < 0 || close < open;
  if (inComment) return close < 0;
  return false;
}

const bad = [];
for (const f of walk(join(ROOT, "src"))) {
  let inComment = false;
  readFileSync(f, "utf8")
    .split("\n")
    .forEach((line, i) => {
      const wasInComment = inComment;
      inComment = stillInComment(line, inComment);
      if (!line.includes("doc-hub")) return;
      if (allowed(line, wasInComment)) return;
      bad.push(`${relative(ROOT, f)}:${i + 1}  ${line.trim().slice(0, 100)}`);
    });
}

if (bad.length) {
  console.error("✗ 界面里出现了写死的文档库目录名 `doc-hub`：\n");
  for (const b of bad) console.error("   " + b);
  console.error(
    "\n  目录名从 hub 配置推导（`Hub::dochub_name`，随用户填的 URL 变），请改用：\n" +
      "    Vue 模板/脚本：store 里的 computed `docName` / `DOCHUB`\n" +
      "    别处：从 `app.hub?.dochub_name` 取\n",
  );
  process.exit(1);
}
console.log("✓ 文档库目录名没有写死（一律来自 hub 配置）");