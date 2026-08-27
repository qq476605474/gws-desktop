// @vitest-environment happy-dom
// 无 @vue/test-utils：手动 createApp().mount() 挂载 SFC；
// gws-bridge / tauri 事件 mock 模式参考 src/components/AddModuleDialog.test.ts
import { afterEach, describe, expect, it, vi } from "vitest";
import { createApp, nextTick, type App } from "vue";
import { createPinia, setActivePinia } from "pinia";

type Payload = Record<string, unknown>;
type Handler = (e: { payload: Payload }) => void;

// vi.mock 工厂随 import 提升、早于本文件函数体执行，共享状态须经 vi.hoisted 创建以避免 TDZ
const mocks = vi.hoisted(() => ({
  runGwsStream: vi.fn<(args: string[], cwd: string, confirmTimeoutMs?: number) => Promise<number>>(),
  respondConfirm: vi.fn<(runId: number, yes: boolean) => Promise<void>>(),
  replayOutput: vi.fn<(runId: number) => Promise<void>>(),
  /** 按事件名保存 listen 注册的 handler，测试中手动触发以模拟事件流 */
  handlers: new Map<string, Handler>(),
}));

vi.mock("../lib/gws-bridge", () => ({
  runGwsStream: mocks.runGwsStream,
  respondConfirm: mocks.respondConfirm,
  replayOutput: mocks.replayOutput,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: Handler) => {
    mocks.handlers.set(event, handler);
    return () => mocks.handlers.delete(event);
  }),
}));

import { useCmdStore } from "../stores/cmd";
import CmdDialog from "./CmdDialog.vue";

let app: App | null = null;
let el: HTMLElement | null = null;
let store: ReturnType<typeof useCmdStore> | null = null;

function mountDialog() {
  const pinia = createPinia();
  setActivePinia(pinia);
  store = useCmdStore();
  el = document.createElement("div");
  document.body.appendChild(el);
  app = createApp(CmdDialog);
  app.use(pinia);
  app.mount(el);
}

/** 模拟后端发出一个 Tauri 事件 */
function emit(event: string, payload: Payload) {
  mocks.handlers.get(event)?.({ payload });
}

function closeButton(): HTMLButtonElement {
  const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
    .find((b) => b.textContent?.trim() === "关闭");
  if (!btn) throw new Error("关闭按钮未找到");
  return btn;
}

/** execDialog 并等三个事件订阅就绪（返回 run 供断言） */
async function startRun(label = "gws sync") {
  mocks.runGwsStream.mockResolvedValue(1);
  mocks.replayOutput.mockResolvedValue(undefined);
  const pending = store!.execDialog(label, ["sync"], "/hub");
  await vi.waitFor(() => expect(mocks.handlers.get("gws-exit:1")).toBeTruthy());
  return pending;
}

afterEach(() => {
  app?.unmount();
  app = null;
  el?.remove();
  el = null;
  store = null;
  mocks.handlers.clear();
  vi.clearAllMocks();
});

describe("CmdDialog 渲染", () => {
  it("无 dialogRun 时不渲染任何弹窗", () => {
    mountDialog();
    expect(el!.querySelector(".mask")).toBeNull();
    expect(el!.textContent).toBe("");
  });

  it("execDialog 后渲染 label、流式输出（ANSI 转 HTML）与运行中状态；关闭禁用；点 mask 不关闭", async () => {
    mountDialog();
    const run = await startRun("gws sync");
    await nextTick();

    expect(el!.querySelector(".mask")).toBeTruthy();
    expect(el!.textContent).toContain("gws sync");
    expect(el!.textContent).toContain("执行中…"); // running 状态文案 + spinner
    expect(closeButton().disabled).toBe(true); // 运行中禁止关闭

    // 流式输出：output 事件累计并经 ansiToHtml 渲染为带色 span
    // （首 chunk 的 ANSI 开色无 reset，后续 chunk 落在同一个 span 内直到 reset）
    emit("gws-output:1", { chunk: "\u001b[32m✓ ok" });
    emit("gws-output:1", { chunk: " done\n" });
    await nextTick();
    const pre = el!.querySelector("pre")!;
    expect(pre.innerHTML).toContain('<span class="c32">✓ ok done\n</span>');

    // mask 不可点击关闭（强制手动点关闭按钮）
    el!.querySelector<HTMLElement>(".mask")!.click();
    await nextTick();
    expect(el!.querySelector(".mask")).toBeTruthy();
    expect(store!.dialogRun).toBe(run);
  });

  it("confirm 态（等待确认）：仍显示执行中、关闭禁用（ConfirmDialog 在其上层处理）", async () => {
    mountDialog();
    await startRun();
    await nextTick();

    emit("gws-confirm:1", { question: "gws 在 /hub 等待确认（30s 无输出）。确认继续？" });
    await nextTick();
    expect(store!.dialogRun?.state).toBe("confirm");
    expect(el!.textContent).toContain("执行中…");
    expect(closeButton().disabled).toBe(true); // isRunning 含 confirm 态
  });
});

describe("CmdDialog 终态与关闭", () => {
  it("exit 0 → ✓ 完成、关闭可用；点关闭 → closeDialog 清空、弹窗卸载", async () => {
    mountDialog();
    await startRun();
    await nextTick();

    emit("gws-exit:1", { code: 0 });
    await vi.waitFor(() => expect(el!.textContent).toContain("✓ 完成"));
    expect(closeButton().disabled).toBe(false);

    closeButton().click();
    await nextTick();
    expect(store!.dialogRun).toBeNull();
    expect(el!.querySelector(".mask")).toBeNull(); // v-if 卸载
  });

  it("exit 非 0 → ✗ 失败、关闭可用", async () => {
    mountDialog();
    await startRun();
    await nextTick();

    emit("gws-exit:1", { code: 1 });
    await vi.waitFor(() => expect(el!.textContent).toContain("✗ 失败"));
    expect(closeButton().disabled).toBe(false);
  });

  it("holdDialog 期间（即使已终态）关闭禁用；releaseDialog 后恢复", async () => {
    mountDialog();
    await startRun();
    await nextTick();

    emit("gws-exit:1", { code: 0 });
    await vi.waitFor(() => expect(closeButton().disabled).toBe(false));

    store!.holdDialog(); // 多命令序列在途（如 AddModuleDialog 逐模块 add）
    await nextTick();
    expect(closeButton().disabled).toBe(true);

    store!.releaseDialog();
    await nextTick();
    expect(closeButton().disabled).toBe(false);
  });
});
