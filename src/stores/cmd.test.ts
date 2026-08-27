import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

type Payload = Record<string, unknown>;
type Handler = (e: { payload: Payload }) => void;

// vi.mock 工厂随 import 提升、早于本文件函数体执行，
// 共享状态必须经 vi.hoisted 创建以避免 TDZ。
const mocks = vi.hoisted(() => ({
  runGwsStream: vi.fn<(args: string[], cwd: string, confirmTimeoutMs?: number) => Promise<number>>().mockResolvedValue(1),
  respondConfirm: vi.fn<(runId: number, yes: boolean) => Promise<void>>().mockResolvedValue(undefined),
  replayOutput: vi.fn<(runId: number) => Promise<void>>().mockResolvedValue(undefined),
  /** 按事件名保存 listen 注册的 handler，测试中手动触发以模拟事件流 */
  handlers: new Map<string, Handler>(),
  /** 已拆除订阅的事件名（监听器泄漏断言：run 终态后订阅应收尾） */
  unlistened: [] as string[],
}));

vi.mock("../lib/gws-bridge", () => ({
  runGwsStream: mocks.runGwsStream,
  respondConfirm: mocks.respondConfirm,
  replayOutput: mocks.replayOutput,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: Handler) => {
    mocks.handlers.set(event, handler);
    return () => {
      // 仅拆除本次注册的 handler：上轮测试遗留的清理定时器触发时不误删新一轮订阅
      if (mocks.handlers.get(event) === handler) mocks.handlers.delete(event);
      mocks.unlistened.push(event);
    };
  }),
}));

import { useCmdStore } from "./cmd";

const RUN_ID = 1;

/** 模拟后端发出一个 Tauri 事件（gws-output:<id> / gws-exit:<id> / gws-confirm:<id>） */
function emit(event: string, payload: Payload) {
  mocks.handlers.get(event)?.({ payload });
}

beforeEach(() => {
  setActivePinia(createPinia());
  mocks.handlers.clear();
  mocks.unlistened.length = 0;
  vi.clearAllMocks();
  mocks.runGwsStream.mockResolvedValue(RUN_ID);
  mocks.replayOutput.mockResolvedValue(undefined);
});

