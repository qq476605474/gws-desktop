// @vitest-environment happy-dom
// 无 @vue/test-utils：手动 createApp().mount() 挂载 SFC；
// mock 模式参考 src/components/TopBar.test.ts（gws-bridge / plugin-dialog / plugin-store）
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createApp, nextTick, type App } from "vue";
import { createPinia, setActivePinia } from "pinia";

type Handler = (e: { payload: Record<string, unknown> }) => void;

// vi.mock 工厂随 import 提升、早于本文件函数体执行，共享状态须经 vi.hoisted 创建以避免 TDZ
const mocks = vi.hoisted(() => ({
  checkGwsInstalled: vi.fn<() => Promise<boolean>>(),
  hubExists: vi.fn<(path: string) => Promise<boolean>>(),
  runGwsStream: vi.fn<(args: string[], cwd: string, confirmTimeoutMs?: number) => Promise<number>>(),
  respondConfirm: vi.fn<(runId: number, yes: boolean) => Promise<void>>(),
  replayOutput: vi.fn<(runId: number) => Promise<void>>(),
  openDialog: vi.fn<() => Promise<string | string[] | null>>(),
  /** 按事件名保存 listen 注册的 handler，测试中手动触发以模拟 gws-exit 事件 */
  handlers: new Map<string, Handler>(),
}));

vi.mock("../lib/gws-bridge", () => ({
  checkGwsInstalled: mocks.checkGwsInstalled,
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
import { currentView } from "../router";
import StartupView from "./StartupView.vue";

let app: App | null = null;
let el: HTMLElement | null = null;
let nextId = 0;

/** 挂载启动页；无 lastHub（默认）不自动进主界面 */
function mountView() {
  const pinia = createPinia();
  setActivePinia(pinia);
  el = document.createElement("div");
  document.body.appendChild(el);
  app = createApp(StartupView);
  app.use(pinia);
  app.mount(el);
}

function clickButton(text: string) {
  const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
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

beforeEach(() => {
  nextId = 0;
  currentView.value = "startup";
  mocks.checkGwsInstalled.mockResolvedValue(true);
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
  currentView.value = "startup";
});

describe("StartupView 打开既有 hub", () => {
  it("无 lastHub：停在启动页，输入路径打开后 setHub+lastHub+进主界面", async () => {
    mountView();
    await new Promise((r) => setTimeout(r, 0));
    expect(currentView.value).toBe("startup");

    setInput("hub 目录路径", "/hub");
    await nextTick();
    clickButton("打开 Hub");
    await vi.waitFor(() => expect(mocks.hubExists).toHaveBeenCalledWith("/hub"));
    await vi.waitFor(() => expect(currentView.value).toBe("main"));
    expect(useHubStore().path).toBe("/hub");
    expect(useSettingsStore().lastHub).toBe("/hub");
  });

  it("路径不是 hub：hubExists false → 错误提示、不进入", async () => {
    mountView();
    await new Promise((r) => setTimeout(r, 0));
    mocks.hubExists.mockResolvedValue(false);

    setInput("hub 目录路径", "/not-a-hub");
    await nextTick();
    clickButton("打开 Hub");
    await vi.waitFor(() => expect(el!.textContent).toContain("该目录不是 gws hub"));
    expect(currentView.value).toBe("startup");
  });
});

describe("StartupView 新建 hub（gws init）", () => {
  it("点入口开弹窗：父目录+新目录名必填才可创建；gws init 于父目录执行目标 <父>/<名>", async () => {
    mountView();
    await new Promise((r) => setTimeout(r, 0));

    clickButton("没有 hub？新建一个 →");
    await nextTick();
    const dialog = el!.querySelector(".dialog")!;
    expect(dialog).toBeTruthy();
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
    // 成功：直达主界面并记住新 hub
    await vi.waitFor(() => expect(currentView.value).toBe("main"));
    expect(useHubStore().path).toBe("/Users/demo/dev/myhub");
    expect(useSettingsStore().lastHub).toBe("/Users/demo/dev/myhub");
  });

  it("gws init 失败（exit 1，如目标已存在/嵌套）：留在弹窗可改可取消，不进入主界面", async () => {
    mountView();
    await new Promise((r) => setTimeout(r, 0));
    clickButton("没有 hub？新建一个 →");
    await nextTick();

    setInput("父目录", "/Users/demo/dev");
    setInput("如 myhub", "myhub");
    await nextTick();
    clickButton("创建");
    await exitWith(1, 1);
    await new Promise((r) => setTimeout(r, 10));
    expect(currentView.value).toBe("startup");
    expect(el!.querySelector(".dialog")).toBeTruthy(); // 弹窗保留（原因见命令弹窗）
    // 取消可关弹窗
    clickButton("取消");
    await nextTick();
    expect(el!.querySelector(".dialog")).toBeNull();
  });

  it("浏览选择父目录：open 返回路径填入父目录输入框", async () => {
    mountView();
    await new Promise((r) => setTimeout(r, 0));
    clickButton("没有 hub？新建一个 →");
    await nextTick();
    mocks.openDialog.mockResolvedValue("/Users/demo/dev");

    // 弹窗内的「浏览…」（外层打开 hub 的同名按钮也在，须限定弹窗内）
    const dialog = el!.querySelector(".dialog")!;
    const browse = Array.from(dialog.querySelectorAll<HTMLButtonElement>("button"))
      .find((b) => b.textContent?.trim() === "浏览…")!;
    browse.click();
    await vi.waitFor(() =>
      expect(Array.from(el!.querySelectorAll<HTMLInputElement>("input"))
        .find((i) => i.placeholder.includes("父目录"))!.value).toBe("/Users/demo/dev"),
    );
  });
});
