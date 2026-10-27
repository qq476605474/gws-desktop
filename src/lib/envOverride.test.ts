//! 「分支覆盖」候选列表的回归测试。

import { describe, expect, it } from "vitest";

import { overrideRepoOptions } from "./envOverride";

const repos = [{ name: "demo-web" }, { name: "demo-api" }];

describe("overrideRepoOptions", () => {
  it("**不给「所有仓库」这个通配** —— 必须逐个仓库指定", () => {
    // 实测反馈：通配盖上去后，没有该分支的仓库会被静默跳过，人就以为都换了
    const opts = overrideRepoOptions(repos, "");
    expect(opts.map((o) => o.value)).toEqual(["demo-web", "demo-api"]);
    expect(opts.some((o) => o.value === "*")).toBe(false);
  });

  it("这一行本来就是 `*`（老数据）→ 仍然列出来，否则改都没法改", () => {
    const opts = overrideRepoOptions(repos, "*");
    expect(opts[0].value).toBe("*");
    expect(opts.map((o) => o.value)).toContain("demo-api");
  });

  it("仓库为空时不炸", () => {
    expect(overrideRepoOptions([], "")).toEqual([]);
  });
});
