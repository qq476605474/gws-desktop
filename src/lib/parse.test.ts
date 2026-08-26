import { describe, it, expect } from "vitest";
import { parseLs, parseSt, parseRepoLs, parseEnvLs, parseDocLs, parseVersion } from "./parse";

const D = "\u001b[2m", N = "\u001b[0m";

describe("parseLs", () => {
  it("解析列表（含空标题列）", () => {
    const out =
      "名称                   标题                           阶段     模块   分支\n" +
      "checkout-revamp        结算流程改版                   dev      3      " + D + "feature-20260818-checkout-revamp" + N + "\n" +
      "login-crash            login-crash                    dev      1      " + D + "hotfix-20260821-login-crash" + N + "\n";
    expect(parseLs(out)).toEqual([
      { name: "checkout-revamp", title: "结算流程改版", stage: "dev", modules: 3, branch: "feature-20260818-checkout-revamp" },
      { name: "login-crash", title: "login-crash", stage: "dev", modules: 1, branch: "hotfix-20260821-login-crash" },
    ]);
  });
  it("空 hub 返回 []", () => {
    expect(parseLs(D + "(暂无工作区，用 gws new 创建)" + N)).toEqual([]);
  });
});

describe("parseSt", () => {
  it("解析模块状态表", () => {
    const B = "\u001b[34m", Y = "\u001b[33m", G = "\u001b[32m";
    const out =
      B + "结算流程改版" + N + "  " + D + "feature-20260818-checkout-revamp" + N + "\n\n" +
      "  模块                               改动     vs远程     vs主干\n" +
      "  order-service                      " + G + "0       " + N + " ↑2 ↓0     " + D + "+5" + N + "\n" +
      "  user-service                       " + Y + "3       " + N + " " + Y + "未推送" + N + "     " + D + "+2" + N + "\n" +
      "  admin-web                          " + "\u001b[31m目录缺失 " + D + "(gws drop 清理)" + N + "\n";
    expect(parseSt(out)).toEqual({
      title: "结算流程改版",
      branch: "feature-20260818-checkout-revamp",
      modules: [
        { name: "order-service", dirty: 0, ahead: 2, behind: 0, pushed: true, aheadOfMain: 5 },
        { name: "user-service", dirty: 3, ahead: null, behind: null, pushed: false, aheadOfMain: 2 },
        { name: "admin-web", missing: true },
      ],
    });
  });
});

describe("parseRepoLs", () => {
  it("解析仓库与主干", () => {
    const out =
      "仓库                                     主干\n" +
      "order-service                            " + D + "main" + N + "\n" +
      "admin-web                                " + D + "master" + N + "\n" +
      D + "共 2 个" + N + "\n";
    expect(parseRepoLs(out)).toEqual([
      { name: "order-service", mainBranch: "main" },
      { name: "admin-web", mainBranch: "master" },
    ]);
  });
});

describe("parseEnvLs", () => {
  it("解析真实 env ls 输出（含表头与装饰）", () => {
    const out =
      "\u001b[34m环境分支\u001b[0m   \u001b[2m(即 envs/ 下的目录，gws env add/rm 增删)\u001b[0m\n" +
      "  \u001b[2m○\u001b[0m dev  \u001b[2m(无模块，跑 gws sync)\u001b[0m\n" +
      "  \u001b[32m●\u001b[0m pre  \u001b[2m(3 个模块)\u001b[0m\n";
    expect(parseEnvLs(out)).toEqual(["dev", "pre"]);
  });
  it("空环境返回 []", () => {
    expect(parseEnvLs("  \u001b[2m(暂无环境，用 gws env add dev pre 创建)\u001b[0m\n")).toEqual([]);
  });
});

describe("parseDocLs", () => {
  it("解析同步状态", () => {
    const B = "\u001b[34m", G = "\u001b[32m";
    const out =
      B + "2026-08-18-checkout-revamp" + N + "\n" +
      "  " + G + "●" + N + " 技术方案.md  " + D + "wiki:123" + N + "\n" +
      "  " + D + "○" + N + " 排期.md  " + D + "(未上传)" + N;
    expect(parseDocLs(out)).toEqual([
      { file: "技术方案.md", synced: true, pageId: "123" },
      { file: "排期.md", synced: false, pageId: null },
    ]);
  });
  it("文件名含空格的行仍被解析（file 保留空格，状态列不被吞进文件名）", () => {
    const B = "\u001b[34m", G = "\u001b[32m";
    const out =
      B + "2026-08-18-checkout-revamp" + N + "\n" +
      "  " + G + "●" + N + " my plan.md  " + D + "wiki:123" + N + "\n" +
      "  " + D + "○" + N + " draft v2.md  " + D + "(未上传)" + N;
    expect(parseDocLs(out)).toEqual([
      { file: "my plan.md", synced: true, pageId: "123" },
      { file: "draft v2.md", synced: false, pageId: null },
    ]);
  });
});

describe("parseVersion", () => {
  it("提取版本号", () => {
    expect(parseVersion("gws 0.4.2  " + D + "(github.com/qq476605474/gws)" + N)).toBe("0.4.2");
  });
});
