//! 合并结果的文案 —— 重点是**「合到哪条分支」不能拿环境标识去报**。
//!
//! 实测反馈：需求合到灰度环境（标识 `pre`，覆盖表里 demo-api 写的是
//! `pre-202610`），界面却提示「已合入并推送 **origin/pre**」—— 人看到的
//! 和实际发生的对不上，比功能错更吓人（那条分支是上个月的）。

import { describe, expect, it } from "vitest";

import { mergeLines } from "./format";
import type { MergeOutcome } from "./types";

function outcome(over: Partial<MergeOutcome> = {}): MergeOutcome {
  return {
    env: "pre",
    stale: [],
    pending: [],
    clean: [],
    skipped: [],
    conflicts: [],
    conflict_dirs: {},
    pushed: [],
    push_failed: [],
    did_push: false,
    branches: {},
    ...over,
  };
}

const text = (o: MergeOutcome) => mergeLines(o).join("\n");

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