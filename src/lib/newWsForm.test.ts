//! 「新建需求」表单规则的回归测试。
//!
//! 实测反馈：第一栏「名称」其实是**分支后缀**，却被当成了中文名；
//! 选了「完整分支名」之后这一栏还用不上，却仍然必填卡人。
//! 这里逐条钉住修好后的行为。

import { describe, expect, it } from "vitest";

import {
  composeBranch,
  canCreate,
  needsSuffix,
  resolveBranch,
  resolveTitle,
  stripLeadingDates,
  TITLE_PLACEHOLDER,
  type NewWsForm,
} from "./newWsForm";

function form(over: Partial<NewWsForm> = {}): NewWsForm {
  return {
    mode: "compose",
    suffix: "收银台改版",
    prefix: "feature",
    date: "20261015",
    fullBranch: "",
    title: "",
    moduleCount: 1,
    ...over,
  };
}

describe("needsSuffix", () => {
  it("前缀组合模式要后缀", () => {
    expect(needsSuffix("compose")).toBe(true);
  });

  it("完整分支名模式不要 —— 这一栏整栏不显示", () => {
    expect(needsSuffix("full")).toBe(false);
  });
});

describe("canCreate", () => {
  it("完整分支名模式：只填了分支名就能建（不再被后缀卡住）", () => {
    const f = form({ mode: "full", suffix: "", fullBranch: "feature-20261010-checkout-revamp" });
    expect(canCreate(f)).toBe(true);
  });

  it("完整分支名模式：什么都不填不能建", () => {
    expect(canCreate(form({ mode: "full", suffix: "", fullBranch: "  " }))).toBe(false);
  });

  it("前缀组合模式：没有后缀不能建", () => {
    expect(canCreate(form({ suffix: "" }))).toBe(false);
  });

  it("一个模块都没选不能建", () => {
    expect(canCreate(form({ moduleCount: 0 }))).toBe(false);
    expect(
      canCreate(form({ mode: "full", suffix: "", fullBranch: "feature-x", moduleCount: 0 })),
    ).toBe(false);
  });
});

describe("resolveBranch", () => {
  it("前缀组合：前缀-日期-后缀", () => {
    expect(resolveBranch(form())).toBe("feature-20261015-收银台改版");
  });

  it("完整分支名：原样用，不拼接", () => {
    const f = form({ mode: "full", fullBranch: " feature-20261010-checkout-revamp " });
    expect(resolveBranch(f)).toBe("feature-20261010-checkout-revamp");
  });

  it("预览时后缀还没填 → 显示占位，不是 `feature-20261015-`", () => {
    expect(composeBranch("feature", "20261015", "")).toBe("feature-20261015-<后缀>");
  });

  // 实测真实分支：bugfix-20261020-20261020-ca-mobile-vault-2.3-bugfix
  it("后缀里已经写了日期 → 拼接时去掉，避免出现两个日期", () => {
    expect(composeBranch("bugfix", "20261020", "20261020-ca-mobile-vault-2.3-bugfix")).toBe(
      "bugfix-20261020-ca-mobile-vault-2.3-bugfix",
    );
    expect(composeBranch("feature", "20261020", "261020-ca")).toBe("feature-20261020-ca");
    expect(composeBranch("feature", "20261020", "2026-10-20-ca")).toBe("feature-20261020-ca");
    expect(composeBranch("feature", "20261020", "2026-10-ca")).toBe("feature-20261020-ca");
    // 连着写了几个也去掉
    expect(composeBranch("feature", "20261020", "2026-10-20-20261020-ca")).toBe(
      "feature-20261020-ca",
    );
  });

  it("后缀只有一个日期 → 不再多拼一段，也不会留下空尾巴", () => {
    expect(composeBranch("feature", "20261020", "20261020")).toBe("feature-20261020");
    expect(composeBranch("feature", "20261020", "2026-10-20")).toBe("feature-20261020");
  });

  it("不是日期的前缀原样保留", () => {
    expect(stripLeadingDates("ca-mobile-vault")).toEqual({
      normalized: "ca-mobile-vault",
      stripped: false,
    });
    expect(stripLeadingDates("20261340-ca").normalized).toBe("20261340-ca");
    expect(composeBranch("feature", "20261020", "ca-mobile-vault")).toBe(
      "feature-20261020-ca-mobile-vault",
    );
  });
});

describe("resolveTitle", () => {
  it("填了就用填的", () => {
    expect(resolveTitle("收银台改版二期", "feature-20261015-收银台改版")).toBe("收银台改版二期");
  });

  it("留空 → 取**完整分支名**（后缀容易重名，两行同名就分不清了）", () => {
    expect(resolveTitle("", "feature-20261015-收银台改版")).toBe("feature-20261015-收银台改版");
    expect(resolveTitle("   ", "feature-20261015-收银台改版")).toBe("feature-20261015-收银台改版");
  });

  it("两种分支模式用同一套兜底", () => {
    const branch = "feature-20261010-checkout-revamp";
    expect(resolveTitle("", branch)).toBe(branch);
    expect(TITLE_PLACEHOLDER).toBe("留空就用分支名");
  });
});