describe("cmd store 状态机", () => {
  it("exec 后 current 为 running、output 为空", async () => {
    const store = useCmdStore();
    const run = await store.exec("gws ls", ["ls"], "/hub");
    expect(mocks.runGwsStream).toHaveBeenCalledWith(["ls"], "/hub", undefined);
    // 订阅完成后必须通知后端回放缓存事件（先订阅后回放的契约）
    expect(mocks.replayOutput).toHaveBeenCalledTimes(1);
    expect(mocks.replayOutput).toHaveBeenCalledWith(RUN_ID);
    expect(run.state).toBe("running");
    expect(run.output).toBe("");
    expect(run.code).toBeNull();
    expect(store.current?.id).toBe(RUN_ID);
    expect(store.current?.state).toBe("running");
    expect(store.history).toHaveLength(0);
    expect(store.isRunning()).toBe(true);
  });

  it("output 事件累计 chunk", async () => {
    const store = useCmdStore();
    const run = await store.exec("gws ls", ["ls"], "/hub");
    emit(`gws-output:${RUN_ID}`, { chunk: "hello" });
    emit(`gws-output:${RUN_ID}`, { chunk: " world" });
    expect(run.output).toBe("hello world");
    expect(store.current?.output).toBe("hello world");
  });

  it("exit code 0 → done 且入 history", async () => {
    const store = useCmdStore();
    const run = await store.exec("gws ls", ["ls"], "/hub");
    emit(`gws-exit:${RUN_ID}`, { code: 0 });
    expect(run.state).toBe("done");
    expect(run.code).toBe(0);
    expect(store.history).toHaveLength(1);
    expect(store.history[0]?.id).toBe(RUN_ID);
    expect(store.isRunning()).toBe(false);
  });

  it("exit code 非 0 → failed", async () => {
    const store = useCmdStore();
    const run = await store.exec("gws run", ["run"], "/hub");
    emit(`gws-exit:${RUN_ID}`, { code: 2 });
    expect(run.state).toBe("failed");
    expect(run.code).toBe(2);
    expect(store.history).toHaveLength(1);
  });

  it("confirm 事件 → confirm 态 + confirmPending；answerConfirm(true) 恢复 running", async () => {
    const store = useCmdStore();
    const run = await store.exec("gws run", ["run"], "/hub");
    emit(`gws-confirm:${RUN_ID}`, { question: "确认发布？" });
    expect(run.state).toBe("confirm");
    expect(store.confirmPending).toEqual({ runId: RUN_ID, question: "确认发布？" });
    expect(store.isRunning()).toBe(true);

    await store.answerConfirm(true);
    expect(run.state).toBe("running");
    expect(store.confirmPending).toBeNull();
    expect(mocks.respondConfirm).toHaveBeenCalledTimes(1);
    expect(mocks.respondConfirm).toHaveBeenCalledWith(RUN_ID, true);
  });

  it("answerConfirm(false) → 当前 run 判 failed 并转发拒绝", async () => {
    const store = useCmdStore();
    const run = await store.exec("gws run", ["run"], "/hub");
    emit(`gws-confirm:${RUN_ID}`, { question: "确认发布？" });
    await store.answerConfirm(false);
    expect(run.state).toBe("failed");
    expect(store.confirmPending).toBeNull();
    expect(mocks.respondConfirm).toHaveBeenCalledTimes(1);
    expect(mocks.respondConfirm).toHaveBeenCalledWith(RUN_ID, false);
  });

  it("respondConfirm reject（run 已被后端清理）时 confirmPending 仍被清空", async () => {
    const store = useCmdStore();
    const run = await store.exec("gws run", ["run"], "/hub");
    emit(`gws-confirm:${RUN_ID}`, { question: "确认发布？" });
    mocks.respondConfirm.mockRejectedValueOnce(new Error("run 不存在"));
    await expect(store.answerConfirm(false)).rejects.toThrow("run 不存在");
    // finally 收尾：弹窗关闭（confirmPending 清空）+ 状态推进，不留僵尸确认
    expect(store.confirmPending).toBeNull();
    expect(run.state).toBe("failed");
    expect(mocks.respondConfirm).toHaveBeenCalledWith(RUN_ID, false);
  });

  it("僵尸 confirm：confirm 后进程自行退出 → confirmPending 被清空", async () => {
    const store = useCmdStore();
    const run = await store.exec("gws run", ["run"], "/hub");
    emit(`gws-confirm:${RUN_ID}`, { question: "确认发布？" });
    expect(store.confirmPending).not.toBeNull();
    emit(`gws-exit:${RUN_ID}`, { code: 1 });
    expect(store.confirmPending).toBeNull();
    expect(run.state).toBe("failed");
    expect(store.history).toHaveLength(1);
  });

  it("exit code null（后端杀进程）→ failed", async () => {
    const store = useCmdStore();
    const run = await store.exec("gws run", ["run"], "/hub");
    emit(`gws-exit:${RUN_ID}`, { code: null });
    expect(run.state).toBe("failed");
    expect(run.code).toBeNull();
    expect(store.history).toHaveLength(1);
  });

  it("陈旧 confirm 不误伤新 run：拒绝旧 run 的确认不改新 run 状态", async () => {
    const store = useCmdStore();
    mocks.runGwsStream.mockResolvedValueOnce(1);
    await store.exec("gws run", ["run"], "/hub");
    emit("gws-confirm:1", { question: "旧 run 的问题" });
    mocks.runGwsStream.mockResolvedValueOnce(2);
    const run2 = await store.exec("gws run", ["run"], "/hub");
    expect(store.current?.id).toBe(2);

    await store.answerConfirm(false);
    expect(mocks.respondConfirm).toHaveBeenCalledWith(1, false);
    expect(run2.state).toBe("running");
    expect(store.confirmPending).toBeNull();
  });

  it("waitDone：exec 后立即调用，随后 exit 事件 → resolve 且 run 为终态", async () => {
    const store = useCmdStore();
    const run = await store.exec("gws run", ["run"], "/hub");
    const pending = store.waitDone(run);
    emit(`gws-exit:${RUN_ID}`, { code: 0 });
    const settled = await pending;
    expect(settled).toBe(run);
    expect(settled.state).toBe("done");
  });

  it("waitDone：调用时已终态（exit 先行）→ 立即 resolve", async () => {
    const store = useCmdStore();
    const run = await store.exec("gws run", ["run"], "/hub");
    emit(`gws-exit:${RUN_ID}`, { code: 1 });
    // 无后续事件，仅靠 immediate 分支即须决议
    const settled = await store.waitDone(run);
    expect(settled).toBe(run);
    expect(settled.state).toBe("failed");
  });

  it("exec opts 透传：confirmTimeoutMs 传给 runGwsStream 第三参", async () => {
    const store = useCmdStore();
    await store.exec("gws drop mod", ["drop", "mod"], "/hub", { confirmTimeoutMs: 1500 });
    expect(mocks.runGwsStream).toHaveBeenCalledWith(["drop", "mod"], "/hub", 1500);
  });
});

