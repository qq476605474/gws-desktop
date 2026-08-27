// @vitest-environment happy-dom
// 无 @vue/test-utils：手动 createApp().mount() 挂载 SFC；
// gws-bridge / tauri 插件 mock 模式参考 src/stores/cmd.test.ts、settings.test.ts
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createApp, nextTick, type App } from "vue";
import { createPinia, setActivePinia } from "pinia";

type Payload = Record<string, unknown>;
type Handler = (e: { payload: Payload }) => void;
type RunResult = { code: number | null; output: string };

const mocks = vi.hoisted(() => ({
  runGws: vi.fn<(args: string[], cwd: string) => Promise<RunResult>>(),
  runGwsStream: vi.fn<(args: string[], cwd: string, confirmTimeoutMs?: number) => Promise<number>>(),
  respondConfirm: vi.fn<(runId: number, yes: boolean) => Promise<void>>(),
  replayOutput: vi.fn<(runId: number) => Promise<void>>(),
  openInFinder: vi.fn<(path: string) => Promise<void>>(),
  openInTerminal: vi.fn<(path: string, terminal: string | null) => Promise<void>>(),
}));

// 组件链上 hub/cmd store 与 PathActions 均从该模块导入，mock 须补齐全部具名导出
vi.mock("../lib/gws-bridge", () => ({
  runGws: mocks.runGws,
  runGwsStream: mocks.runGwsStream,
  respondConfirm: mocks.respondConfirm,
  replayOutput: mocks.replayOutput,
  openInFinder: mocks.openInFinder,
  openInTerminal: mocks.openInTerminal,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-store", () => ({
  load: vi.fn(),
  Store: class Store {},
}));

import { useHubStore } from "../stores/hub";
import WorkspaceDetail from "./WorkspaceDetail.vue";

let app: App | null = null;
let el: HTMLElement | null = null;

/** 每次 runGws 调用挂起，由测试按序 resolve——用于模拟并发 refresh 的乱序返回 */
const pending: Array<{ resolve: (v: RunResult) => void }> = [];

function mountDetail() {
  const pinia = createPinia();
  setActivePinia(pinia);
  const hub = useHubStore();
  hub.setHub("/hub");
  el = document.createElement("div");
  document.body.appendChild(el);
  app = createApp(WorkspaceDetail, { name: "demo", onClose: () => {} });
  app.use(pinia);
  app.mount(el);
}

/** 含单模块的 gws st 输出夹具（无 ANSI，parse 系列可处理） */
function stOut(mod: string): string {
  return "结算流程改版  feature-20260818-checkout-revamp\n\n" +
    "  模块              改动    vs远程    vs主干\n" +
    `  ${mod}            0       ↑2 ↓0     +5\n`;
}

function clickRetry() {
  const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
    .find((b) => b.textContent?.includes("重试"));
  if (!btn) throw new Error("重试按钮未找到");
  btn.click();
}

beforeEach(() => {
  mocks.runGws.mockImplementation(() => new Promise<RunResult>((resolve) => pending.push({ resolve })));
});

afterEach(() => {
  app?.unmount();
  app = null;
  el?.remove();
  el = null;
  pending.length = 0;
  vi.clearAllMocks();
});

