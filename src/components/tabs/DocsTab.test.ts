// @vitest-environment happy-dom
// 无 @vue/test-utils：手动 createApp().mount() 挂载 SFC；
// gws-bridge / tauri 事件与插件 mock 模式参考 src/components/tabs/ReposTab.test.ts
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
import DocsTab from "./DocsTab.vue";

let app: App | null = null;
let el: HTMLElement | null = null;
let nextId = 0;

const D = "\u001b[2m", N = "\u001b[0m", B = "\u001b[34m", G = "\u001b[32m";

/** gws doc ls 输出夹具：首行 docdir（带 ANSI 装饰），其后每行一个文档 [文件, pageId|null] */
function docLsOut(files: Array<[string, string | null]> = [["技术方案.md", "123"], ["排期.md", null]]): string {
  const head = B + "2026-08-18-checkout-revamp" + N;
  const rows = files.map(([file, pageId]) =>
    "  " + (pageId ? G + "●" + N : D + "○" + N) + " " + file + "  " +
    (pageId ? D + "wiki:" + pageId + N : D + "(未上传)" + N),
  );
  return [head, ...rows].join("\n");
}

/** 挂载 Tab（ws 列表直接写入 hub store，模拟 WorkspacesTab 已刷新过的状态） */
function mountTab(workspaces = [
  { name: "checkout-revamp", title: "结算流程改版", stage: "dev", modules: 3, branch: "feature-20260818-checkout-revamp" },
  { name: "login-crash", title: "login-crash", stage: "dev", modules: 1, branch: "hotfix-20260821-login-crash" },
]) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const hub = useHubStore();
  hub.setHub("/hub");
  hub.workspaces = workspaces;
  el = document.createElement("div");
  document.body.appendChild(el);
  app = createApp(DocsTab);
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

