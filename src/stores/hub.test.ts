// 测试跑在 node 环境（无 DOM 依赖）
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

type RunResult = { code: number | null; output: string };

// vi.mock 工厂随 import 提升、早于本文件函数体执行（同 cmd.test.ts），
// mock 函数必须经 vi.hoisted 创建以避免 TDZ。
const mocks = vi.hoisted(() => ({
  runGws: vi.fn<(args: string[], cwd: string) => Promise<RunResult>>(),
}));

vi.mock("../lib/gws-bridge", () => ({
  runGws: mocks.runGws,
}));

import { useHubStore } from "./hub";

// 夹具对齐 gws 真实输出形态（无 ANSI 亦可被 parse 系列处理）
const LS_OUT =
  "名称                   标题             阶段     模块   分支\n" +
  "checkout-revamp        结算流程改版     dev      3      feature-20260818-checkout-revamp\n";
const REPO_OUT =
  "仓库                                     主干\n" +
  "order-service                            main\n" +
  "admin-web                                master\n" +
  "共 2 个\n";
const ENV_OUT =
  "环境分支   (即 envs/ 下的目录，gws env add/rm 增删)\n" +
  "  ○ dev  (无模块，跑 gws sync)\n" +
  "  ● pre  (3 个模块)\n";

/** refreshAll 用 Promise.all 并发跑 ls / repo ls / env ls 三条命令，mock 须按 args[0] 分派。 */
function dispatchResults(byCmd: Record<string, RunResult>) {
  mocks.runGws.mockImplementation(async (args: string[]) => byCmd[args[0]] ?? { code: 0, output: "" });
}

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
});

describe("hub store", () => {
  it("成功路径：三命令 code 0 → workspaces/repos/envs 填充、error 为空", async () => {
    dispatchResults({
      ls: { code: 0, output: LS_OUT },
      repo: { code: 0, output: REPO_OUT },
      env: { code: 0, output: ENV_OUT },
    });
    const store = useHubStore();
    store.setHub("/hub");
    await store.refreshAll();
    expect(mocks.runGws).toHaveBeenCalledTimes(3);
    expect(mocks.runGws).toHaveBeenCalledWith(["ls"], "/hub");
    expect(mocks.runGws).toHaveBeenCalledWith(["repo", "ls"], "/hub");
    expect(mocks.runGws).toHaveBeenCalledWith(["env", "ls"], "/hub");
    expect(store.workspaces).toEqual([
      { name: "checkout-revamp", title: "结算流程改版", stage: "dev", modules: 3, branch: "feature-20260818-checkout-revamp" },
    ]);
    expect(store.repos).toEqual([
      { name: "order-service", mainBranch: "main" },
      { name: "admin-web", mainBranch: "master" },
    ]);
    expect(store.envs).toEqual(["dev", "pre"]);
    expect(store.error).toBe("");
  });

  it("spawn 失败（code null）：error 置为该 output，旧数据保留不清空", async () => {
    dispatchResults({
      ls: { code: 0, output: LS_OUT },
      repo: { code: 0, output: REPO_OUT },
      env: { code: 0, output: ENV_OUT },
    });
    const store = useHubStore();
    store.setHub("/hub");
    await store.refreshAll();
    expect(store.workspaces).toHaveLength(1);

    // 第二次刷新时 ls spawn 失败：已填充的数据不得被清空
    dispatchResults({
      ls: { code: null, output: "启动失败: 权限不足" },
      repo: { code: 0, output: REPO_OUT },
      env: { code: 0, output: ENV_OUT },
    });
    await store.refreshAll();
    expect(store.error).toBe("启动失败: 权限不足");
    expect(store.workspaces).toHaveLength(1);
    expect(store.repos).toHaveLength(2);
    expect(store.envs).toEqual(["dev", "pre"]);
  });

  it("非零退出码不算错误：env ls code 1（有环境时的真实怪癖）→ error 为空、envs 正常解析", async () => {
    dispatchResults({
      ls: { code: 0, output: LS_OUT },
      repo: { code: 0, output: REPO_OUT },
      env: { code: 1, output: ENV_OUT },
    });
    const store = useHubStore();
    store.setHub("/hub");
    await store.refreshAll();
    expect(store.error).toBe("");
    expect(store.envs).toEqual(["dev", "pre"]);
    expect(store.workspaces).toHaveLength(1);
  });

  it("invoke reject：error = String(e)，refreshAll 不抛出", async () => {
    mocks.runGws.mockRejectedValue(new Error("invoke 失败"));
    const store = useHubStore();
    store.setHub("/hub");
    await expect(store.refreshAll()).resolves.toBeUndefined();
    expect(store.error).toBe("Error: invoke 失败");
  });

  it("空 path：refreshAll 直接返回，runGws 未被调用", async () => {
    const store = useHubStore();
    await store.refreshAll();
    expect(mocks.runGws).not.toHaveBeenCalled();
  });

  it("setHub 切换：清空 workspaces/repos/envs/error", async () => {
    dispatchResults({
      ls: { code: 0, output: LS_OUT },
      repo: { code: 0, output: REPO_OUT },
      env: { code: 0, output: ENV_OUT },
    });
    const store = useHubStore();
    store.setHub("/hub-a");
    await store.refreshAll();
    expect(store.workspaces).toHaveLength(1);

    store.setHub("/hub-b");
    expect(store.path).toBe("/hub-b");
    expect(store.workspaces).toEqual([]);
    expect(store.repos).toEqual([]);
    expect(store.envs).toEqual([]);
    expect(store.error).toBe("");
  });
});
