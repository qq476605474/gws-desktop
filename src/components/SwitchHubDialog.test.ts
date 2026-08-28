// @vitest-environment happy-dom
// 无 @vue/test-utils：手动 createApp().mount() 挂载 SFC；
// mock 模式参考 src/views/StartupView.test.ts
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createApp, nextTick, type App } from "vue";
import { createPinia, setActivePinia } from "pinia";

type Handler = (e: { payload: Record<string, unknown> }) => void;

// vi.mock 工厂随 import 提升、早于本文件函数体执行，共享状态须经 vi.hoisted 创建以避免 TDZ
const mocks = vi.hoisted(() => ({
  hubExists: vi.fn<(path: string) => Promise<boolean>>(),
  runGwsStream: vi.fn<(args: string[], cwd: string, confirmTimeoutMs?: number) => Promise<number>>(),
  respondConfirm: vi.fn<(runId: number, yes: boolean) => Promise<void>>(),
  replayOutput: vi.fn<(runId: number) => Promise<void>>(),
  openDialog: vi.fn<() => Promise<string | string[] | null>>(),
  /** 按事件名保存 listen 注册的 handler，测试中手动触发以模拟 gws-exit 事件 */
  handlers: new Map<string, Handler>(),
}));

vi.mock("../lib/gws-bridge", () => ({
  hubExists: mocks.hubExists,
  runGwsStream: mocks.runGwsStream,
  respondConfirm: mocks.respondConfirm,
  replayOutput: mocks.replayOutput,
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: mocks.openDialog,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: Handler) => {
    mocks.handlers.set(event, handler);
    return () => {
      if (mocks.handlers.get(event) === handler) mocks.handlers.delete(event);
    };
  }),
}));

vi.mock("@tauri-apps/plugin-store", () => ({
  load: vi.fn(),
  Store: class Store {},
}));

import { useSettingsStore } from "../stores/settings";
import { useHubStore } from "../stores/hub";
import SwitchHubDialog from "./SwitchHubDialog.vue";

let app: App | null = null;
let el: HTMLElement | null = null;
let nextId = 0;
let closed = 0;

/** 挂载切换弹窗；hub.path 预置为当前 hub，close 事件计数到 closed */
function mountDialog(currentHub = "/hub/old") {
  const pinia = createPinia();
  setActivePinia(pinia);
  useHubStore().setHub(currentHub);
  el = document.createElement("div");
  document.body.appendChild(el);
  app = createApp(SwitchHubDialog, { onClose: () => closed++ });
  app.use(pinia);
  app.mount(el);
}

function clickButton(text: string, scope?: ParentNode) {
  const btn = Array.from((scope ?? el!).querySelectorAll<HTMLButtonElement>("button"))
    .find((b) => b.textContent?.trim() === text);
  if (!btn) throw new Error(`按钮「${text}」未找到`);
  btn.click();
}

function setInput(placeholderFragment: string, value: string) {
  const input = Array.from(el!.querySelectorAll<HTMLInputElement>("input"))
    .find((i) => i.placeholder.includes(placeholderFragment));
  if (!input) throw new Error(`输入框（placeholder 含「${placeholderFragment}」）未找到`);
  input.value = value;
  input.dispatchEvent(new Event("input"));
}

/** 等待第 runId 次 exec 完成 listen 订阅后发出退出码 */
async function exitWith(runId: number, code: number) {
  await vi.waitFor(() => expect(mocks.handlers.get(`gws-exit:${runId}`)).toBeTruthy());
  mocks.handlers.get(`gws-exit:${runId}`)!({ payload: { code } });
}

/** 取最内层弹窗（新建 hub 叠层在切换弹窗之上，querySelectorAll 取最后一个） */
function initDialog(): HTMLElement {
  const dialogs = el!.querySelectorAll<HTMLElement>(".dialog");
  return dialogs[dialogs.length - 1]!;
}

beforeEach(() => {
  nextId = 0;
  closed = 0;
  mocks.hubExists.mockResolvedValue(true);
  mocks.openDialog.mockResolvedValue(null);
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
});