/** 选择工作区下拉并触发 change（v-model 赋值先于 @change 的 refresh 执行） */
function switchWs(name: string) {
  const sel = el!.querySelector<HTMLSelectElement>("select")!;
  if (!sel) throw new Error("工作区下拉未找到");
  sel.value = name;
  sel.dispatchEvent(new Event("change"));
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

describe("DocsTab.refresh", () => {
  it("默认选第一个工作区：doc ls 于 /hub/ws/<第一工作区>，docDir 剥 ANSI 解析、同步状态渲染", async () => {
    mocks.runGws.mockResolvedValue({ code: 0, output: docLsOut() });
    mountTab();

    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["doc", "ls"], "/hub/ws/checkout-revamp"));
    await vi.waitFor(() => expect(el!.querySelectorAll("tbody tr").length).toBe(2));
    expect(el!.textContent).toContain("● 已同步 (wiki:123)");
    expect(el!.textContent).toContain("○ 未上传");
    // docDir 取自首行（剥 ANSI）：路径列为 docs/<docdir>/<file>，无转义序列残留
    expect(el!.textContent).toContain("docs/2026-08-18-checkout-revamp/技术方案.md");
    expect(el!.textContent).not.toContain("\u001b");
    // select 首项显示当前工作区
    expect(el!.querySelector("select")!.textContent).toContain("当前: checkout-revamp");
  });

  it("wsFilter 切换：change 里 refresh 已读到新值（v-model 先生效），以新工作区为 cwd", async () => {
    mocks.runGws.mockResolvedValue({ code: 0, output: docLsOut() });
    mountTab();
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["doc", "ls"], "/hub/ws/checkout-revamp"));

    switchWs("login-crash");
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["doc", "ls"], "/hub/ws/login-crash"));
  });

  it("doc ls 非零退出：错误尾行进错误行（含重试），docs 保留旧数据、错误文本不进表格", async () => {
    mocks.runGws.mockResolvedValueOnce({ code: 0, output: docLsOut() }); // 挂载时首次 refresh 成功
    mocks.runGws.mockResolvedValue({ code: 1, output: "gws: 工作区不存在\ngws: 请先 gws get 拉取" });
    mountTab();
    await vi.waitFor(() => expect(el!.querySelectorAll("tbody tr").length).toBe(2));

    switchWs("login-crash"); // 触发第二次 refresh，doc ls 失败
    await vi.waitFor(() => expect(el!.textContent).toContain("gws: 请先 gws get 拉取"));
    expect(el!.textContent).toContain("重试");
    expect(el!.querySelectorAll("tbody tr").length).toBe(2); // docs 未被错误输出更新
    expect(el!.textContent).toContain("技术方案.md"); // 仍是旧数据
    expect(el!.textContent).not.toContain("工作区不存在"); // 错误只取尾行展示，首行不进任何位置
  });

  it("切换工作区失败：表格工作区列与行内 push cwd 均指向数据归属的旧工作区（docsWs 快照）", async () => {
    mocks.runGws.mockResolvedValueOnce({ code: 0, output: docLsOut() }); // checkout-revamp 成功
    mocks.runGws.mockResolvedValue({ code: 1, output: "gws: 工作区不存在" });
    mountTab();
    await vi.waitFor(() => expect(el!.querySelectorAll("tbody tr").length).toBe(2));

    switchWs("login-crash"); // doc ls 失败：docsWs/docs/docDir 不动
    await vi.waitFor(() => expect(el!.textContent).toContain("gws: 工作区不存在"));
    // 旧文档表保持自洽：工作区列仍是数据归属的 checkout-revamp，而非切换目标 login-crash
    expect(el!.querySelectorAll<HTMLTableRowElement>("tbody tr")[0]!.cells[1]!.textContent).toBe("checkout-revamp");

    // 行内"上传"的 cwd 用 docsWs（旧工作区）——旧文件不会 push 到新工作区
    clickButton("上传");
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(["doc", "push", "技术方案.md"], "/hub/ws/checkout-revamp", 30000),
    );
    await exitWith(1, 0);
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledTimes(3)); // 挂载 + 切换 + 命令后刷新
  });

  it("doc ls 以 exit 0 返回 hub 级假列表（首行 docs）：哨兵拦截，err 提示、docs 保留旧值", async () => {
    mocks.runGws.mockResolvedValueOnce({ code: 0, output: docLsOut() });
    // gws 上游怪癖（实证：.workspace.json 缺 docs 键等）：首行 basename 退化为 "docs"
    // + hub 级 README.md 行，exit 0——不设防会渲染成假表格
    mocks.runGws.mockResolvedValue({ code: 0, output: B + "docs" + N + "\n  " + G + "●" + N + " README.md  " + D + "wiki:999" + N });
    mountTab();
    await vi.waitFor(() => expect(el!.querySelectorAll("tbody tr").length).toBe(2));

    switchWs("login-crash");
    await vi.waitFor(() => expect(el!.textContent).toContain("当前目录不是有效的工作区"));
    expect(el!.querySelectorAll("tbody tr").length).toBe(2); // 假列表不进表格，旧数据保留
    expect(el!.textContent).toContain("技术方案.md");
    expect(el!.textContent).not.toContain("README.md");
    // docDir 未被假值 "docs" 污染：路径列仍是旧 docdir
    expect(el!.textContent).toContain("docs/2026-08-18-checkout-revamp/技术方案.md");
    expect(el!.textContent).toContain("重试");
  });

  it("doc ls 失败输出的错误尾行剥 ANSI 后展示（不渲染转义序列）", async () => {
    mocks.runGws.mockResolvedValue({ code: 1, output: D + "gws: 文档仓库不存在" + N + "\n" + D + "gws: 请先初始化" + N });
    mountTab();

    await vi.waitFor(() => expect(el!.textContent).toContain("gws: 请先初始化"));
    expect(el!.textContent).not.toContain("\u001b");
  });

  it("doc ls spawn 失败（code null）：错误信息直接展示", async () => {
    mocks.runGws.mockResolvedValue({ code: null, output: "启动失败: gws 未安装" });
    mountTab();

    await vi.waitFor(() => expect(el!.textContent).toContain("启动失败: gws 未安装"));
    expect(el!.querySelector("table")).toBeNull();
    expect(el!.textContent).toContain("重试");
  });

  it("runGws reject：err = String(e)，不崩、不死加载", async () => {
    mocks.runGws.mockRejectedValue(new Error("invoke 失败"));
    mountTab();

    await vi.waitFor(() => expect(el!.textContent).toContain("Error: invoke 失败"));
    expect(el!.querySelector("table")).toBeNull();
    expect(el!.textContent).toContain("重试");
  });

  it("无工作区 hub：ls 兜底后仍无目标，docs 安静置空不崩", async () => {
    mocks.runGws.mockResolvedValue({ code: 0, output: D + "(暂无工作区，用 gws new 创建)" + N });
    mountTab([]);

    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["ls"], "/hub"));
    await vi.waitFor(() => expect(el!.textContent).toContain("（暂无文档——在当前工作区 gws doc new 创建）"));
    expect(el!.querySelector("table")).toBeNull();
    expect(el!.querySelector(".error")).toBeNull();
    expect(mocks.runGws).toHaveBeenCalledTimes(1); // ls 兜底后无目标，不发 doc ls
  });

  it("ls 兜底非零退出：安静降级为无工作区空态（不报错、不发 doc ls）", async () => {
    mocks.runGws.mockResolvedValue({ code: 1, output: "gws: hub 结构损坏" });
    mountTab([]);

    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["ls"], "/hub"));
    await vi.waitFor(() => expect(el!.textContent).toContain("（暂无文档——在当前工作区 gws doc new 创建）"));
    expect(el!.querySelector(".error")).toBeNull(); // 不额外报错
    expect(mocks.runGws).toHaveBeenCalledTimes(1);
  });

  it("首次加载在途显示「加载中…」而非「暂无文档」", async () => {
    const pending: Array<{ resolve: (v: RunResult) => void }> = [];
    mocks.runGws.mockImplementation(() => new Promise<RunResult>((resolve) => pending.push({ resolve })));
    mountTab();

    await nextTick();
    expect(el!.textContent).not.toContain("暂无文档");
    expect(el!.textContent).toContain("加载中…");
    expect(el!.querySelector("table")).toBeNull();

    pending[0]!.resolve({ code: 0, output: docLsOut() });
    await vi.waitFor(() => expect(el!.querySelectorAll("tbody tr").length).toBe(2));
    expect(el!.textContent).not.toContain("加载中…");
  });

  it("hub.workspaces 为空：ls 兜底解析工作区后正常 doc ls 第一个", async () => {
    const lsOut =
      "名称                   标题                           阶段     模块   分支\n" +
      "checkout-revamp        结算流程改版                   dev      3      " + D + "feature-20260818-checkout-revamp" + N + "\n";
    mocks.runGws.mockImplementation(async (args: string[]) =>
      args[0] === "ls" ? { code: 0, output: lsOut } : { code: 0, output: docLsOut() },
    );
    mountTab([]);

    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["doc", "ls"], "/hub/ws/checkout-revamp"));
    await vi.waitFor(() => expect(el!.querySelectorAll("tbody tr").length).toBe(2));
  });

  it("并发守卫：切换工作区触发两次 refresh，旧响应迟到不覆盖最新结果", async () => {
    // 每次 runGws 调用挂起，由测试按序 resolve——模拟并发 refresh 的乱序返回
    const pending: Array<{ resolve: (v: RunResult) => void }> = [];
    mocks.runGws.mockImplementation(() => new Promise<RunResult>((resolve) => pending.push({ resolve })));
    mountTab(); // refresh #1（挂载，checkout-revamp）在途
    switchWs("login-crash"); // refresh #2 在途
    expect(mocks.runGws).toHaveBeenCalledTimes(2);

    // 旧请求（#1）迟到的成功响应必须被丢弃（n !== seq）
    pending[0]!.resolve({ code: 0, output: docLsOut([["技术方案.md", "123"]]) });
    await nextTick();
    expect(el!.querySelector("table")).toBeNull();
    expect(el!.textContent).not.toContain("技术方案.md");

    // 最新请求（#2）的结果被采纳
    pending[1]!.resolve({ code: 0, output: docLsOut([["排期.md", null]]) });
    await vi.waitFor(() => expect(el!.querySelector("table")).toBeTruthy());
    expect(el!.textContent).toContain("排期.md");
    expect(el!.textContent).not.toContain("技术方案.md");
  });
});

