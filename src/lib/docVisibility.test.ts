import { describe, expect, it } from "vitest";

import { visiblePeriods } from "./docVisibility";
import type { Family, PeriodWithState } from "../types";

function period(over: Partial<PeriodWithState>): PeriodWithState {
  return {
    schema: 1,
    branch: "feature-x",
    family: "族",
    name: "需求",
    label: "V1",
    date: "2026-10-01",
    owner: "",
    summary: "",
    repos: [],
    links: [],
    notes: "",
    merged: false,
    has_content: true,
    available: true,
    ...over,
  };
}

const family: Family = {
  name: "巨量单元状态同步",
  periods: [
    period({ branch: "empty", has_content: false, date: "2026-10-21" }),
    period({ branch: "remote-only", available: false, date: "2026-10-20" }),
    period({ branch: "old", merged: true, date: "2026-10-10" }),
    period({ branch: "new", date: "2026-10-19" }),
  ],
};

describe("visiblePeriods", () => {
  it("空期次与本地打不开的期次都不出现", () => {
    // 实测反馈：只剩脚手架的 fake 期次、以及只在远程/环境里的
    // 「巨量单元状态同步」，点开是空的却一直列着
    const out = visiblePeriods(family, "");
    expect(out.map((p) => p.branch)).toEqual(["new", "old"]);
  });

  it("开发中在前、已归档在后，各自时间倒序", () => {
    const out = visiblePeriods(family, "");
    expect(out[0]!.branch).toBe("new");
    expect(out[1]!.merged).toBe(true);
  });

  it("搜索词命中分支名", () => {
    expect(visiblePeriods(family, "remote")).toEqual([]);
    expect(visiblePeriods(family, "new").map((p) => p.branch)).toEqual(["new"]);
  });

  it("搜索词命中族名时整族都算", () => {
    const out = visiblePeriods(family, "巨量");
    expect(out).toHaveLength(2);
  });
});
