//! 「未检出」判定的回归测试。
//!
//! 这里钉的是**安全性方向**：判定要「失败开放」——
//! 没读到目录清单时宁可让按钮可用，也不能把有用的按钮全禁掉。
//! 反过来的话，用户看到的是「全都是灰的」，比「点了没反应」更难排查。

import { describe, expect, it } from "vitest";

import { checkedOut, confirmedEmpty, type EnvDirs } from "./envDirs";

const dirs: EnvDirs = { dev: ["demo-payment", "doc-hub"], pre: [] };

describe("checkedOut", () => {
  it("列出来的模块算已检出", () => {
    expect(checkedOut(dirs, "dev", "demo-payment")).toBe(true);
    expect(checkedOut(dirs, "dev", "doc-hub")).toBe(true);
  });

  it("读到了、但里面没有 → 未检出（环境页那种一个子目录都没有的情况）", () => {
    expect(checkedOut(dirs, "pre", "demo-payment")).toBe(false);
  });

  it("整个环境都没读出来 → 不当成「没有」，按钮照旧可用", () => {
    expect(checkedOut(dirs, "staging", "demo-payment")).toBe(true);
  });

  it("该环境读失败（值为 null）→ 同样不当成「没有」", () => {
    const failed: EnvDirs = { dev: null };
    expect(checkedOut(failed, "dev", "demo-payment")).toBe(true);
  });
});

describe("confirmedEmpty", () => {
  it("读到空目录才算「确认一个都没有」", () => {
    expect(confirmedEmpty(dirs, "pre")).toBe(true);
  });

  it("有东西、或没读到 → 都不弹「还没检出任何仓库」", () => {
    expect(confirmedEmpty(dirs, "dev")).toBe(false);
    expect(confirmedEmpty(dirs, "staging")).toBe(false);
    expect(confirmedEmpty({ dev: null }, "dev")).toBe(false);
  });
});