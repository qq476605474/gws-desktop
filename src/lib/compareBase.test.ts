//! 对比基准的回归测试。
//!
//! 实测反馈：**「与主干」永远显示 0 个改动**。根因是「与主干」返回 `null`
//! （意思是"后端自己探测主干"），而调用方把 `null` 当成"目标没选全"，
//! **请求根本没发**。这里把三种返回值的区别钉死。

import { describe, expect, it } from "vitest";

import { compareBase, isReady, type BaseInput } from "./compareBase";

const src = { repo: "demo-api", branch: "feature-20260818-ca-mobile-vault-v2.3" };

function input(over: Partial<BaseInput> = {}): BaseInput {
  return { target: "main", source: src, branchInput: "", env: "", ...over };
}

describe("compareBase", () => {
  it("与主干 → undefined（交给后端探测），**不是** null", () => {
    expect(compareBase(input({ target: "main" }))).toBeUndefined();
  });

  it("与本地 → HEAD（未提交的改动）；分支没推送也能比", () => {
    // 实测反馈：分支还没推送时「与远程」的 origin/<分支> 不存在，只剩一句报错，
    // 本地明明改了东西却什么都看不到 —— 所以默认目标改成这个。
    expect(compareBase(input({ target: "worktree" }))).toBe("HEAD");
    expect(isReady(compareBase(input({ target: "worktree" })))).toBe(true);
  });

  it("与远程 → origin/<同名分支>", () => {
    expect(compareBase(input({ target: "remote" }))).toBe(
      "origin/feature-20260818-ca-mobile-vault-v2.3",
    );
  });

  it("与环境 → origin/<该环境在此仓库的分支>；没选环境就是「没选全」", () => {
    expect(compareBase(input({ target: "env", env: "dev", envBranch: "dev" }))).toBe("origin/dev");
    expect(compareBase(input({ target: "env", env: "" }))).toBeNull();
    // 选了环境但查不到该仓库的分支 → 也算没选全（不能退化成主干，标题会骗人）
    expect(compareBase(input({ target: "env", env: "dev", envBranch: undefined }))).toBeNull();
  });

  it("与远程分支 → 填了才算，没填是 null", () => {
    expect(compareBase(input({ target: "branch", branchInput: " dev " }))).toBe("origin/dev");
    expect(compareBase(input({ target: "branch", branchInput: "   " }))).toBeNull();
  });

  it("与本地分支 → 不带 origin/", () => {
    expect(compareBase(input({ target: "local", branchInput: "main" }))).toBe("main");
    expect(compareBase(input({ target: "local", branchInput: "" }))).toBeNull();
  });

  it("与基线 → 用后端解析出的 ref；还没解析出来是 null（先别请求）", () => {
    expect(
      compareBase(input({ target: "baseline", baselineRef: "origin/feature-20261001-up" })),
    ).toBe("origin/feature-20261001-up");
    // 没解析出来**不能**返回 undefined —— 那会被当成「与主干」，标题会骗人
    expect(compareBase(input({ target: "baseline" }))).toBeNull();
    expect(isReady(compareBase(input({ target: "baseline" })))).toBe(false);
  });

  it("外部指定了基准（合并检查那种）→ 一切以它为准", () => {
    expect(compareBase(input({ target: "main", override: "origin/pre" }))).toBe("origin/pre");
    expect(compareBase(input({ target: "env", env: "", override: "origin/pre" }))).toBe("origin/pre");
  });
});

describe("isReady", () => {
  it("「与主干」的 undefined 是**可以发请求**的 —— 这就是那个 bug 的分界线", () => {
    expect(isReady(compareBase(input({ target: "main" })))).toBe(true);
  });

  it("具体基准当然可以发", () => {
    expect(isReady("origin/main")).toBe(true);
  });

  it("目标没选全（null）→ 先别请求（免得把「与主干」的结果挂在别的标题下）", () => {
    expect(isReady(null)).toBe(false);
    expect(isReady(compareBase(input({ target: "env", env: "" })))).toBe(false);
  });
});