describe("cmd store 弹窗执行（execDialog/closeDialog/holdDialog）", () => {
  it("execDialog 默认 confirmTimeoutMs=30000，dialogRun 指向该 run", async () => {
    const store = useCmdStore();
    const run = await store.execDialog("gws sync", ["sync"], "/hub");
    expect(mocks.runGwsStream).toHaveBeenCalledWith(["sync"], "/hub", 30000);
    expect(store.dialogRun).toBe(run);
    expect(store.dialogRun?.label).toBe("gws sync");
    expect(store.current).toBe(run); // exec 复用：current 同步指向（ConfirmDialog/守卫等仍可用）
  });

  it("execDialog 显式覆盖 confirmTimeoutMs（如 gws drop 传 1500）", async () => {
    const store = useCmdStore();
    await store.execDialog("gws drop mod", ["drop", "mod"], "/hub", { confirmTimeoutMs: 1500 });
    expect(mocks.runGwsStream).toHaveBeenCalledWith(["drop", "mod"], "/hub", 1500);
  });

  it("closeDialog 清空 dialogRun；再次 execDialog 重新设置", async () => {
    const store = useCmdStore();
    const run1 = await store.execDialog("gws sync", ["sync"], "/hub");
    expect(store.dialogRun).toBe(run1);
    store.closeDialog();
    expect(store.dialogRun).toBeNull();

    mocks.runGwsStream.mockResolvedValueOnce(2);
    const run2 = await store.execDialog("gws pull", ["pull"], "/hub");
    expect(store.dialogRun).toBe(run2);
    expect(store.dialogRun).not.toBe(run1); // 弹窗内容切换到新 run
  });

  it("execDialog 不影响 exec：普通 exec 不设置 dialogRun", async () => {
    const store = useCmdStore();
    await store.exec("gws ls", ["ls"], "/hub");
    expect(store.dialogRun).toBeNull();
  });

  it("holdDialog 计数：多次 hold/release 成对递减，release 到 0 不下穿", async () => {
    const store = useCmdStore();
    expect(store.holdCount).toBe(0);
    store.holdDialog();
    store.holdDialog();
    expect(store.holdCount).toBe(2);
    store.releaseDialog();
    expect(store.holdCount).toBe(1);
    store.releaseDialog();
    expect(store.holdCount).toBe(0);
    // 防御：多余 release 不得把计数打成 -1
    store.releaseDialog();
    expect(store.holdCount).toBe(0);
  });
});

describe("cmd store execDialog 失败展示（invoke reject 合成 failed run）", () => {
  it("execDialog reject → dialogRun 为合成 failed run（label/错误文案）、原样 rethrow", async () => {
    mocks.runGwsStream.mockRejectedValueOnce(new Error("gws 未安装"));
    const store = useCmdStore();
    await expect(store.execDialog("gws new demo", ["new", "demo"], "/hub"))
      .rejects.toThrow("gws 未安装");
    const run = store.dialogRun!;
    expect(run.id).toBe(-1); // 与真实 runId 区分的合成标记
    expect(run.label).toBe("gws new demo");
    expect(run.output).toContain("gws 未安装");
    expect(run.state).toBe("failed");
    expect(run.code).toBeNull();
    // 合成 run 不进 current：不算运行中，关闭按钮立即可用
    expect(store.isRunning()).toBe(false);
    expect(store.current).toBeNull();
    store.closeDialog();
    expect(store.dialogRun).toBeNull();
  });

  it("合成 failed run 不入 history、不订阅事件（无真实进程）", async () => {
    mocks.runGwsStream.mockRejectedValueOnce(new Error("IPC 失败"));
    const store = useCmdStore();
    await expect(store.execDialog("gws sync", ["sync"], "/hub")).rejects.toThrow("IPC 失败");
    expect(store.history).toHaveLength(0);
    expect(mocks.handlers.size).toBe(0);
    expect(mocks.replayOutput).not.toHaveBeenCalled();
  });
});

