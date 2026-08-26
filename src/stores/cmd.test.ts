import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

type Payload = Record<string, unknown>;
type Handler = (e: { payload: Payload }) => void;

// vi.mock 工厂随 import 提升、早于本文件函数体执行，
// 共享状态必须经 vi.hoisted 创建以避免 TDZ。
const mocks = vi.hoisted(() => ({
  runGwsStream: vi.fn<(args: string[], cwd: string) => Promise<number>>().mockResolvedValue(1),
  respondConfirm: vi.fn<(runId: number, yes: boolean) => Promise<void>>().mockResolvedValue(undefined),
  /** 按事件名保存 listen 注册的 handler，测试中手动触发以模拟事件流 */
  handlers: new Map<string, Handler>(),
}));

vi.mock("../lib/gws-bridge", () => ({
  runGwsStream: mocks.runGwsStream,
  respondConfirm: mocks.respondConfirm,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: Handler) => {
    mocks.handlers.set(event, handler);
    return () => mocks.handlers.delete(event);
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
  vi.clearAllMocks();
  mocks.runGwsStream.mockResolvedValue(RUN_ID);
});

describe("cmd store 状态机", () => {
  it("exec 后 current 为 running、output 为空", async () => {
    const store = useCmdStore();
    const run = await store.exec("gws ls", ["ls"], "/hub");
    expect(mocks.runGwsStream).toHaveBeenCalledWith(["ls"], "/hub");
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
});