describe("DocsTab 命令操作", () => {
  it("create：doc new 于当前工作区，成功后清输入；exit 前 refresh 未发起（锁 waitDone→refresh 时序）", async () => {
    mocks.runGws.mockResolvedValue({ code: 0, output: docLsOut() });
    mountTab();
    await vi.waitFor(() => expect(el!.querySelector("table")).toBeTruthy());
    mocks.runGws.mockClear();

    setInput("技术方案v2.md");
    await nextTick();
    clickButton("+ 新建文档");
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(["doc", "new", "技术方案v2.md"], "/hub/ws/checkout-revamp", 30000),
    );
    // 负向断言：exit 事件未发出、命令未到终态前不得提前 refresh（runGws 已清计数）
    expect(mocks.runGws).not.toHaveBeenCalled();

    await exitWith(1, 0);
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["doc", "ls"], "/hub/ws/checkout-revamp"));
    await nextTick();
    expect(inputValue()).toBe("");
  });

  it("create：非零退出不清输入（前后空白以 trim 值传参），仍刷新", async () => {
    mocks.runGws.mockResolvedValue({ code: 0, output: docLsOut() });
    mountTab();
    await vi.waitFor(() => expect(el!.querySelector("table")).toBeTruthy());
    mocks.runGws.mockClear();

    setInput("  排期v2.md  ");
    await nextTick();
    clickButton("+ 新建文档");
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(["doc", "new", "排期v2.md"], "/hub/ws/checkout-revamp", 30000),
    );
    await exitWith(1, 1); // doc new 失败
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["doc", "ls"], "/hub/ws/checkout-revamp"));
    await nextTick();
    expect(inputValue()).toBe("  排期v2.md  "); // 失败保留原始输入便于重试
  });

  it("create：exec reject（IPC 失败）不崩，仍刷新，输入保留", async () => {
    mocks.runGws.mockResolvedValue({ code: 0, output: docLsOut() });
    mountTab();
    await vi.waitFor(() => expect(el!.querySelector("table")).toBeTruthy());
    mocks.runGws.mockClear();
    mocks.runGwsStream.mockRejectedValueOnce(new Error("invoke 失败"));

    setInput("技术方案v2.md");
    await nextTick();
    clickButton("+ 新建文档");
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["doc", "ls"], "/hub/ws/checkout-revamp"));
    await nextTick();
    expect(inputValue()).toBe("技术方案v2.md");
  });

  it("create：文件名含内嵌空格直接拒绝——err 提示、不 exec、输入保留", async () => {
    mocks.runGws.mockResolvedValue({ code: 0, output: docLsOut() });
    mountTab();
    await vi.waitFor(() => expect(el!.querySelector("table")).toBeTruthy());
    mocks.runGws.mockClear();

    setInput("my plan.md");
    await nextTick();
    clickButton("+ 新建文档");
    await new Promise((r) => setTimeout(r, 0));
    expect(el!.textContent).toContain("文档名不能包含空格");
    expect(mocks.runGwsStream).not.toHaveBeenCalled(); // 不发命令
    expect(mocks.runGws).not.toHaveBeenCalled(); // 也不触发刷新
    expect(inputValue()).toBe("my plan.md"); // 输入保留便于改名重试
  });

  it("create：双击守卫——首击在途（submitting）时同步第二击不重复 exec", async () => {
    mocks.runGws.mockResolvedValue({ code: 0, output: docLsOut() });
    mountTab();
    await vi.waitFor(() => expect(el!.querySelector("table")).toBeTruthy());
    mocks.runGws.mockClear();

    setInput("技术方案v2.md");
    await nextTick();
    clickButton("+ 新建文档");
    clickButton("+ 新建文档"); // Vue 未及重渲染禁用按钮，第二击只能靠 submitting 守卫拦截
    await vi.waitFor(() => expect(mocks.runGwsStream).toHaveBeenCalledTimes(1));

    await exitWith(1, 0);
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledTimes(1)); // 命令结束后恰一次 refresh
  });

  it("push：doc push <file> 于当前工作区，结束后刷新", async () => {
    mocks.runGws.mockResolvedValue({ code: 0, output: docLsOut() });
    mountTab();
    await vi.waitFor(() => expect(el!.querySelector("table")).toBeTruthy());
    mocks.runGws.mockClear();

    clickButton("上传"); // 首行 技术方案.md
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(["doc", "push", "技术方案.md"], "/hub/ws/checkout-revamp", 30000),
    );
    expect(mocks.runGws).not.toHaveBeenCalled(); // 终态前不刷新

    await exitWith(1, 0);
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["doc", "ls"], "/hub/ws/checkout-revamp"));
  });

  it("commit：工具栏「commit 全部文档」无文件参数，于数据归属工作区执行，结束后刷新", async () => {
    mocks.runGws.mockResolvedValue({ code: 0, output: docLsOut() });
    mountTab();
    await vi.waitFor(() => expect(el!.querySelector("table")).toBeTruthy());
    mocks.runGws.mockClear();

    clickButton("commit 全部文档");
    await vi.waitFor(() => expect(mocks.runGwsStream).toHaveBeenCalledWith(["doc", "commit"], "/hub/ws/checkout-revamp", 30000));
    expect(mocks.runGws).not.toHaveBeenCalled(); // 终态前不刷新

    await exitWith(1, 0);
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["doc", "ls"], "/hub/ws/checkout-revamp"));
  });

  it("push/commit：已有命令在跑（isRunning）时入口直接返回，不 exec 不刷新", async () => {
    mocks.runGws.mockResolvedValue({ code: 0, output: docLsOut() });
    mountTab();
    await vi.waitFor(() => expect(el!.querySelector("table")).toBeTruthy());
    mocks.runGws.mockClear();

    clickButton("上传"); // 启动 doc push（current.state=running）
    await vi.waitFor(() => expect(mocks.handlers.get("gws-exit:1")).toBeTruthy());
    // 此时 isRunning()=true；happy-dom 的 click() 不受 disabled 限制，仍会派发事件——
    // commit 只能靠函数入口守卫拦截
    clickButton("commit 全部文档");
    await new Promise((r) => setTimeout(r, 0));
    expect(mocks.runGwsStream).toHaveBeenCalledTimes(1); // 仅 doc push
    expect(mocks.runGws).not.toHaveBeenCalled(); // commit 被拦截，push 未终态，均未触发刷新

    await exitWith(1, 0);
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["doc", "ls"], "/hub/ws/checkout-revamp"));
  });
});