describe("SwitchHubDialog 打开既有 hub", () => {
  it("回显当前 hub；切到新 hub：hubExists 校验后 setHub+lastHub 并 emit close", async () => {
    mountDialog("/hub/old");
    await nextTick();
    expect(Array.from(el!.querySelectorAll<HTMLInputElement>("input"))
      .find((i) => i.placeholder.includes("hub 目录路径"))!.value).toBe("/hub/old");

    setInput("hub 目录路径", "/hub/new");
    await nextTick();
    clickButton("打开 Hub");
    await vi.waitFor(() => expect(mocks.hubExists).toHaveBeenCalledWith("/hub/new"));
    await vi.waitFor(() => expect(closed).toBe(1));
    expect(useHubStore().path).toBe("/hub/new");
    expect(useSettingsStore().lastHub).toBe("/hub/new");
  });

  it("路径不是 hub：错误提示，不 emit close", async () => {
    mountDialog("/hub/old");
    await nextTick();
    mocks.hubExists.mockResolvedValue(false);

    setInput("hub 目录路径", "/not-a-hub");
    await nextTick();
    clickButton("打开 Hub");
    await vi.waitFor(() => expect(el!.textContent).toContain("该目录不是 gws hub"));
    expect(closed).toBe(0);
    expect(useHubStore().path).toBe("/hub/old");
  });
});

describe("SwitchHubDialog 新建 hub（gws init）", () => {
  it("入口开叠层弹窗：必填才可创建；gws init 于父目录执行目标 <父>/<名>，成功后 setHub+lastHub+整体关闭", async () => {
    mountDialog("/hub/old");
    await nextTick();
    expect(el!.textContent).toContain("没有 hub？");

    clickButton("新建一个 →");
    await nextTick();
    const dialog = initDialog();
    expect(dialog.querySelector("h3")!.textContent).toBe("新建 Hub");
    // 两个都空：创建禁用
    const create = Array.from(dialog.querySelectorAll<HTMLButtonElement>("button"))
      .find((b) => b.textContent?.trim() === "创建")!;
    expect(create.disabled).toBe(true);

    setInput("父目录", "/Users/demo/dev");
    setInput("如 myhub", "myhub");
    await nextTick();
    expect(create.disabled).toBe(false);
    create.click();
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(["init", "/Users/demo/dev/myhub"], "/Users/demo/dev", 30000),
    );

    await exitWith(1, 0);
    await vi.waitFor(() => expect(closed).toBe(1)); // 新 hub 生效：整体关闭触发 MainView 重挂载
    expect(useHubStore().path).toBe("/Users/demo/dev/myhub");
    expect(useSettingsStore().lastHub).toBe("/Users/demo/dev/myhub");
  });

  it("gws init 失败（exit 1）：留窗可改可取消，hub 未变不 emit close", async () => {
    mountDialog("/hub/old");
    await nextTick();
    clickButton("新建一个 →");
    await nextTick();

    setInput("父目录", "/Users/demo/dev");
    setInput("如 myhub", "myhub");
    await nextTick();
    clickButton("创建", initDialog());
    await exitWith(1, 1);
    await new Promise((r) => setTimeout(r, 10));
    expect(closed).toBe(0);
    expect(useHubStore().path).toBe("/hub/old");
    expect(initDialog().querySelector("h3")!.textContent).toBe("新建 Hub"); // 叠层弹窗保留
    // 取消只关叠层，切换弹窗仍在
    clickButton("取消", initDialog());
    await nextTick();
    expect(el!.querySelectorAll(".dialog").length).toBe(1);
    expect(el!.textContent).toContain("切换 Hub 目录");
  });

  it("浏览选择父目录：open 返回路径填入父目录输入框", async () => {
    mountDialog("/hub/old");
    await nextTick();
    clickButton("新建一个 →");
    await nextTick();
    mocks.openDialog.mockResolvedValue("/Users/demo/dev");

    clickButton("浏览…", initDialog());
    await vi.waitFor(() =>
      expect(Array.from(el!.querySelectorAll<HTMLInputElement>("input"))
        .find((i) => i.placeholder.includes("父目录"))!.value).toBe("/Users/demo/dev"),
    );
  });
});
