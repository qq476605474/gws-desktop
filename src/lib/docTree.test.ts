//! 文档树节点判定的回归测试。

import { describe, expect, it } from "vitest";

import { isEmptyDir, isLeafNode, nodesFromFiles } from "./docTree";
import type { DocNode } from "../types";

const file = (name: string): DocNode => ({ name, abs: `/p/${name}`, dir: false });
const dir = (name: string, children: DocNode[] = []): DocNode => ({
  name,
  abs: `/p/${name}`,
  dir: true,
  children,
});

describe("isLeafNode", () => {
  it("文件是叶子", () => {
    expect(isLeafNode(file("a.md"))).toBe(true);
  });

  it("**空目录必须是叶子** —— 否则展开会一直转圈（踩过的坑）", () => {
    expect(isLeafNode(dir("sql", []))).toBe(true);
    // children 是 undefined 也一样（后端可能不给这个字段）
    expect(isLeafNode({ name: "sql", abs: "/p/sql", dir: true })).toBe(true);
  });

  it("有子项的目录不是叶子（要能展开）", () => {
    expect(isLeafNode(dir("sql", [file("a.sql")]))).toBe(false);
  });
});

describe("isEmptyDir", () => {
  it("只对「目录且没子项」为真", () => {
    expect(isEmptyDir(dir("sql"))).toBe(true);
    expect(isEmptyDir(dir("sql", [file("a.sql")]))).toBe(false);
    expect(isEmptyDir(file("a.md"))).toBe(false);
  });
});

describe("nodesFromFiles", () => {
  const f = (rel: string, abs?: string) => ({
    name: rel.split("/").pop()!,
    rel,
    abs: abs ?? `/docs/${rel}`,
  });

  it("把扁平列表拼成目录树，目录在前、同类按名排", () => {
    const nodes = nodesFromFiles([f("b.md"), f("sql/a.sql"), f("a.md"), f("sql/c.sql")]);
    expect(nodes.map((n) => n.name)).toEqual(["sql", "a.md", "b.md"]);
    expect(nodes[0].dir).toBe(true);
    expect(nodes[0].children!.map((c) => c.name)).toEqual(["a.sql", "c.sql"]);
  });

  it("目录节点的 abs 由根前缀推出（不必后端另给目录清单）", () => {
    const nodes = nodesFromFiles([f("sql/a.sql")]);
    expect(nodes[0].abs).toBe("/docs/sql");
    expect(nodes[0].children![0].abs).toBe("/docs/sql/a.sql");
  });

  it("多层目录也能拼", () => {
    const nodes = nodesFromFiles([f("a/b/c.md")]);
    expect(nodes[0].name).toBe("a");
    expect(nodes[0].children![0].name).toBe("b");
    expect(nodes[0].children![0].children![0].name).toBe("c.md");
  });

  it("滤掉系统垃圾文件（.DS_Store 之类）", () => {
    const nodes = nodesFromFiles([f("a.md"), f(".DS_Store"), f("sub/.DS_Store")]);
    expect(nodes.map((n) => n.name)).toEqual(["a.md"]);
  });

  it("空列表不炸", () => {
    expect(nodesFromFiles([])).toEqual([]);
  });
});

describe("nodesFromFiles 的空目录", () => {
  it("空目录也要出现在树里（`sql/` 刚建出来是空的，看不见它就等于没提示）", () => {
    const nodes = nodesFromFiles(
      [{ name: "a.md", rel: "a.md", abs: "/d/a.md" }],
      ["sql"],
      "/d",
    );
    expect(nodes.map((n) => n.name)).toEqual(["sql", "a.md"]);
    expect(nodes[0].dir).toBe(true);
    expect(nodes[0].children).toEqual([]);
  });

  it("空目录的 abs 也拼得对", () => {
    const nodes = nodesFromFiles([], ["sql"], "/d");
    expect(nodes[0].abs).toBe("/d/sql");
  });

  it("空目录里的垃圾不算数（.DS_Store 不算文件）", () => {
    // emptyDirs 里带垃圾路径要被滤掉，不该凭空造节点
    expect(nodesFromFiles([], [".DS_Store"], "/d")).toEqual([]);
  });
});

describe("Windows 形态的路径（实测反馈：Windows 包里文档没有树形）", () => {
  it("rel 用 / 分隔、abs 用 \\ 时也要能建出树（并且目录的 abs 是绝对的）", () => {
    // 后端已把 rel 统一成 `/`（见 Rust 侧 `to_slash`），而 abs 在 Windows 上仍是
    // `C:\hub\...\原型\index.html`。原来的实现靠"abs 以 rel 结尾"反推根前缀 ——
    // Windows 上必然切不出来，目录的 abs 就成了相对路径。
    const root = String.raw`C:\hub\ws\feature-x\doc-hub\workspaces\示例\v1`;
    const nodes = nodesFromFiles(
      [
        {
          name: "index.html",
          rel: "原型/index.html",
          abs: root + String.raw`\原型\index.html`,
        },
      ],
      [],
      root,
    );
    expect(nodes.map((n) => n.name)).toEqual(["原型"]);
    expect(nodes[0].dir).toBe(true);
    expect(nodes[0].abs).toBe("C:/hub/ws/feature-x/doc-hub/workspaces/示例/v1/原型");
    expect(nodes[0].children![0].name).toBe("index.html");
  });

  it("根的结尾分隔符（/ 或 \\）不该拼出双斜杠", () => {
    const nodes = nodesFromFiles(
      [{ name: "a.md", rel: "sub/a.md", abs: String.raw`C:\d\sub\a.md` }],
      [],
      "C:\\d\\",
    );
    expect(nodes[0].abs).toBe("C:/d/sub");
  });
});
