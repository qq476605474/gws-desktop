// @vitest-environment happy-dom
// 无 @vue/test-utils：手动 createApp().mount() 挂载 SFC；
// gws-bridge / tauri 事件 mock 模式参考 src/components/AddModuleDialog.test.ts
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createApp, nextTick, type App } from "vue";
import { createPinia, setActivePinia } from "pinia";

type Payload = Record<string, unknown>;
type Handler = (e: { payload: Payload }) => void;

// vi.mock 工厂随 import 提升、早于本文件函数体执行，共享状态须经 vi.hoisted 创建以避免 TDZ
const mocks = vi.hoisted(() => ({
  runGws: vi.fn(),
  runGwsStream: vi.fn<(args: string[], cwd: string, confirmTimeoutMs?: number) => Promise<number>>(),
  respondConfirm: vi.fn<(runId: number, yes: boolean) => Promise<void>>(),
  replayOutput: vi.fn<(runId: number) => Promise<void>>(),
  /** 按事件名保存 listen 注册的 handler，测试中手动触发以模拟事件流 */
  handlers: new Map<string, Handler>(),
}));

vi.mock("../lib/gws-bridge", () => ({
  runGws: mocks.runGws,
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
    };
  }),
}));

import { useHubStore } from "../stores/hub";
import NewWorkspaceDialog from "./NewWorkspaceDialog.vue";

let app: App | null = null;
let el: HTMLElement | null = null;
let nextId = 0;
/** emit 顺序探针：created 必须先于 close（父组件先刷新列表再卸载弹窗） */
const events: string[] = [];

function mountDialog() {
  const pinia = createPinia();
  setActivePinia(pinia);
  const hub = useHubStore();
  hub.setHub("/hub");
  el = document.createElement("div");
  document.body.appendChild(el);
  events.length = 0;
  app = createApp(NewWorkspaceDialog, {
    onClose: () => events.push("close"),
    onCreated: () => events.push("created"),
  });
  app.use(pinia);
  app.mount(el);
}

/** 第 idx 个文本输入框（0=名称） */
function setInput(idx: number, value: string) {
  const input = el!.querySelectorAll<HTMLInputElement>("input")[idx]!;
  input.value = value;
  input.dispatchEvent(new Event("input"));
}

/** 点击创建：先等 DOM 补丁——v-model 同步更新 name，但按钮 disabled 属性
 *  要到 nextTick 才落到 DOM，同步点击会被（仍禁用的）按钮吞掉 */
async function clickCreate() {
  await nextTick();
  const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
    .find((b) => b.textContent?.trim() === "创建");
  if (!btn) throw new Error("创建按钮未找到");
  btn.click();
}

/** 等待第 runId 次 exec 完成 listen 订阅后发出退出码 */
async function exitWith(runId: number, code: number) {
  await vi.waitFor(() => expect(mocks.handlers.get(`gws-exit:${runId}`)).toBeTruthy());
  mocks.handlers.get(`gws-exit:${runId}`)!({ payload: { code } });
}

beforeEach(() => {
  nextId = 0;
  mocks.runGwsStream.mockImplementation(async () => ++nextId);
  mocks.replayOutput.mockResolvedValue(undefined);
});

afterEach(() => {
  app?.unmount();
  app = null;
  el?.remove();
  el = null;
  mocks.handlers.clear();
  vi.clearAllMocks();
  events.length = 0;
});

describe("NewWorkspaceDialog", () => {
  it("名称 placeholder 写明 gws new 的实际规则：必填、无留空反推", () => {
    mountDialog();
    const placeholder = el!.querySelectorAll<HTMLInputElement>("input")[0]!.placeholder;
    expect(placeholder).toContain("必填");
    expect(placeholder).toContain("前缀-日期-名称");
  });

  it("创建按钮在名称为空时禁用（gws new 名称是必填位置参数）", () => {
    mountDialog();
    const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
      .find((b) => b.textContent?.trim() === "创建")!;
    expect(btn.disabled).toBe(true);
    setInput(0, "demo");
    return vi.waitFor(() => expect(btn.disabled).toBe(false));
  });

  it("命令成功（exit 0）：created 先于 close 通知（父组件先刷新列表再卸载弹窗）", async () => {
    mountDialog();
    setInput(0, "demo");
    await clickCreate();
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(["new", "demo"], "/hub", 30000),
    );
    await exitWith(1, 0);
    await vi.waitFor(() => expect(events).toEqual(["created", "close"]));
  });

  it("命令失败（exit 1）：不 emit created/close，弹窗保留输入便于重试", async () => {
    mountDialog();
    setInput(0, "demo");
    await clickCreate();
    await exitWith(1, 1);
    // 等若干拍确认无任何 emit（waitDone 已决议、组件仍挂载）
    await new Promise((r) => setTimeout(r, 10));
    expect(events).toEqual([]);
    expect(el!.querySelector(".dialog")).toBeTruthy(); // 弹窗未卸载
    // 失败后可重试：再次点击创建仍会发起新命令
    await clickCreate();
    await vi.waitFor(() => expect(mocks.runGwsStream).toHaveBeenCalledTimes(2));
  });

  it("execDialog reject（IPC 失败）：err 内联提示、不关窗不 emit created", async () => {
    mountDialog();
    mocks.runGwsStream.mockRejectedValueOnce(new Error("gws 未安装"));
    setInput(0, "demo");
    await clickCreate();
    await vi.waitFor(() => expect(el!.textContent).toContain("gws 未安装"));
    expect(events).toEqual([]);
    expect(el!.querySelector(".dialog")).toBeTruthy();
  });

  it("参数拼装：模块/标题/前缀/自定义分支按需附加", async () => {
    mountDialog();
    setInput(0, "demo");
    setInput(1, "结算改版"); // 标题
    setInput(2, "custom-branch"); // 完全自定义分支名（文本输入序：名称/标题/自定义分支）
    // 前缀切 hotfix
    const select = el!.querySelector<HTMLSelectElement>("select")!;
    select.value = "hotfix";
    select.dispatchEvent(new Event("change"));
    await clickCreate();
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(
        ["new", "demo", "--title", "结算改版", "--prefix", "hotfix", "--branch", "custom-branch"],
        "/hub",
        30000,
      ),
    );
  });
});
