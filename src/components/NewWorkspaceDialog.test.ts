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

/** 今日日期 YYYYMMDD（与组件 composedBranch 同格式，断言拼接结果用） */
function today(): string {
  const d = new Date();
  return `${d.getFullYear()}${String(d.getMonth() + 1).padStart(2, "0")}${String(d.getDate()).padStart(2, "0")}`;
}

/** 挂载弹窗（预置两个候选模块：模块必选后，创建用例须先勾选才能点创建） */
function mountDialog() {
  const pinia = createPinia();
  setActivePinia(pinia);
  const hub = useHubStore();
  hub.setHub("/hub");
  hub.repos = [
    { name: "order-service", mainBranch: "main" },
    { name: "user-web", mainBranch: "main" },
  ];
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

/** 按 placeholder 片段定位文本输入框（字段增删/换序不漂移；单选 radio 无 placeholder 不受扰） */
function inputByPlaceholder(fragment: string): HTMLInputElement {
  const input = Array.from(el!.querySelectorAll<HTMLInputElement>("input"))
    .find((i) => i.placeholder.includes(fragment));
  if (!input) throw new Error(`输入框（placeholder 含「${fragment}」）未找到`);
  return input;
}

function setName(v: string) { const i = inputByPlaceholder("目录名"); i.value = v; i.dispatchEvent(new Event("input")); }
function setPrefix(v: string) { const i = inputByPlaceholder("英文前缀"); i.value = v; i.dispatchEvent(new Event("input")); }
function setSuffix(v: string) { const i = inputByPlaceholder("英文后缀"); i.value = v; i.dispatchEvent(new Event("input")); }
function setFullBranch(v: string) { const i = inputByPlaceholder("如 feature-20260828"); i.value = v; i.dispatchEvent(new Event("input")); }
function setTitle(v: string) { const i = inputByPlaceholder("中文标题"); i.value = v; i.dispatchEvent(new Event("input")); }

/** 切分支填法（单选 change 派发，模式同 checkbox 勾选）：compose=前缀组合 / full=完整分支名 */
async function switchMode(mode: "compose" | "full") {
  const radio = Array.from(el!.querySelectorAll<HTMLInputElement>('input[type="radio"]'))
    .find((r) => r.value === mode)!;
  radio.checked = true;
  radio.dispatchEvent(new Event("change"));
  await nextTick();
}

/** 勾选模块复选框（checkbox v-model 监听 change 事件）；两次勾选之间须等 patch：
 *  数组型 checkbox 的 change 处理器基于旧数组计算，同步连发会让后一次覆盖前一次的勾选
 *  （模式同 AddModuleDialog.test.ts 的 checkAll） */
async function checkModule(name: string) {
  const box = Array.from(el!.querySelectorAll<HTMLInputElement>('input[type="checkbox"]'))
    .find((i) => i.value === name);
  if (!box) throw new Error(`模块「${name}」复选框未找到`);
  box.checked = true;
  box.dispatchEvent(new Event("change"));
  await nextTick();
}

/** 基线来源输入框填值（按 placeholder 定位，与序号解耦） */
function setFrom(value: string) {
  const input = inputByPlaceholder("留空=主干");
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
  it("名称 placeholder 写明必填且可中文（目录名，不再有留空反推分支名的旧语义）", () => {
    mountDialog();
    const placeholder = inputByPlaceholder("目录名").placeholder;
    expect(placeholder).toContain("必填");
    expect(placeholder).toContain("可中文");
  });

  it("创建按钮在名称为空或未勾选模块时禁用（名称与模块均必填，模块不再有「不选=全部仓库」语义）", async () => {
    mountDialog();
    const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
      .find((b) => b.textContent?.trim() === "创建")!;
    expect(btn.disabled).toBe(true); // 名称空 + 未选模块（组合模式前缀已有默认值）
    setName("demo");
    await nextTick();
    expect(btn.disabled).toBe(true); // 模块必选：仅填名称仍禁用
    await checkModule("order-service");
    await vi.waitFor(() => expect(btn.disabled).toBe(false));
    // 完整分支名模式：分支为空禁用（该模式下分支是必填项）
    await switchMode("full");
    await vi.waitFor(() => expect(btn.disabled).toBe(true));
    setFullBranch("feature-20260828-demo");
    await vi.waitFor(() => expect(btn.disabled).toBe(false));
  });

  it("组合模式前缀清空禁用创建（空前缀拼出的分支以 - 开头不合法）", async () => {
    mountDialog();
    setName("demo");
    await checkModule("order-service");
    await vi.waitFor(() => {
      const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
        .find((b) => b.textContent?.trim() === "创建")!;
      return expect(btn.disabled).toBe(false);
    });
    setPrefix("");
    await nextTick();
    const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
      .find((b) => b.textContent?.trim() === "创建")!;
    expect(btn.disabled).toBe(true);
  });

  it("命令成功（exit 0）：created 先于 close 通知（父组件先刷新列表再卸载弹窗）", async () => {
    mountDialog();
    setName("demo");
    await checkModule("order-service");
    await clickCreate();
    // 组合模式默认：feature-<今日>-demo（分支名留空退回名称作后缀）
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(
        ["new", "demo", "--modules", "order-service", "--branch", `feature-${today()}-demo`],
        "/hub",
        30000,
      ),
    );
    await exitWith(1, 0);
    await vi.waitFor(() => expect(events).toEqual(["created", "close"]));
  });

  it("命令失败（exit 1）：不 emit created/close，弹窗保留输入便于重试", async () => {
    mountDialog();
    setName("demo");
    await checkModule("order-service");
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
    setName("demo");
    await checkModule("order-service");
    await clickCreate();
    await vi.waitFor(() => expect(el!.textContent).toContain("gws 未安装"));
    expect(events).toEqual([]);
    expect(el!.querySelector(".dialog")).toBeTruthy();
  });

  it("submitting 期间点 mask/取消不关窗（防 IPC 间隙卸载致 created 通知丢失）", async () => {
    mountDialog();
    setName("demo");
    await checkModule("order-service");
    await clickCreate();
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

describe("NewWorkspaceDialog 分支两种填法（用户反馈 #12：拆开填写、提交拼接、互斥）", () => {
  it("组合模式：预览实时拼接 <前缀>-<日期>-<分支名>，分支名留空退回名称", async () => {
    mountDialog();
    setName("收银台改版");
    await nextTick();
    expect(el!.textContent).toContain(`分支：feature-${today()}-收银台改版`); // 后缀留空用名称
    setPrefix("hotfix");
    setSuffix("checkout-revamp");
    await nextTick();
    expect(el!.textContent).toContain(`分支：hotfix-${today()}-checkout-revamp`);
  });

  it("组合模式提交：目录名中文 + 英文后缀并存，--branch 传完整拼接值（名称与分支独立）", async () => {
    mountDialog();
    setName("收银台改版");
    setPrefix("hotfix");
    setSuffix("checkout-revamp");
    await checkModule("order-service");
    await clickCreate();
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(
        ["new", "收银台改版", "--modules", "order-service", "--branch", `hotfix-${today()}-checkout-revamp`],
        "/hub",
        30000,
      ),
    );
  });

  it("完整分支名模式：原样传 --branch，不拼接", async () => {
    mountDialog();
    setName("demo");
    await switchMode("full");
    setFullBranch("feature-20260828-checkout-revamp");
    await checkModule("order-service");
    await clickCreate();
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(
        ["new", "demo", "--modules", "order-service", "--branch", "feature-20260828-checkout-revamp"],
        "/hub",
        30000,
      ),
    );
  });

  it("两模式互斥：同一时刻只渲染一组的输入（组合=前缀+分支名，完整=整支输入）", async () => {
    mountDialog();
    // 默认组合模式：前缀/分支名在，完整分支名输入不在
    expect(() => inputByPlaceholder("英文前缀")).not.toThrow();
    expect(() => inputByPlaceholder("英文后缀")).not.toThrow();
    expect(() => inputByPlaceholder("如 feature-20260828")).toThrow();
    await switchMode("full");
    expect(() => inputByPlaceholder("如 feature-20260828")).not.toThrow();
    expect(() => inputByPlaceholder("英文前缀")).toThrow();
    expect(() => inputByPlaceholder("英文后缀")).toThrow();
    // 切回组合模式：字段回到视野（各模式输入值保留，不因切换丢失）
    await switchMode("compose");
    expect(() => inputByPlaceholder("英文前缀")).not.toThrow();
  });

  it("前缀建议 datalist：覆盖市面常用前缀（feature 默认 + bugfix/hotfix/release/support/docs/refactor/test/chore）", () => {
    mountDialog();
    const options = Array.from(el!.querySelectorAll("#prefix-list option"));
    expect(options.map((o) => o.getAttribute("value"))).toEqual([
      "feature", "bugfix", "hotfix", "release", "support", "docs", "refactor", "test", "chore",
    ]);
    // 前缀输入框是自由文本（datalist 仅建议）：非列表值可直接填
    setPrefix("release2");
    expect(inputByPlaceholder("英文前缀").value).toBe("release2");
  });

  it("参数拼装：标题/基线按需附加，--branch 恒传（组合模式拼接值）", async () => {
    mountDialog();
    setName("demo");
    setTitle("结算改版");
    setFrom("需求A 阶段2");
    await checkModule("order-service");
    await checkModule("user-web"); // join 顺序即勾选顺序
    await clickCreate();
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(
        ["new", "demo", "--modules", "order-service,user-web", "--from", "需求A,阶段2", "--title", "结算改版", "--branch", `feature-${today()}-demo`],
        "/hub",
        30000,
      ),
    );
  });
});