describe("cmd store 监听器生命周期（run 终态后拆除，防长会话泄漏）", () => {
  it("exit 事件后三个订阅被拆除（setTimeout 0 让一拍）：两次 exec 监听表不残留", async () => {
    const store = useCmdStore();
    mocks.runGwsStream.mockResolvedValueOnce(1);
    await store.exec("a", ["a"], "/hub");
    emit("gws-exit:1", { code: 0 });
    mocks.runGwsStream.mockResolvedValueOnce(2);
    await store.exec("b", ["b"], "/hub");
    emit("gws-exit:2", { code: 1 });

    // 两次 exec × 3 个订阅全部拆除：监听表清空（而非随 exec 次数累积）
    // 注：前序测试遗留的清理定时器也会在此期间触发并向 unlistened 追加记录
    //（身份检查使其对 handlers 无害），故断言以 handlers 清空 + 6 个事件名齐全为准
    await vi.waitFor(() => expect(mocks.handlers.size).toBe(0));
    const names = new Set(mocks.unlistened);
    for (const n of [
      "gws-output:1", "gws-exit:1", "gws-confirm:1",
      "gws-output:2", "gws-exit:2", "gws-confirm:2",
    ]) {
      expect(names.has(n)).toBe(true);
    }
  });

  it("运行中（无 exit）不拆订阅：后续 output/confirm 事件仍可达", async () => {
    const store = useCmdStore();
    const run = await store.exec("a", ["a"], "/hub");
    expect(mocks.handlers.size).toBe(3); // 三个订阅全部在册
    emit(`gws-output:${RUN_ID}`, { chunk: "hi" });
    emit(`gws-confirm:${RUN_ID}`, { question: "继续？" });
    expect(run.output).toBe("hi");
    expect(run.state).toBe("confirm");
    expect(store.confirmPending).toEqual({ runId: RUN_ID, question: "继续？" });
  });

  it("exit 后拆订阅：同 run 迟到的 output 事件不再影响 run（防御性）", async () => {
    const store = useCmdStore();
    const run = await store.exec("a", ["a"], "/hub");
    emit(`gws-exit:${RUN_ID}`, { code: 0 });
    await vi.waitFor(() => expect(mocks.handlers.size).toBe(0)); // 本 run 订阅已拆
    emit(`gws-output:${RUN_ID}`, { chunk: "late" }); // 订阅已拆，handler 不在
    expect(run.output).toBe("");
    expect(store.history).toHaveLength(1);
  });

  it("listen 注册中途失败：拆除已注册订阅再抛（不留泄漏）", async () => {
    const { listen } = await import("@tauri-apps/api/event");
    // 第 2 个 listen（exit）注册失败：output 已注册、confirm 未到
    //（vi.mocked 拿真实 listen 类型，handler 需桥接为本文件的 Handler 形状）
    vi.mocked(listen)
      .mockImplementationOnce(async (event, handler) => {
        const name = String(event);
        const h = handler as unknown as Handler;
        mocks.handlers.set(name, h);
        return () => {
          if (mocks.handlers.get(name) === h) mocks.handlers.delete(name);
          mocks.unlistened.push(name);
        };
      })
      .mockImplementationOnce(async () => {
        throw new Error("订阅失败");
      });
    const store = useCmdStore();
    await expect(store.exec("a", ["a"], "/hub")).rejects.toThrow("订阅失败");
    // 已注册的 output 订阅被拆除，监听表不残留
    expect(mocks.handlers.size).toBe(0);
    expect(mocks.unlistened).toContain(`gws-output:${RUN_ID}`);
  });
});
