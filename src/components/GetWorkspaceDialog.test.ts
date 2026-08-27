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
import GetWorkspaceDialog from "./GetWorkspaceDialog.vue";

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
  app = createApp(GetWorkspaceDialog, {
    onClose: () => events.push("close"),
    onCreated: () => events.push("created"),
  });
  app.use(pinia);
  app.mount(el);
}

/** 第 idx 个文本输入框（0=远程分支，1=本地工作区名，2=标题） */
function setInput(idx: number, value: string) {
  const input = el!.querySelectorAll<HTMLInputElement>("input")[idx]!;
  input.value = value;
  input.dispatchEvent(new Event("input"));
}

/** 拉取按钮（「取消」之外的 primary 按钮） */
function pullButton(): HTMLButtonElement {
  const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
    .find((b) => b.textContent?.trim() === "拉取");
  if (!btn) throw new Error("拉取按钮未找到");
  return btn;
}

/** 点击拉取：先等 DOM 补丁——v-model 同步更新 branch，但按钮 disabled 属性
 *  要到 nextTick 才落到 DOM，同步点击会被（仍禁用的）按钮吞掉 */
async function clickPull() {
  await nextTick();
  pullButton().click();
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

describe("GetWorkspaceDialog", () => {
  it("命令成功（exit 0）：created 先于 close 通知（父组件先刷新列表再卸载弹窗）", async () => {
    mountDialog();
    setInput(0, "feature-20260818-demo");
    await clickPull();
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(
        ["get", "feature-20260818-demo"],
        "/hub",
        30000,
      ),
    );
    await exitWith(1, 0);
    await vi.waitFor(() => expect(events).toEqual(["created", "close"]));
  });

  it("--name/--title 按需附加", async () => {
    mountDialog();
    setInput(0, "feature-20260818-demo");
    setInput(1, "local-demo");
    setInput(2, "结算改版");
    await clickPull();
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(
        ["get", "feature-20260818-demo", "--name", "local-demo", "--title", "结算改版"],
        "/hub",
        30000,
      ),
    );
  });

  it("branch/name 前后空白：trim 后传参（\"feature-1 \" → 首参 \"feature-1\"）", async () => {
    mountDialog();
    setInput(0, "feature-1 ");
    setInput(1, " demo ");
    await clickPull();
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(
        ["get", "feature-1", "--name", "demo"],
        "/hub",
        30000,
      ),
    );
  });

  it("纯空格 branch：拉取按钮 disabled，点击/派发事件均不 exec", async () => {
    mountDialog();
    setInput(0, "   ");
    await nextTick();
    const btn = pullButton();
    expect(btn.disabled).toBe(true); // trim 后为空 → 禁用
    // happy-dom 的 click() 会拦 disabled，再派发原生事件防程序化绕过——两层都不发命令
    btn.click();
    btn.dispatchEvent(new Event("click", { bubbles: true }));
    await new Promise((r) => setTimeout(r, 0));
    expect(mocks.runGwsStream).not.toHaveBeenCalled();
    expect(events).toEqual([]);
  });

  it("命令失败（exit 1）：不 emit created/close，弹窗保留输入便于重试", async () => {
    mountDialog();
    setInput(0, "feature-20260818-demo");
    await clickPull();
    await exitWith(1, 1);
    await new Promise((r) => setTimeout(r, 10));
    expect(events).toEqual([]);
    expect(el!.querySelector(".dialog")).toBeTruthy(); // 弹窗未卸载
  });

  it("execDialog reject（IPC 失败）：err 内联提示、不关窗不 emit created", async () => {
    mountDialog();
    mocks.runGwsStream.mockRejectedValueOnce(new Error("invoke 失败"));
    setInput(0, "feature-20260818-demo");
    await clickPull();
    await vi.waitFor(() => expect(el!.textContent).toContain("invoke 失败"));
    expect(events).toEqual([]);
    expect(el!.querySelector(".dialog")).toBeTruthy();
  });

  it("submitting 期间点 mask/取消不关窗（防 IPC 间隙卸载致 created 通知丢失）", async () => {
    mountDialog();
    setInput(0, "feature-20260818-demo");
    await clickPull();
    // 命令在途（无 exit 事件 → waitDone 挂起、submitting=true）：
    // 此窗口内一次 mask 点击/取消即可卸载组件，终态后 emit("created") 变 no-op
    expect(mocks.runGwsStream).toHaveBeenCalledTimes(1);
    (el!.querySelector(".mask") as HTMLElement).click(); // click.self：目标为 mask 自身
    const cancel = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
      .find((b) => b.textContent?.trim() === "取消")!;
    await vi.waitFor(() => expect(cancel.disabled).toBe(true));
    cancel.click();
    await new Promise((r) => setTimeout(r, 10));
    expect(events).toEqual([]);
    expect(el!.querySelector(".dialog")).toBeTruthy();
  });
});
