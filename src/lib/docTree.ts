//! 文档目录树的**节点判定**。
//!
//! 抽出来是因为其中一条规则踩过坑、而且是**静默的**：搞错了界面不会报错，
//! 只会「一直转圈」（见 [`isLeafNode`]）。

import type { DocNode } from "../types";

/** 目录且一个子项都没有。 */
export function isEmptyDir(n: DocNode): boolean {
  return n.dir && !n.children?.length;
}

/**
 * 这个节点在 NTree 里该不该是**叶子**。
 *
 * # 空目录必须是叶子 —— 否则展开会**一直转圈**
 *
 * Naive UI 的 NTree 把「没有 `children` 又 `isLeaf !== true`」的节点当成
 * **异步待加载**节点。展开时它把该节点的 key 记进内部的 `loadingKeys`
 * 并把切换器渲染成**转圈**；随即发现没给 `onLoad`，于是直接 return ——
 * **那个 key 永远不会被移除**，就一直是转圈。
 * （源码：`TreeNode.mjs` 展开时 `loadingKeys.add(...)`，
 *   `Tree.mjs` 里 `warn("There is unloaded node in data but props.onLoad is not specified.")` 后 `return`。）
 *
 * 实测反馈：文档树里展开一个空目录（如 `sql/`）转个不停。
 *
 * 空目录本来也没有东西可展开，标成叶子（配合「（空）」后缀）最诚实。
 */
export function isLeafNode(n: DocNode): boolean {
  return !n.dir || isEmptyDir(n);
}

/** 文档相关的操作系统垃圾，展示时一律滤掉。 */
const OS_JUNK = [".DS_Store", "Thumbs.db", "desktop.ini"];
const isJunk = (name: string) => OS_JUNK.includes(name) || name.startsWith("._");

/**
 * 把**扁平**的文件列表拼成目录树（给 `DocTree` 用）。
 *
 * `doc_view` 给的是平铺的 `files`（每个带 `rel` 相对路径），而树要嵌套结构。
 * 目录节点的 `abs` 由「文件 abs 去掉 rel 尾部」得到的前缀推出 —— 这样不必
 * 依赖后端再给一份目录清单。
 *
 * 排序：目录在前，同类按名字 —— 跟文件管理器一致。
 */
export function nodesFromFiles(
  files: { name: string; rel: string; abs: string }[],
  emptyDirs: string[] = [],
  rootDir = "",
): DocNode[] {
  const root: DocNode = { name: "", abs: "", dir: true, children: [] };
  // 先按目录结构建出空目录的节点（`sql/` 这类刚建出来是空的，
  // 只列 files 的话界面上根本看不见它）
  for (const d of emptyDirs) {
    const parts = d.split("/").filter(Boolean).filter((p) => !isJunk(p));
    if (!parts.length) continue;
    let cur = root;
    for (let i = 0; i < parts.length; i++) {
      let next = cur.children!.find((c) => c.dir && c.name === parts[i]);
      if (!next) {
        next = { name: parts[i], abs: `${rootDir}/${parts.slice(0, i + 1).join("/")}`, dir: true, children: [] };
        cur.children!.push(next);
      }
      cur = next;
    }
  }
  /**
   * 目录节点的绝对路径：**从根目录拼**出来。
   *
   * ⚠️ 原来是从文件的 `abs` 里切掉 `rel` 后缀反推前缀 —— 那要求 `abs` 以 `rel`
   * 结尾，而 Windows 上 `abs` 用 `\`、`rel` 用 `/`，**必然切不出来**，于是目录的
   * `abs` 变成相对路径（点「打开目录」就找不到）。所以以 `rootDir` 为准。
   * `rootDir` 缺省时（那是测试与老调用点）才退回切后缀。
   */
  const dirAbs = (prefix: string, parts: string[]): string => {
    const rel = parts.join("/");
    // 根目录自己可能带 `\`（Windows）—— 一并换成 `/`：混着两种分隔符最难看，
    // 而 Windows 的 API 两种都吃。
    if (rootDir) return `${rootDir.replace(/\\/g, "/").replace(/\/+$/, "")}/${rel}`;
    return prefix ? prefix + rel : rel;
  };

  for (const f of files) {
    const parts = f.rel.split("/").filter(Boolean);
    // 按**文件名**判垃圾（不是逐段过滤）：逐段过滤会把 `sub/.DS_Store` 滤成
    // `["sub"]`，反而凭空造出一个叫 sub 的文件。
    if (!parts.length || isJunk(parts[parts.length - 1])) continue;
    // 退回方案用：文件 abs 去掉 rel 剩下的就是根目录前缀（含结尾斜杠）
    const prefix = f.abs.endsWith(f.rel) ? f.abs.slice(0, f.abs.length - f.rel.length) : "";
    let cur = root;
    for (let i = 0; i < parts.length - 1; i++) {
      const dirName = parts[i];
      let next = cur.children!.find((c) => c.dir && c.name === dirName);
      if (!next) {
        next = {
          name: dirName,
          abs: dirAbs(prefix, parts.slice(0, i + 1)),
          dir: true,
          children: [],
        };
        cur.children!.push(next);
      }
      cur = next;
    }
    cur.children!.push({ name: parts[parts.length - 1], abs: f.abs, dir: false });
  }
  const sortRec = (n: DocNode) => {
    if (!n.children) return;
    n.children.sort((a, b) => Number(b.dir) - Number(a.dir) || a.name.localeCompare(b.name));
    n.children.forEach(sortRec);
  };
  sortRec(root);
  return root.children ?? [];
}
