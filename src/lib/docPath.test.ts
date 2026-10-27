//! 文档目录优先级的回归测试。
//!
//! 实测反馈：已归档（已上线）的文档在 `repos/doc-hub/workspaces/...`，
//! 开发中的在 `ws/<分支>/doc-hub/workspaces/...`。
//! **优先取生产环境那份，取不到再退工作区那份** —— 显示与「文件管理」必须是同一个答案。

import { describe, expect, it } from "vitest";

import { docDirCandidates, docDirs, docWhereLabel } from "./docPath";

const DOC = "doc-hub";
const ROOT = "/hub";
const FAMILY = "示例需求";
const BRANCH = "feature-20261013-private-agent-v3";

describe("docDirs", () => {
  it("两份拷贝各自的路径", () => {
    const d = docDirs(ROOT, DOC, FAMILY, BRANCH);
    expect(d.main).toBe(
      "/hub/repos/doc-hub/workspaces/示例需求/feature-20261013-private-agent-v3",
    );
    expect(d.ws).toBe(
      "/hub/ws/feature-20261013-private-agent-v3/doc-hub/workspaces/示例需求/feature-20261013-private-agent-v3",
    );
  });
});

describe("docDirCandidates", () => {
  it("已归档（已上线）→ 先生产环境，再工作区", () => {
    const c = docDirCandidates(ROOT, DOC, FAMILY, BRANCH, true);
    expect(c.map((x) => x.where)).toEqual(["main", "ws"]);
    expect(c[0].dir).toBe(docDirs(ROOT, DOC, FAMILY, BRANCH).main);
  });

  it("开发中 → 先工作区，再生产环境", () => {
    const c = docDirCandidates(ROOT, DOC, FAMILY, BRANCH, false);
    expect(c.map((x) => x.where)).toEqual(["ws", "main"]);
    expect(c[0].dir).toBe(docDirs(ROOT, DOC, FAMILY, BRANCH).ws);
  });

  it("两份都要给出来 —— 界面靠它「取不到就退下一个」，不是猜一个路径", () => {
    expect(docDirCandidates(ROOT, DOC, FAMILY, BRANCH, true)).toHaveLength(2);
    expect(docDirCandidates(ROOT, DOC, FAMILY, BRANCH, false)).toHaveLength(2);
  });
});

describe("docWhereLabel", () => {
  it("两份的说明要不一眼混起来", () => {
    expect(docWhereLabel("main")).toContain("main");
    expect(docWhereLabel("ws")).toContain("工作区");
    expect(docWhereLabel("main")).not.toBe(docWhereLabel("ws"));
  });
});