describe("WorkspaceDetail.refresh 错误处理与并发守卫", () => {
  it("gws st 非零退出：显示错误行与重试按钮，不把错误输出渲染成空表、不死“加载中”", async () => {
    mountDetail();
    expect(mocks.runGws).toHaveBeenCalledWith(["st"], "/hub/ws/demo");
    pending[0]!.resolve({ code: 1, output: "gws: 工作区不存在\nfatal: 仓库已损坏" });
    await vi.waitFor(() => expect(el!.textContent).toContain("fatal: 仓库已损坏"));
    expect(el!.querySelector("table")).toBeNull(); // 错误文本不得经 parseSt 渲染成“成功”空表
    expect(el!.textContent).not.toContain("加载中");
    expect(el!.textContent).toContain("重试");
  });

  it("spawn 失败（code null）：错误信息直接展示", async () => {
    mountDetail();
    pending[0]!.resolve({ code: null, output: "启动失败: gws 未安装" });
    await vi.waitFor(() => expect(el!.textContent).toContain("启动失败: gws 未安装"));
    expect(el!.querySelector("table")).toBeNull();
  });

  it("runGws reject：stErr = String(e)，不产生 unhandled rejection", async () => {
    mocks.runGws.mockImplementationOnce(() => Promise.reject(new Error("invoke 失败")));
    mountDetail();
    await vi.waitFor(() => expect(el!.textContent).toContain("Error: invoke 失败"));
    expect(el!.querySelector("table")).toBeNull();
    expect(el!.textContent).toContain("重试");
  });

  it("重试成功：表格渲染、错误行消失", async () => {
    mountDetail();
    pending[0]!.resolve({ code: 0, output: stOut("order-service") });
    await vi.waitFor(() => expect(el!.querySelector("table")).toBeTruthy());
    expect(el!.textContent).toContain("order-service");
    expect(el!.textContent).toContain("feature-20260818-checkout-revamp");
    expect(el!.textContent).not.toContain("重试");
  });

  it("并发守卫：连续两次重试，旧响应迟到不覆盖最新结果", async () => {
    mountDetail();
    // 首次失败 → 出现错误行与重试按钮
    pending[0]!.resolve({ code: 1, output: "gws: 目录不存在" });
    await vi.waitFor(() => expect(el!.textContent).toContain("gws: 目录不存在"));

    // 连点两次重试 → refresh #2、#3 两个在途请求
    clickRetry();
    clickRetry();
    expect(mocks.runGws).toHaveBeenCalledTimes(3);

    // 旧请求（#2）迟到的成功响应必须被丢弃（n !== seq）
    pending[1]!.resolve({ code: 0, output: stOut("old-mod") });
    await nextTick();
    expect(el!.querySelector("table")).toBeNull();
    expect(el!.textContent).not.toContain("old-mod");

    // 最新请求（#3）的结果被采纳，错误行清空
    pending[2]!.resolve({ code: 0, output: stOut("new-mod") });
    await vi.waitFor(() => expect(el!.querySelector("table")).toBeTruthy());
    expect(el!.textContent).toContain("new-mod");
    expect(el!.textContent).not.toContain("old-mod");
    expect(el!.textContent).not.toContain("重试");
  });
});

describe("WorkspaceDetail 命令操作（弹窗式 execDialog）", () => {
  function clickText(text: string) {
    const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
      .find((b) => b.textContent?.trim() === text);
    if (!btn) throw new Error(`按钮「${text}」未找到`);
    btn.click();
  }

  /** 先让 st 表格渲染出来（refresh 就绪），再触发命令按钮 */
  async function mountReady() {
    mountDetail();
    mocks.runGwsStream.mockResolvedValue(1);
    mocks.replayOutput.mockResolvedValue(undefined);
    pending[0]!.resolve({ code: 0, output: stOut("order-service") });
    await vi.waitFor(() => expect(el!.querySelector("table")).toBeTruthy());
  }

  it("doCmd 走 execDialog：默认 confirmTimeoutMs=30000（慢命令防假确认）", async () => {
    await mountReady();
    clickText("Pull");
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(["pull"], "/hub/ws/demo", 30000),
    );
  });

  it("gws drop 单独传 confirmTimeoutMs=1500（GUI 下唯一真读 stdin 的命令）", async () => {
    await mountReady();
    clickText("移除");
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(["drop", "order-service"], "/hub/ws/demo", 1500),
    );
  });

  it("removeWs 走 execDialog：gws rm 于 hub.path、默认 30000", async () => {
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(confirm).mockResolvedValue(true);
    await mountReady();
    clickText("删除工作区");
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(["rm", "demo", "--force"], "/hub", 30000),
    );
  });
});
