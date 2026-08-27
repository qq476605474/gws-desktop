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

describe("WorkspaceDetail 命令操作（弹窗式 execDialog）", () => {
  it("doCmd 走 execDialog：默认 confirmTimeoutMs=30000（慢命令防假确认）；Pull 系已改走 confirm，用 Done 校验触发纯 doCmd", async () => {
    await mountReady();
    clickText("Done 校验");
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(["done"], "/hub/ws/demo", 30000),
    );
  });

  it("gws drop 先原生 confirm，确认后单独传 confirmTimeoutMs=1500（GUI 下唯一真读 stdin 的命令）", async () => {
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(confirm).mockResolvedValue(true);
    await mountReady();
    clickText("移除");
    await vi.waitFor(() =>
      expect(confirm).toHaveBeenCalledWith("确认移除模块 order-service？（未推送提交时 gws 会再次确认是否丢弃）"),
    );
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

describe("WorkspaceDetail 写操作前确认（Pull/Push/Merge 系列统一 confirmThenDo）", () => {
  it("Push：confirm 取消 → 不执行命令；Pull 同为写操作（更新本地），取消同样不执行、确认后执行", async () => {
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(confirm).mockResolvedValue(false);
    await mountReady();
    clickText("Push");
    // 等一拍确认 confirm 已被调用且命令未发起
    await vi.waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
    await new Promise((r) => setTimeout(r, 10));
    expect(mocks.runGwsStream).not.toHaveBeenCalled();
    // 确认弹窗文案带上当前分支
    expect(vi.mocked(confirm).mock.calls[0]![0]).toContain("feature-20260818-checkout-revamp");
    // Pull 不再是读侧直通：亦经 confirm，取消 → 不执行
    clickText("Pull");
    await vi.waitFor(() => expect(confirm).toHaveBeenCalledTimes(2));
    expect(vi.mocked(confirm).mock.calls[1]![0]).toContain("确认拉取远程更新到当前分支");
    await new Promise((r) => setTimeout(r, 10));
    expect(mocks.runGwsStream).not.toHaveBeenCalled();
    // 确认 → 执行 gws pull（默认 30000）
    vi.mocked(confirm).mockResolvedValue(true);
    clickText("Pull");
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(["pull"], "/hub/ws/demo", 30000),
    );
  });

  it("Pull --rebase 亦经 confirm：取消不执行；确认后 pull --rebase（默认 30000）", async () => {
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(confirm).mockResolvedValue(false);
    await mountReady();
    clickText("Pull --rebase");
    await vi.waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
    // 文案警示变基后果（本地未推送提交会被变基）
    expect(vi.mocked(confirm).mock.calls[0]![0]).toContain("确认以 rebase 方式拉取远程更新");
    await new Promise((r) => setTimeout(r, 10));
    expect(mocks.runGwsStream).not.toHaveBeenCalled();

    vi.mocked(confirm).mockResolvedValue(true);
    clickText("Pull --rebase");
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(["pull", "--rebase"], "/hub/ws/demo", 30000),
    );
  });

  it("Push：confirm 确认 → 执行 gws push（默认 30000）", async () => {
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(confirm).mockResolvedValue(true);
    await mountReady();
    clickText("Push");
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(["push"], "/hub/ws/demo", 30000),
    );
  });

  it("重入守卫：confirm pending 期间再点 Push 不发起第二次 confirm（防双击排队两次确认）", async () => {
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    // 第一次 confirm 挂起，模拟原生对话框等待用户点击的 IPC 间隙
    let resolveFirst: (v: boolean) => void = () => {};
    vi.mocked(confirm).mockImplementationOnce(
      () => new Promise<boolean>((r) => { resolveFirst = r; }),
    );
    await mountReady();
    clickText("Push");
    await vi.waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
    clickText("Push"); // run 尚未启动（isRunning 不生效）：须由本地守卫拦下
    await new Promise((r) => setTimeout(r, 10));
    expect(confirm).toHaveBeenCalledTimes(1);
    // 收尾：取消确认 → 不执行命令
    resolveFirst(false);
    await new Promise((r) => setTimeout(r, 10));
    expect(mocks.runGwsStream).not.toHaveBeenCalled();
  });

  it("Merge+Push：confirm 取消 → 不执行；确认 → merge <env> --push", async () => {
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(confirm).mockResolvedValue(false);
    await mountReady();
    const hub = useHubStore();
    hub.envs = ["dev", "pre"];
    await nextTick();
    // 选择环境 pre（select v-model 监听 change）
    const select = el!.querySelector<HTMLSelectElement>("select")!;
    select.value = "pre";
    select.dispatchEvent(new Event("change"));
    await nextTick();

    clickText("Merge+Push");
    await vi.waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
    expect(vi.mocked(confirm).mock.calls[0]![0]).toContain("pre");
    await new Promise((r) => setTimeout(r, 10));
    expect(mocks.runGwsStream).not.toHaveBeenCalled();

    vi.mocked(confirm).mockResolvedValue(true);
    clickText("Merge+Push");
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(["merge", "pre", "--push"], "/hub/ws/demo", 30000),
    );
  });

  it("Merge（本地）：confirm 取消 → 不执行；确认 → 执行 merge <env>（本地合并误点同样危险）", async () => {
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(confirm).mockResolvedValue(false);
    await mountReady();
    const hub = useHubStore();
    hub.envs = ["dev"];
    await nextTick();
    const select = el!.querySelector<HTMLSelectElement>("select")!;
    select.value = "dev";
    select.dispatchEvent(new Event("change"));
    await nextTick();
    clickText("Merge（本地）");
    await vi.waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
    expect(vi.mocked(confirm).mock.calls[0]![0]).toContain("dev");
    await new Promise((r) => setTimeout(r, 10));
    expect(mocks.runGwsStream).not.toHaveBeenCalled();

    vi.mocked(confirm).mockResolvedValue(true);
    clickText("Merge（本地）");
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(["merge", "dev"], "/hub/ws/demo", 30000),
    );
  });
});

