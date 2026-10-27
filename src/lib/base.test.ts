//! 基线显示的回归测试。

import { describe, expect, it } from "vitest";

import { baseChainLabel, hasBase } from "./base";

describe("baseChainLabel", () => {
  it("末尾补上「主干」——主干是兜底，不补就看不出落到哪", () => {
    expect(baseChainLabel(["ws:feature-v2"])).toBe("feature-v2 → 主干");
  });

  it("去掉 ws: 前缀（那是命令行 --from 的写法）", () => {
    expect(baseChainLabel(["ws:feature-20260818-ca-mobile-vault-v2"]))
      .toBe("feature-20260818-ca-mobile-vault-v2 → 主干");
  });

  it("多基线**保持优先级顺序**（链上第一个命中的先用）", () => {
    expect(baseChainLabel(["ws:phase2", "ws:phase1"])).toBe("phase2 → phase1 → 主干");
  });

  it("非 ws: 的原样显示（比如直接写 ref）", () => {
    expect(baseChainLabel(["main"])).toBe("main → 主干");
  });

  it("没填基线时就是纯兜底", () => {
    expect(baseChainLabel([])).toBe("主干");
  });

  it("忽略空项与只有空白的项（别显示成 ` → 主干`）", () => {
    expect(baseChainLabel(["", "  ", "ws:a"])).toBe("a → 主干");
  });
});

describe("hasBase", () => {
  it("空 / 只有空白算没填", () => {
    expect(hasBase([])).toBe(false);
    expect(hasBase(["", "   "])).toBe(false);
  });

  it("填了就算有", () => {
    expect(hasBase(["ws:a"])).toBe(true);
  });
});
