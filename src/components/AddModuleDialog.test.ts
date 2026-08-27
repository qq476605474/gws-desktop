// @vitest-environment happy-dom
// 无 @vue/test-utils：手动 createApp().mount() 挂载 SFC；
// gws-bridge / tauri 事件 mock 模式参考 src/stores/cmd.test.ts
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
  /** 按事件名保存 listen 注册的 handler，测试中手动触发以模拟 gws-exit 事件 */
  handlers: new Map<string, Handler>(),
}));

// hub store 亦从该模块导入 runGws，mock 须补齐具名导出，否则导入链接缺绑定
vi.mock("../lib/gws-bridge", () => ({
  runGws: vi.fn(),
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

import { useHubStore } from "../stores/hub";
import { useCmdStore } from "../stores/cmd";
import AddModuleDialog from "./AddModuleDialog.vue";

let app: App | null = null;
let el: HTMLElement | null = null;

/** 挂载弹窗（repo 列表直接写入 hub store），事件经 props 探针收集 */
function mountDialog(onAdded: () => void, onClose: () => void) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const hub = useHubStore();
  hub.setHub("/hub");
  hub.repos = [
    { name: "mod-a", mainBranch: "main" },
    { name: "mod-b", mainBranch: "main" },
  ];
  el = document.createElement("div");
  document.body.appendChild(el);
  app = createApp(AddModuleDialog, { wsPath: "/hub/ws/demo", onAdded, onClose });
  app.use(pinia);
  app.mount(el);
}

function checkboxes(): HTMLInputElement[] {
  return Array.from(el!.querySelectorAll<HTMLInputElement>('input[type="checkbox"]'));
}

/** 勾选全部模块（checkbox v-model 监听 change 事件） */
async function checkAll() {
  for (const b of checkboxes()) {
    b.checked = true;
    b.dispatchEvent(new Event("change"));
    // 两次 dispatch 之间须等 patch 刷新 el._modelValue：数组型 checkbox 的 change
    // 处理器基于旧数组 concat，同步连发会让后一次覆盖前一次的勾选
    await nextTick();
  }
}

function clickAdd() {
  const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
    .find((b) => b.textContent?.trim() === "添加");
  if (!btn) throw new Error("添加按钮未找到");
  btn.click();
}

/** 等待第 runId 次 exec 完成 listen 订阅后发出退出码（runId 从 1 递增） */
async function exitWith(runId: number, code: number) {
  await vi.waitFor(() => expect(mocks.handlers.get(`gws-exit:${runId}`)).toBeTruthy());
  mocks.handlers.get(`gws-exit:${runId}`)!({ payload: { code } });
}

afterEach(() => {
  app?.unmount();
  app = null;
  el?.remove();
  el = null;
  mocks.handlers.clear();
  vi.clearAllMocks();
});

describe("AddModuleDialog.add 部分失败处理", () => {
  it("第一个模块失败、第二个成功：不 emit added/close，err 列出失败项，selected 只剩失败项", async () => {
    const added = vi.fn();
    const closed = vi.fn();
    mountDialog(added, closed);
    let nextId = 0;
    mocks.runGwsStream.mockImplementation(async () => ++nextId);

    await checkAll();
    clickAdd();

    // mod-a 失败（exit 1）；模块间独立 → mod-b 仍继续执行且成功（exit 0）
    await exitWith(1, 1);
    await exitWith(2, 0);

    await vi.waitFor(() => expect(el!.textContent).toContain("部分模块添加失败：mod-a（详见命令弹窗）"));
    expect(added).not.toHaveBeenCalled();
    expect(closed).not.toHaveBeenCalled();
    // 失败不中断：两个模块都被尝试，且在弹窗工作区目录下执行；
    // execDialog 默认 confirmTimeoutMs=30000（慢命令防假确认）
    expect(mocks.runGwsStream).toHaveBeenCalledTimes(2);
    expect(mocks.runGwsStream).toHaveBeenNthCalledWith(1, ["add", "mod-a"], "/hub/ws/demo", 30000);
    expect(mocks.runGwsStream).toHaveBeenNthCalledWith(2, ["add", "mod-b"], "/hub/ws/demo", 30000);

    // 已成功项被剔除：mod-a（失败）保持勾选可直接重试，mod-b（成功）取消勾选
    await nextTick();
    const after = checkboxes();
    expect(after[0]!.checked).toBe(true);
    expect(after[1]!.checked).toBe(false);
  });

  it("全部成功：行为不变——emit added 与 close，无错误提示；序列期间 hold 弹窗、结束释放", async () => {
    const added = vi.fn();
    const closed = vi.fn();
    mountDialog(added, closed);
    let nextId = 0;
    mocks.runGwsStream.mockImplementation(async () => ++nextId);
    const cmd = useCmdStore();

    await checkAll();
    clickAdd();

    await exitWith(1, 0);
    // 序列中途（第 2 个 add 尚未开始）：hold 计数>0，弹窗保持、关闭按钮禁用
    expect(cmd.holdCount).toBe(1);
    await exitWith(2, 0);

    await vi.waitFor(() => expect(added).toHaveBeenCalledTimes(1));
    expect(closed).toHaveBeenCalledTimes(1);
    expect(el!.textContent).not.toContain("部分模块添加失败");
    // 序列结束（finally release）：计数归零，弹窗可手动关闭
    expect(cmd.holdCount).toBe(0);
  });
});
