// @vitest-environment happy-dom
// 无 @vue/test-utils：手动 createApp().mount() 挂载 SFC；
// gws-bridge / tauri 事件与插件 mock 模式参考 src/components/WorkspaceDetail.test.ts
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createApp, type App } from "vue";
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
  openPath: vi.fn<(path: string) => Promise<void>>(),
  copyText: vi.fn<(text: string) => Promise<void>>(),
  /** 按事件名保存 listen 注册的 handler，测试中手动触发以模拟事件流 */
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
  openPath: mocks.openPath,
  copyText: mocks.copyText,
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
  confirm: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-store", () => ({
  load: vi.fn(),
  Store: class Store {},
}));

import { useHubStore } from "../../stores/hub";
import WorkspacesTab from "./WorkspacesTab.vue";

let app: App | null = null;
let el: HTMLElement | null = null;

/** gws ls 输出夹具：两个工作区（无 ANSI，parseLs 可解析） */
const LS_OUT =
  "名称              标题            阶段    模块    分支\n" +
  "demo              结算改版        dev     3       feature-20260818-demo\n" +
  "cart              购物车改版      dev     2       feature-20260818-cart\n";

/** demo 工作区的 gws st 输出夹具（同 WorkspaceDetail.test.ts 形态） */
const ST_OUT =
  "结算改版  feature-20260818-demo\n\n" +
  "  模块              改动    vs远程    vs主干\n" +
  "  order-service     0       ↑2 ↓0     +5\n";

function mountTab() {
  const pinia = createPinia();
  setActivePinia(pinia);
  const hub = useHubStore();
  hub.setHub("/hub");
  el = document.createElement("div");
  document.body.appendChild(el);
  app = createApp(WorkspacesTab);
  app.use(pinia);
  app.mount(el);
}

/** 点击文本匹配的按钮（trim 后全等） */
function clickButton(text: string) {
  const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
    .find((b) => b.textContent?.trim() === text);
  if (!btn) throw new Error(`按钮「${text}」未找到`);
  btn.click();
}

beforeEach(() => {
  // refreshAll（ls/repo ls/env ls）与 WorkspaceDetail 的 st 都走 runGws：
  // 按 args[0] 分发夹具，其余命令（repo ls/env ls）返回空
  mocks.runGws.mockImplementation(async (args: string[]) => {
    if (args[0] === "ls") return { code: 0, output: LS_OUT };
    if (args[0] === "st") return { code: 0, output: ST_OUT };
    return { code: 0, output: "" };
  });
});

afterEach(() => {
  app?.unmount();
  app = null;
  el?.remove();
  el = null;
  mocks.handlers.clear();
  vi.clearAllMocks();
});

describe("WorkspacesTab 列表", () => {
  it("挂载即 refreshAll：渲染 gws ls 解析出的工作区行", async () => {
    mountTab();
    await vi.waitFor(() => expect(el!.querySelectorAll(".ws-row")).toHaveLength(2));
    expect(el!.textContent).toContain("demo");
    expect(el!.textContent).toContain("cart");
    expect(el!.textContent).toContain("结算改版");
    expect(mocks.runGws).toHaveBeenCalledWith(["ls"], "/hub");
  });

  it("空列表显示占位（暂无工作区）", async () => {
    mocks.runGws.mockImplementation(async () => ({ code: 0, output: "" }));
    mountTab();
    await vi.waitFor(() => expect(el!.textContent).toContain("(暂无工作区)"));
    expect(el!.querySelectorAll(".ws-row")).toHaveLength(0);
  });

  it("name 与 title 相同（缺省 title=name）只显示一次；不同则两者都显示", async () => {
    // 分支名不含工作区名，textContent 计数才不受分支列干扰
    const lsSameTitle =
      "名称              标题            阶段    模块    分支\n" +
      "feat-a            feat-a          dev     3       feature-1\n" +
      "feat-b            购物车改版      dev     2       feature-2\n";
    mocks.runGws.mockImplementation(async (args: string[]) => {
      if (args[0] === "ls") return { code: 0, output: lsSameTitle };
      return { code: 0, output: "" };
    });
    mountTab();
    await vi.waitFor(() => expect(el!.querySelectorAll(".ws-row")).toHaveLength(2));
    const rows = Array.from(el!.querySelectorAll(".ws-row"));
    const main = (i: number) => rows[i]!.querySelector(".ws-main")!.textContent ?? "";
    // feat-a：name===title → 该文本整行只出现一次（修复前 strong+muted 深浅重复两次），title span 不渲染
    expect((main(0).match(/feat-a/g) ?? []).length).toBe(1);
    expect(rows[0]!.querySelector(".ws-main .muted")).toBeNull();
    // feat-b：name!==title → name 与 title 都渲染
    expect((main(1).match(/feat-b/g) ?? []).length).toBe(1);
    expect(main(1)).toContain("购物车改版");
    expect(rows[1]!.querySelector(".ws-main .muted")).toBeTruthy();
  });
});

describe("WorkspacesTab 互斥详情（真跳转）", () => {
  /** 列表就绪后点第一行（demo）进入详情，并等 st 表格渲染 */
  async function openDetail() {
    mountTab();
    await vi.waitFor(() => expect(el!.querySelectorAll(".ws-row")).toHaveLength(2));
    (Array.from(el!.querySelectorAll<HTMLElement>(".ws-row"))[0]).click();
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["st"], "/hub/ws/demo"));
    await vi.waitFor(() => expect(el!.querySelector("table")).toBeTruthy());
  }

  it("点行进详情：列表与工具栏整体卸载（互斥），详情占满 tab 区域", async () => {
    await openDetail();
    expect(el!.textContent).toContain("demo");
    expect(el!.textContent).toContain("order-service"); // WorkspaceDetail 的 st 表格
    // 互斥断言：详情打开时列表/工具栏不可见
    expect(el!.querySelectorAll(".ws-row")).toHaveLength(0);
    expect(el!.querySelector(".toolbar")).toBeNull();
    expect(el!.textContent).not.toContain("+ 新建需求");
    expect(el!.textContent).not.toContain("(暂无工作区)");
  });

  it("详情 ← 返回：列表恢复、详情卸载（无数据残留）", async () => {
    await openDetail();
    clickButton("← 返回");
    await vi.waitFor(() => expect(el!.querySelectorAll(".ws-row")).toHaveLength(2));
    expect(el!.querySelector("table")).toBeNull();
    expect(el!.textContent).not.toContain("order-service");
    expect(el!.textContent).toContain("+ 新建需求"); // 工具栏恢复
    expect(el!.textContent).toContain("⇄ 导入需求"); // gws get 入口（用户反馈 #4 文案）
  });

  it("从详情返回后再点另一行（cart）：:key 变化重挂载、拉取新工作区的 st", async () => {
    await openDetail();
    clickButton("← 返回");
    await vi.waitFor(() => expect(el!.querySelectorAll(".ws-row")).toHaveLength(2));
    (Array.from(el!.querySelectorAll<HTMLElement>(".ws-row"))[1]).click();
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["st"], "/hub/ws/cart"));
  });
});