describe("WorkspaceDetail 同步最新代码（sync-main，--from 可选）", () => {
  it("工具栏按钮布局：Sync/Sync-main 已移除（gws sync 归仓库/环境 tab），保留「同步最新代码」", async () => {
    await mountReady();
    const texts = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
      .map((b) => b.textContent?.trim());
    expect(texts).not.toContain("Sync");
    expect(texts).not.toContain("Sync-main");
    expect(texts).toContain("同步最新代码");
  });

  /** 点「同步最新代码」打开 SyncMainDialog 并等其渲染 */
  async function openSyncMain() {
    await mountReady();
    clickText("同步最新代码");
    await vi.waitFor(() => expect(el!.querySelector(".dialog")).toBeTruthy());
  }

  /** SyncMainDialog 内的 from 输入框填值（v-model 监听 input） */
  async function setFrom(value: string) {
    const input = el!.querySelector<HTMLInputElement>(".dialog input")!;
    input.value = value;
    input.dispatchEvent(new Event("input"));
    await nextTick();
  }

  it("输入 from → 开始同步 → confirm 确认：执行 sync-main --yes --from <ref>，弹窗即关", async () => {
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(confirm).mockResolvedValue(true);
    await openSyncMain();
    await setFrom("release-2.0");
    clickText("开始同步");
    // run 后弹窗立即关闭（执行由父组件 confirmThenDo 接手）
    await vi.waitFor(() => expect(el!.querySelector(".dialog")).toBeNull());
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(
        ["sync-main", "--yes", "--from", "release-2.0"],
        "/hub/ws/demo",
        30000,
      ),
    );
    // 确认文案带上来源分支
    expect(vi.mocked(confirm).mock.calls[0]![0]).toContain("release-2.0");
  });

  it("留空 from → 开始同步：args 无 --from（走创建时基线）", async () => {
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(confirm).mockResolvedValue(true);
    await openSyncMain();
    clickText("开始同步");
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(["sync-main", "--yes"], "/hub/ws/demo", 30000),
    );
    expect(vi.mocked(confirm).mock.calls[0]![0]).toContain("创建时基线");
  });

  it("confirm 取消 → 不执行命令（sync-main 合并属变更操作，须确认）", async () => {
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(confirm).mockResolvedValue(false);
    await openSyncMain();
    await setFrom("release-2.0");
    clickText("开始同步");
    await vi.waitFor(() => expect(confirm).toHaveBeenCalledTimes(1));
    await new Promise((r) => setTimeout(r, 10));
    expect(mocks.runGwsStream).not.toHaveBeenCalled();
  });
});

describe("WorkspaceDetail 头部刷新按钮", () => {
  it("点击刷新 → 重新拉 gws st，新结果渲染", async () => {
    await mountReady();
    expect(mocks.runGws).toHaveBeenCalledTimes(1);
    clickText("刷新");
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledTimes(2));
    expect(mocks.runGws).toHaveBeenLastCalledWith(["st"], "/hub/ws/demo");
    pending[1]!.resolve({ code: 0, output: stOut("fresh-module") });
    await vi.waitFor(() => expect(el!.textContent).toContain("fresh-module"));
  });

  it("命令运行中刷新按钮禁用（本文件 listen mock 为 no-op，无法模拟 exit，恢复半程不在此验证）", async () => {
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(confirm).mockResolvedValue(true); // Pull 已改走 confirmThenDo：先过原生确认才 exec
    await mountReady();
    clickText("Pull");
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(["pull"], "/hub/ws/demo", 30000),
    );
    // 命令未结束（无 exit 事件）→ 运行中：刷新禁用
    const refreshBtn = () => Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
      .find((b) => b.textContent?.trim() === "刷新")!;
    await vi.waitFor(() => expect(refreshBtn().disabled).toBe(true));
    expect(mocks.runGws).toHaveBeenCalledTimes(1); // 未重复拉取
  });
});
