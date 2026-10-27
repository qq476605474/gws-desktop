//! 需求分组的回归测试。

import { describe, expect, it } from "vitest";

import { groupByFamily, monthGroupName, UNCLASSIFIED } from "./wsGroups";

const w = (title: string, doc_family: string | null, created = "2026-10-14") => ({
  title,
  doc_family,
  created,
});

describe("groupByFamily", () => {
  it("同一需求族的不同期次归到一组（用户的实际场景）", () => {
    const g = groupByFamily([
      w("增长手机号加密2.3", "增长手机号加密"),
      w("增长手机号加密3期", "增长手机号加密"),
      w("示例需求v1.3", "示例需求"),
    ]);
    expect(g.map((x) => x.family)).toEqual(["增长手机号加密", "示例需求"]);
    expect(g[0].items.map((x) => x.title)).toEqual(["增长手机号加密2.3", "增长手机号加密3期"]);
    expect(g[1].items).toHaveLength(1);
  });

  it("没填分类的**按创建年月**归组，不是一个巨大的「未分类」桶", () => {
    // 实测反馈：一律归「未分类」的话，那个桶会越攒越大，分组就没意义了
    const g = groupByFamily([
      w("甲", null, "2026-10-14"),
      w("乙", "有族", "2026-10-01"),
      w("丙", null, "2026-10-02"),
      w("丁", null, "2026-09-30"),
    ]);
    expect(g.map((x) => x.family)).toEqual(["2026-10", "有族", "2026-09"]);
    expect(g[0].items.map((x) => x.title)).toEqual(["甲", "丙"]);
    expect(g[2].items.map((x) => x.title)).toEqual(["丁"]);
  });

  it("只有空白也算没填（后端可能给空串）", () => {
    const g = groupByFamily([w("甲", "   "), w("乙", "")]);
    expect(g).toHaveLength(1);
    expect(g[0].family).toBe("2026-10");
    expect(g[0].items).toHaveLength(2);
  });

  it("连创建日期都没有才退回「未分类」（正常数据不会走到）", () => {
    const g = groupByFamily([w("甲", null, "")]);
    expect(g[0].family).toBe(UNCLASSIFIED);
  });

  it("分组不打乱顺序：组按首项位置排，组内保持原序", () => {
    const g = groupByFamily([
      w("a", "族1"),
      w("b", "族2"),
      w("c", "族1"),
      w("d", "族2"),
    ]);
    expect(g.map((x) => x.family)).toEqual(["族1", "族2"]);
    expect(g[0].items.map((x) => x.title)).toEqual(["a", "c"]);
    expect(g[1].items.map((x) => x.title)).toEqual(["b", "d"]);
  });

  it("空列表不炸", () => {
    expect(groupByFamily([])).toEqual([]);
  });
});

describe("monthGroupName", () => {
  it("从 `YYYY-MM-DD` 取年月", () => {
    expect(monthGroupName("2026-10-14")).toBe("2026-10");
    expect(monthGroupName(" 2026-01-01 ")).toBe("2026-01");
  });

  it("格式不对就退回未分类（不猜）", () => {
    expect(monthGroupName("")).toBe(UNCLASSIFIED);
    expect(monthGroupName(undefined)).toBe(UNCLASSIFIED);
    expect(monthGroupName("10/14/2026")).toBe(UNCLASSIFIED);
  });
});
