import { describe, expect, it } from "vitest";

import { buildCommitMessage, COMMIT_TYPES } from "./conventional";

describe("Conventional Commits 填写工具", () => {
  it("按 <type>(<scope>): <description> 拼出标题", () => {
    expect(
      buildCommitMessage({ type: "feat", scope: "payment", summary: "支持分次支付" }),
    ).toBe("feat(payment): 支持分次支付");
    expect(buildCommitMessage({ type: "fix", summary: "修正金额四舍五入" })).toBe(
      "fix: 修正金额四舍五入",
    );
  });

  it("正文另起一段；不兼容变更同时加 ! 和 footer", () => {
    expect(
      buildCommitMessage({
        type: "refactor",
        scope: "checkout",
        summary: "收敛下单入口",
        body: "旧入口保留一个版本，便于灰度回滚。",
        breaking: true,
        breakingNote: "下单接口的返回字段改了名字",
      }),
    ).toBe(
      "refactor(checkout)!: 收敛下单入口\n\n" +
        "- 旧入口保留一个版本，便于灰度回滚。\n\n" +
        "BREAKING CHANGE: 下单接口的返回字段改了名字",
    );
  });

  it("详细说明一行一条，自动补 `- `，已有前缀和空行不动", () => {
    expect(
      buildCommitMessage({
        type: "feat",
        summary: "收银台改版",
        body: "支持分次支付\n- 已支持退款\n\n1. 修复金额显示\n* 新增发票入口",
      }),
    ).toBe(
      "feat: 收银台改版\n\n" +
        "- 支持分次支付\n- 已支持退款\n1. 修复金额显示\n* 新增发票入口",
    );
  });

  it("描述留空返回空串（由调用方回退到默认说明）", () => {
    expect(buildCommitMessage({ type: "feat", summary: "   " })).toBe("");
  });

  it("类型清单覆盖日常会用到的那几类", () => {
    const values = COMMIT_TYPES.map((t) => t.value);
    for (const v of ["feat", "fix", "docs", "refactor", "test", "chore", "revert"]) {
      expect(values).toContain(v);
    }
  });
});