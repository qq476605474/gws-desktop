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
  runGwsStream: vi.fn<(args: string[], cwd: string, confirmTimeoutMs?: number) => Promise<number>>(),
  respondConfirm: vi.fn<(runId: number, yes: boolean) => Promise<void>>(),
  replayOutput: vi.fn<(runId: number) => Promise<void>>(),
  openInFinder: vi.fn<(path: string) => Promise<void>>(),
  openInTerminal: vi.fn<(path: string, terminal: string | null) => Promise<void>>(),
  listDir: vi.fn<(path: string) => Promise<string[]>>(),
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
  listDir: mocks.listDir,
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
import EnvsTab from "./EnvsTab.vue";

let app: App | null = null;
let el: HTMLElement | null = null;
let nextId = 0;

/** 挂载 Tab（env 列表直接写入 hub store，模拟 WorkspacesTab 已刷新过的状态） */
function mountTab() {
  const pinia = createPinia();
  setActivePinia(pinia);
  const hub = useHubStore();
  hub.setHub("/hub");
  hub.envs = ["dev", "pre"];
  el = document.createElement("div");
  document.body.appendChild(el);
  app = createApp(EnvsTab);
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
  mocks.listDir.mockResolvedValue([]);
});

afterEach(() => {
  app?.unmount();
  app = null;
  el?.remove();
  el = null;
  mocks.handlers.clear();
  vi.clearAllMocks();
});

describe("EnvsTab", () => {
  it("addEnv：args 与 cwd 正确，成功后清空输入并刷新", async () => {
    mountTab();
    setInput("uat");
    await nextTick();
    clickButton("+ 添加环境");

    await vi.waitFor(() => expect(mocks.runGwsStream).toHaveBeenCalledTimes(1));
    expect(mocks.runGwsStream).toHaveBeenCalledWith(["env", "add", "uat"], "/hub", 30000);

    // 负向断言：exit 事件未发出、命令未到终态前不得提前 refreshAll（锁死 waitDone→refreshAll 时序）
    expect(mocks.runGws).not.toHaveBeenCalled();

    await exitWith(1, 0);
    // 等终态后才 refreshAll（waitDone 模式）：三条 ls 均在命令结束后发起
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["env", "ls"], "/hub"));
    await nextTick();
    expect(inputValue()).toBe("");
  });

  it("addEnv：exec reject（IPC 失败）不崩，refreshAll 仍被调，输入保留便于重试", async () => {
    mountTab();
    mocks.runGwsStream.mockRejectedValueOnce(new Error("invoke 失败"));
    setInput("uat");
    await nextTick();
    clickButton("+ 添加环境");

    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["ls"], "/hub"));
    await nextTick();
    expect(inputValue()).toBe("uat");
  });

  it("addEnv：命令失败（退出码 1）不清输入，仍刷新", async () => {
    mountTab();
    setInput("uat");
    await nextTick();
    clickButton("+ 添加环境");

    await exitWith(1, 1); // env add 失败
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["env", "ls"], "/hub"));
    await nextTick();
    expect(inputValue()).toBe("uat");
  });

  it("sync：于 hub.path 执行 gws sync，结束后刷新", async () => {
    mountTab();
    clickButton("同步最新代码");

    await vi.waitFor(() => expect(mocks.runGwsStream).toHaveBeenCalledWith(["sync"], "/hub", 30000));
    await exitWith(1, 0);
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["env", "ls"], "/hub"));
  });

  it("rmEnv：confirm 取消不发命令；确认后于 hub.path 执行 env rm <name> 并刷新", async () => {
    mountTab();
    mocks.confirm.mockResolvedValueOnce(false);
    clickButton("移除"); // 首行 dev

    await vi.waitFor(() => expect(mocks.confirm).toHaveBeenCalledWith("移除环境 dev？"));
    await nextTick();
    expect(mocks.runGwsStream).not.toHaveBeenCalled();
    expect(mocks.runGws).not.toHaveBeenCalled();

    mocks.confirm.mockResolvedValueOnce(true);
    clickButton("移除");
    await vi.waitFor(() => expect(mocks.runGwsStream).toHaveBeenCalledWith(["env", "rm", "dev"], "/hub", 30000));
    // 取消分支未发命令，本次确认是本测试第一个 exec → runId 1
    await exitWith(1, 0);
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["env", "ls"], "/hub"));
  });

  it("rmEnv：confirm 确认时已有命令在跑（isRunning）→ 不 exec 不刷新", async () => {
    mountTab();
    // confirm 挂起：模拟原生对话框打开期间，用户已从另一入口（sync）启动命令
    let resolveConfirm!: (v: boolean) => void;
    mocks.confirm.mockImplementationOnce(
      () => new Promise<boolean>((r) => { resolveConfirm = r; }),
    );
    clickButton("移除"); // 首行 dev；点击时无命令在跑，按钮可点
    await vi.waitFor(() => expect(mocks.confirm).toHaveBeenCalledWith("移除环境 dev？"));

    clickButton("同步最新代码"); // 弹窗打开期间另一入口启动命令
    // gws-exit:1 订阅就绪时 exec 已设置 current（state=running）→ isRunning() 为 true
    await vi.waitFor(() => expect(mocks.handlers.get("gws-exit:1")).toBeTruthy());

    resolveConfirm(true); // 用户点击“确认”
    await new Promise((r) => setTimeout(r, 0)); // 冲微任务：rmEnv 应在 isRunning 守卫处直接返回
    expect(mocks.runGwsStream).toHaveBeenCalledTimes(1); // 仅 sync，未发 env rm
    expect(mocks.runGwsStream).toHaveBeenCalledWith(["sync"], "/hub", 30000);
    expect(mocks.runGws).not.toHaveBeenCalled(); // rmEnv 被拦截，sync 未终态，均未触发刷新

    await exitWith(1, 0); // sync 正常收尾后仍刷新
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["env", "ls"], "/hub"));
  });
});

