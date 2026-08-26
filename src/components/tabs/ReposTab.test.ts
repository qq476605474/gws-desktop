// @vitest-environment happy-dom
// 无 @vue/test-utils：手动 createApp().mount() 挂载 SFC；
// gws-bridge / tauri 事件与插件 mock 模式参考 src/components/WorkspaceDetail.test.ts
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createApp, nextTick, type App } from "vue";
import { createPinia, setActivePinia } from "pinia";

type Payload = Record<string, unknown>;
type Handler = (e: { payload: Payload }) => void;
type RunResult = { code: number | null; output: string };

// vi.mock 工厂随 import 提升、早于本文件函数体执行，共享状态须经 vi.hoisted 创建以避免 TDZ
const mocks = vi.hoisted(() => ({
  runGws: vi.fn<(args: string[], cwd: string) => Promise<RunResult>>(),
  runGwsStream: vi.fn<(args: string[], cwd: string) => Promise<number>>(),
  respondConfirm: vi.fn<(runId: number, yes: boolean) => Promise<void>>(),
  replayOutput: vi.fn<(runId: number) => Promise<void>>(),
  openInFinder: vi.fn<(path: string) => Promise<void>>(),
  openInTerminal: vi.fn<(path: string, terminal: string | null) => Promise<void>>(),
  confirm: vi.fn<(message: string) => Promise<boolean>>(),
  /** 按事件名保存 listen 注册的 handler，测试中手动触发以模拟 gws-exit 事件 */
  handlers: new Map<string, Handler>(),
}));

// 组件链上 hub/cmd store 与 PathActions 均从该模块导入，mock 须补齐全部具名导出
vi.mock("../../lib/gws-bridge", () => ({
  runGws: mocks.runGws,
  runGwsStream: mocks.runGwsStream,
  respondConfirm: mocks.respondConfirm,
  replayOutput: mocks.replayOutput,
  openInFinder: mocks.openInFinder,
  openInTerminal: mocks.openInTerminal,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: Handler) => {
    mocks.handlers.set(event, handler);
    return () => mocks.handlers.delete(event);
  }),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: mocks.confirm,
}));

vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-store", () => ({
  load: vi.fn(),
  Store: class Store {},
}));

import { useHubStore } from "../../stores/hub";
import ReposTab from "./ReposTab.vue";

let app: App | null = null;
let el: HTMLElement | null = null;
let nextId = 0;

/** 挂载 Tab（repo 列表直接写入 hub store，模拟 WorkspacesTab 已刷新过的状态） */
function mountTab() {
  const pinia = createPinia();
  setActivePinia(pinia);
  const hub = useHubStore();
  hub.setHub("/hub");
  hub.repos = [
    { name: "order-service", mainBranch: "main" },
    { name: "admin-web", mainBranch: "master" },
  ];
  el = document.createElement("div");
  document.body.appendChild(el);
  app = createApp(ReposTab);
  app.use(pinia);
  app.mount(el);
}

function setInput(value: string) {
  const input = el!.querySelector<HTMLInputElement>("input");
  if (!input) throw new Error("输入框未找到");
  input.value = value;
  input.dispatchEvent(new Event("input"));
}

function inputValue(): string {
  return el!.querySelector<HTMLInputElement>("input")!.value;
}

function clickButton(text: string) {
  const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
    .find((b) => b.textContent?.trim() === text);
  if (!btn) throw new Error(`按钮「${text}」未找到`);
  btn.click();
}

/** 等待第 runId 次 exec 完成 listen 订阅后发出退出码（runId 从 1 递增） */
async function exitWith(runId: number, code: number) {
  await vi.waitFor(() => expect(mocks.handlers.get(`gws-exit:${runId}`)).toBeTruthy());
  mocks.handlers.get(`gws-exit:${runId}`)!({ payload: { code } });
}

beforeEach(() => {
  nextId = 0;
  mocks.runGwsStream.mockImplementation(async () => ++nextId);
  mocks.runGws.mockResolvedValue({ code: 0, output: "" });
  mocks.replayOutput.mockResolvedValue(undefined);
  mocks.confirm.mockResolvedValue(true);
});

afterEach(() => {
  app?.unmount();
  app = null;
  el?.remove();
  el = null;
  mocks.handlers.clear();
  vi.clearAllMocks();
});

describe("ReposTab", () => {
  it("addRepos：多 URL 空格分隔展开为 repo add 参数于 hub.path，成功后清空输入并刷新", async () => {
    mountTab();
    setInput("https://git.example.com/a.git   https://git.example.com/b.git");
    await nextTick();
    clickButton("+ 添加仓库");

    await vi.waitFor(() => expect(mocks.runGwsStream).toHaveBeenCalledTimes(1));
    expect(mocks.runGwsStream).toHaveBeenCalledWith(
      ["repo", "add", "https://git.example.com/a.git", "https://git.example.com/b.git"],
      "/hub",
    );

    await exitWith(1, 0);
    // 等终态后才 refreshAll（waitDone 模式）：三条 ls 均在命令结束后发起
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["repo", "ls"], "/hub"));
    await nextTick();
    expect(inputValue()).toBe("");
  });

  it("addRepos：exec reject（IPC 失败）不崩，refreshAll 仍被调，输入保留便于重试", async () => {
    mountTab();
    mocks.runGwsStream.mockRejectedValueOnce(new Error("invoke 失败"));
    setInput("https://git.example.com/a.git");
    await nextTick();
    clickButton("+ 添加仓库");

    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["ls"], "/hub"));
    await nextTick();
    expect(inputValue()).toBe("https://git.example.com/a.git");
  });

  it("addRepos：命令失败（非零退出码）不清输入，仍刷新", async () => {
    mountTab();
    setInput("https://git.example.com/a.git");
    await nextTick();
    clickButton("+ 添加仓库");

    await exitWith(1, 1); // repo add 失败
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["repo", "ls"], "/hub"));
    await nextTick();
    expect(inputValue()).toBe("https://git.example.com/a.git");
  });

  it("rm：confirm 取消 → 不发命令、不刷新", async () => {
    mountTab();
    mocks.confirm.mockResolvedValueOnce(false);
    clickButton("移除"); // 首行 order-service

    await vi.waitFor(() => expect(mocks.confirm).toHaveBeenCalledWith("移除仓库 order-service？"));
    await nextTick();
    expect(mocks.runGwsStream).not.toHaveBeenCalled();
    expect(mocks.runGws).not.toHaveBeenCalled();
  });

  it("rm：confirm 确认 → 于 hub.path 执行 repo rm <name>，结束后刷新", async () => {
    mountTab();
    clickButton("移除");
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(["repo", "rm", "order-service"], "/hub"),
    );
    await exitWith(1, 0);
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["repo", "ls"], "/hub"));
  });
});