describe("NewWorkspaceDialog 基线来源（--from）", () => {
  it("留空不传：args 无 --from（gws 默认走创建时基线，主干兜底）", async () => {
    mountDialog();
    setName("demo");
    await checkModule("order-service");
    await clickCreate();
    // 精确数组断言：--from 不出现（--from 仅在有输入时附加）
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(
        ["new", "demo", "--modules", "order-service", "--branch", `feature-${today()}-demo`],
        "/hub",
        30000,
      ),
    );
  });

  it("多值解析：空格与逗号分隔均归一为逗号 join（顺序即优先级），位置在 --modules 之后", async () => {
    mountDialog();
    setName("demo");
    await checkModule("order-service");
    setFrom("需求A 阶段2"); // 空格分隔两个基线
    await clickCreate();
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(
        ["new", "demo", "--modules", "order-service", "--from", "需求A,阶段2", "--branch", `feature-${today()}-demo`],
        "/hub",
        30000,
      ),
    );

    // 逗号分隔输入归一结果相同：先等首条命令终态（submitting 归零）再发起第二次创建
    await exitWith(1, 0);
    setFrom("需求A,阶段2");
    await clickCreate();
    await vi.waitFor(() =>
      expect(mocks.runGwsStream).toHaveBeenCalledWith(
        ["new", "demo", "--modules", "order-service", "--from", "需求A,阶段2", "--branch", `feature-${today()}-demo`],
        "/hub",
        30000,
      ),
    );
  });
});