describe("EnvsTab 环境行展开模块", () => {
  /** 第 idx 个环境行（.env-row 点击切换展开/收起） */
  function envRow(idx: number): HTMLElement {
    const row = el!.querySelectorAll<HTMLElement>(".env-row")[idx];
    if (!row) throw new Error(`第 ${idx} 个环境行未找到`);
    return row;
  }

  it("点击环境行：调 listDir 于 envs/<名> 并展开显示模块；再点收起；收起再展开用缓存不重拉", async () => {
    mountTab();
    mocks.listDir.mockResolvedValue(["cart-service", "user-web"]);
    envRow(0).click(); // dev

    await vi.waitFor(() => expect(mocks.listDir).toHaveBeenCalledWith("/hub/envs/dev"));
    await vi.waitFor(() => {
      expect(el!.textContent).toContain("cart-service");
      expect(el!.textContent).toContain("user-web");
    });

    envRow(0).click(); // 收起
    await nextTick();
    expect(el!.querySelector(".env-modules")).toBeNull();
    expect(el!.textContent).not.toContain("cart-service");

    mocks.listDir.mockClear();
    envRow(0).click(); // 再展开：走缓存
    await nextTick();
    expect(el!.textContent).toContain("cart-service");
    expect(mocks.listDir).not.toHaveBeenCalled();
  });

  it("rmEnv 成功后清展开缓存：删后重建同名环境再展开会重新 listDir", async () => {
    mountTab();
    mocks.listDir.mockResolvedValue(["cart-service"]);
    envRow(0).click(); // 展开 dev
    await vi.waitFor(() => expect(mocks.listDir).toHaveBeenCalledWith("/hub/envs/dev"));
    await vi.waitFor(() => expect(el!.textContent).toContain("cart-service"));
    mocks.listDir.mockClear();

    clickButton("移除"); // 首行 dev（confirm 默认 true）
    await vi.waitFor(() => expect(mocks.runGwsStream).toHaveBeenCalledWith(["env", "rm", "dev"], "/hub", 30000));
    await exitWith(1, 0);
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["env", "ls"], "/hub"));

    // 同会话重建同名环境（模拟后续刷新列表再现 dev）：再展开必须重新拉取，
    // 不得命中 rmEnv 前的旧缓存（否则 listDir 不会被再次调用）
    useHubStore().envs = ["dev", "pre"];
    await nextTick();
    envRow(0).click();
    await vi.waitFor(() => expect(mocks.listDir).toHaveBeenCalledWith("/hub/envs/dev"));
    await vi.waitFor(() => expect(el!.textContent).toContain("cart-service"));
  });

  it("sync 后失效已展开行的模块缓存：listDir 对 dev 再次调用", async () => {
    mountTab();
    mocks.listDir.mockResolvedValue(["cart-service"]);
    envRow(0).click(); // 展开 dev（listDir 调 1 次显示模块）
    await vi.waitFor(() => expect(mocks.listDir).toHaveBeenCalledWith("/hub/envs/dev"));
    await vi.waitFor(() => expect(el!.textContent).toContain("cart-service"));
    mocks.listDir.mockClear();

    clickButton("同步最新代码");
    await vi.waitFor(() => expect(mocks.runGwsStream).toHaveBeenCalledWith(["sync"], "/hub", 30000));
    await exitWith(1, 0);
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["env", "ls"], "/hub"));
    // sync 补建模块目录会改变 envs/<名> 下的内容：invalidateModules 重拉已展开行
    await vi.waitFor(() => expect(mocks.listDir).toHaveBeenCalledWith("/hub/envs/dev"));
  });

  it("模块为空：显示（无模块，跑 gws sync 补建）", async () => {
    mountTab();
    mocks.listDir.mockResolvedValue([]);
    envRow(0).click();

    await vi.waitFor(() => expect(el!.textContent).toContain("（无模块，跑 gws sync 补建）"));
  });

  it("listDir 失败：展开区显示错误小字", async () => {
    mountTab();
    mocks.listDir.mockRejectedValue("列出目录失败 /hub/envs/dev: no such file");
    envRow(0).click();

    await vi.waitFor(() => expect(el!.textContent).toContain("列出目录失败 /hub/envs/dev"));
  });

  it("PathActions/移除按钮点击不触发展开（@click.stop）", async () => {
    mountTab();
    mocks.confirm.mockResolvedValue(false); // 移除走取消分支，避免引入命令执行
    const row = envRow(0); // dev

    // 移除按钮：confirm 弹出但行不展开
    const rm = Array.from(row.querySelectorAll<HTMLButtonElement>("button"))
      .find((b) => b.textContent?.trim() === "移除")!;
    rm.click();
    await vi.waitFor(() => expect(mocks.confirm).toHaveBeenCalledWith("移除环境 dev？"));
    expect(mocks.listDir).not.toHaveBeenCalled();
    expect(el!.querySelector(".env-modules")).toBeNull();

    // PathActions 复制路径按钮：剪贴板被写但行不展开
    const copy = row.querySelector<HTMLButtonElement>('button[title="复制路径"]')!;
    copy.click();
    await new Promise((r) => setTimeout(r, 0));
    expect(mocks.listDir).not.toHaveBeenCalled();
    expect(el!.querySelector(".env-modules")).toBeNull();
  });
});
