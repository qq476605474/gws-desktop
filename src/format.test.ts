//! 合并结果的文案 —— 重点是**「合到哪条分支」不能拿环境标识去报**。
//!
//! 实测反馈：需求合到灰度环境（标识 `pre`，覆盖表里 demo-api 写的是
//! `pre-202610`），界面却提示「已合入并推送 **origin/pre**」—— 人看到的
//! 和实际发生的对不上，比功能错更吓人（那条分支是上个月的）。

import { describe, expect, it } from "vitest";

import { mergeLines } from "./format";
import type { MergeOutcome, PendingReason } from "./types";

function outcome(over: Partial<MergeOutcome> = {}): MergeOutcome {
  return {
    env: "pre",
    req_branch: "feature-x",
    stale: [],
    pending: [],
    clean: [],
    skipped: [],
    conflicts: [],
    conflict_dirs: {},
    conflict_files: {},
    resumed_conflicts: [],
    pushed: [],
    push_failed: [],
    did_push: false,
    branches: {},
    ...over,
  };
}

const text = (o: MergeOutcome) => mergeLines(o).join("\n");

describe("mergeLines · pending 的几种原因不能混为一谈", () => {
  // 实测 BUG：远程 dev 被 force 重建后，本地 HEAD 一动没动，
  // 界面却提示「已合并到本地 … 确认后再勾选推送」——按提示去推必然非快进失败。
  const diverged: PendingReason = {
    kind: "env_diverged",
    ahead: 3288,
    behind: 12,
    rebuilt: true,
  };

  it("分叉（远程被重建）时说「本轮未合并 + 需要强制重建」，绝不说「已合并」", () => {
    const t = text(
      outcome({
        pending: [["demo-api", diverged]],
        branches: { "demo-api": "dev" },
      }),
    );
    expect(t).toContain("本轮未合并");
    expect(t).toContain("force push");
    expect(t).toContain("强制重建");
    expect(t).toContain("origin/dev");
    expect(t).not.toContain("已合并到本地");
  });

  it("分叉时用的仍是**实际环境分支**（pre-202610），不是环境标识", () => {
    const t = text(
      outcome({
        pending: [["demo-api", diverged]],
        branches: { "demo-api": "pre-202610" },
      }),
    );
    expect(t).toContain("origin/pre-202610");
    expect(t).not.toContain("origin/pre，");
  });

  it("本地已有未推送成果（远程没动）时说「本轮未重新合并」，不冒充本轮合过", () => {
    const t = text(
      outcome({ pending: [["a", { kind: "env_ahead", ahead: 3 }]] }),
    );
    expect(t).toContain("本轮未重新合并");
    expect(t).toContain("3 个未推送提交");
    expect(t).not.toContain("已合并到本地");
  });

  it("远程被回退（回退点是本地祖先）时说「未合并 + 强制重建」，不能快进推旧历史", () => {
    const t = text(
      outcome({
        pending: [["a", { kind: "env_rolled_back", ahead: 5 }]],
        branches: { a: "dev" },
      }),
    );
    expect(t).toContain("被回退过");
    expect(t).toContain("一并恢复");
    expect(t).toContain("强制重建");
    expect(t).toContain("本轮没有合并任何模块");
    expect(t).not.toContain("已合并到本地");
  });

  it("需求分支有未推送提交时提示先推送", () => {
    const t = text(
      outcome({
        clean: ["a"],
        pending: [["a", { kind: "requirement_unpushed", ahead: 2 }]],
      }),
    );
    expect(t).toContain("不含需求分支未推送的提交");
    expect(t).toContain("推送需求分支");
  });

  it("真的合入了（clean 非空）才说「已合并到本地」", () => {
    const t = text(outcome({ clean: ["a"] }));
    expect(t).toContain("已合并到本地 envs/pre");
  });

  it("只有跳过的模块时维持「没有任何模块合入」", () => {
    const t = text(outcome({ skipped: [["a", "该仓库没有 pre 分支"]] }));
    expect(t).toContain("没有任何模块合入");
    expect(t).not.toContain("已合并到本地");
  });
});

describe("mergeLines · 推送目标", () => {
  it("报的是**实际分支**（pre-202610），不是环境标识（pre）", () => {
    const t = text(
      outcome({
        did_push: true,
        pushed: ["demo-api"],
        branches: { "demo-api": "pre-202610" },
      }),
    );
    expect(t).toContain("origin/pre-202610");
    expect(t).not.toContain("origin/pre（");
  });

  it("各模块分支不同时逐个列出，不挑一个代表", () => {
    const t = text(
      outcome({
        did_push: true,
        pushed: ["demo-api", "demo-app"],
        branches: { "demo-api": "pre-202610", "demo-app": "pre" },
      }),
    );
    expect(t).toContain("origin/pre-202610");
    expect(t).toContain("origin/pre");
  });

  it("拿不到分支信息时退回环境标识（不显示成空白）", () => {
    const t = text(outcome({ did_push: true, pushed: ["x"], branches: {} }));
    expect(t).toContain("origin/pre");
  });
});

describe("mergeLines · 冲突现场", () => {
  it("上次没解完的冲突要标明「现场保留」，并列出文件", () => {
    const t = text(
      outcome({
        conflicts: ["a"],
        resumed_conflicts: ["a"],
        conflict_files: { a: ["src/x.ts"] },
        conflict_dirs: { a: "/hub/envs/pre/a" },
      }),
    );
    expect(t).toContain("上次没解完");
    expect(t).toContain("src/x.ts");
    expect(t).toContain("/hub/envs/pre/a");
  });

  it("新产生的冲突同样列出文件，但不冒充「上次没解完」", () => {
    const t = text(
      outcome({ conflicts: ["b"], conflict_files: { b: ["pom.xml"] } }),
    );
    expect(t).toContain("pom.xml");
    expect(t).not.toContain("上次没解完");
  });
});