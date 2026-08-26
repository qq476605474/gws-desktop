// @vitest-environment happy-dom
// 无 @vue/test-utils：手动 createApp().mount() 挂载 SFC；
// gws-bridge / tauri 事件 mock 模式参考 src/components/AddModuleDialog.test.ts
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
  latestGwsVersion: vi.fn<() => Promise<string>>(),
  /** 按事件名保存 listen 注册的 handler，测试中手动触发以模拟 gws-exit 事件 */
  handlers: new Map<string, Handler>(),
}));

// 组件链上 hub/cmd store 均从该模块导入，mock 须补齐全部具名导出，否则导入链接缺绑定
vi.mock("../lib/gws-bridge", () => ({
  runGws: mocks.runGws,
  runGwsStream: mocks.runGwsStream,
  respondConfirm: mocks.respondConfirm,
  replayOutput: mocks.replayOutput,
  latestGwsVersion: mocks.latestGwsVersion,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: Handler) => {
    mocks.handlers.set(event, handler);
    return () => mocks.handlers.delete(event);
  }),
}));

import { useCmdStore } from "../stores/cmd";
import { useHubStore } from "../stores/hub";
import AboutDialog from "./AboutDialog.vue";

let app: App | null = null;
let el: HTMLElement | null = null;
let nextId = 0;

/** 挂载弹窗（hub.path=/hub），close 事件经 props 探针收集 */
function mountDialog(onClose: () => void) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const hub = useHubStore();
  hub.setHub("/hub");
  el = document.createElement("div");
  document.body.appendChild(el);
  app = createApp(AboutDialog, { onClose });
  app.use(pinia);
  app.mount(el);
}

function button(text: string): HTMLButtonElement {
  const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
    .find((b) => b.textContent?.trim() === text);
  if (!btn) throw new Error(`按钮「${text}」未找到`);
  return btn;
}

function clickButton(text: string) {
  button(text).click();
}

function clickMask() {
  el!.querySelector<HTMLElement>(".mask")!.click();
}

/** 等待第 runId 次 exec 完成 listen 订阅后发出退出码（runId 从 1 递增） */
async function exitWith(runId: number, code: number) {
  await vi.waitFor(() => expect(mocks.handlers.get(`gws-exit:${runId}`)).toBeTruthy());
  mocks.handlers.get(`gws-exit:${runId}`)!({ payload: { code } });
}

beforeEach(() => {
  nextId = 0;
  mocks.runGwsStream.mockImplementation(async () => ++nextId);
  mocks.runGws.mockResolvedValue({ code: 0, output: "gws 0.4.2" });
  mocks.replayOutput.mockResolvedValue(undefined);
  mocks.latestGwsVersion.mockResolvedValue("0.5.0");
});

afterEach(() => {
  app?.unmount();
  app = null;
  el?.remove();
  el = null;
  mocks.handlers.clear();
  vi.clearAllMocks();
});

describe("AboutDialog 挂载加载当前版本", () => {
  it("onMounted：gws version 于 hub.path，解析输出显示版本号（在途占位「未知」不显示垃圾值）", async () => {
    const pending: Array<{ resolve: (v: RunResult) => void }> = [];
    mocks.runGws.mockImplementation(() => new Promise<RunResult>((resolve) => pending.push({ resolve })));
    mountDialog(vi.fn());

    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledWith(["version"], "/hub"));
    await nextTick();
    expect(el!.textContent).toContain("未知"); // 在途占位，而非空白或错误解析值
    expect(el!.textContent).not.toContain("0.4.2");

    pending[0]!.resolve({ code: 0, output: "gws 0.4.2" });
    await vi.waitFor(() => expect(el!.textContent).toContain("0.4.2"));
    expect(el!.textContent).not.toContain("未知");
    expect(el!.querySelector(".error")).toBeNull();
    expect(mocks.runGws).toHaveBeenCalledTimes(1);
    // 未检查过更新：远端行与更新按钮均不出现
    expect(el!.textContent).not.toContain("远端最新");
    expect(el!.textContent).not.toContain("更新到");
  });

  it("version 非零退出：错误尾行（剥 ANSI）进 err、版本未知；查到 latest 也不出现更新按钮（current 为空）", async () => {
    const D = "\u001b[2m", N = "\u001b[0m";
    mocks.runGws.mockResolvedValue({ code: 1, output: D + "gws: 版本命令失败" + N + "\n" + D + "gws: 请检查安装" + N });
    mountDialog(vi.fn());

    await vi.waitFor(() => expect(el!.textContent).toContain("请检查安装"));
    expect(el!.querySelector(".error")).toBeTruthy();
    expect(el!.textContent).not.toContain("\u001b"); // 剥 ANSI 后展示
    expect(el!.textContent).not.toContain("版本命令失败"); // 只取尾行
    expect(el!.textContent).toContain("未知");

    // current 为空：即使查到远端最新也无从比较，不出现更新按钮
    clickButton("检查更新");
    await vi.waitFor(() => expect(el!.textContent).toContain("远端最新"));
    expect(el!.textContent).toContain("0.5.0");
    expect(el!.textContent).not.toContain("更新到");
  });

  it("version spawn 失败（code null）：错误信息展示、版本未知", async () => {
    mocks.runGws.mockResolvedValue({ code: null, output: "启动失败: gws 未安装" });
    mountDialog(vi.fn());

    await vi.waitFor(() => expect(el!.textContent).toContain("启动失败: gws 未安装"));
    expect(el!.textContent).toContain("未知");
    expect(el!.querySelector(".error")).toBeTruthy();
  });

  it("runGws reject：err = String(e)，不崩、版本未知", async () => {
    mocks.runGws.mockRejectedValue(new Error("invoke 失败"));
    mountDialog(vi.fn());

    await vi.waitFor(() => expect(el!.textContent).toContain("Error: invoke 失败"));
    expect(el!.textContent).toContain("未知");
  });
});

describe("AboutDialog 检查更新", () => {
  it("latest 与 current 不同：显示远端最新与「更新到」按钮，无「已是最新」", async () => {
    mountDialog(vi.fn());
    await vi.waitFor(() => expect(el!.textContent).toContain("0.4.2"));

    clickButton("检查更新");
    await vi.waitFor(() => expect(el!.textContent).toContain("远端最新"));
    expect(el!.textContent).toContain("0.5.0");
    expect(button("更新到 0.5.0")).toBeTruthy();
    expect(el!.textContent).not.toContain("已是最新");
  });

  it("latest 与 current 相同：显示「已是最新版本」，无更新按钮", async () => {
    mocks.latestGwsVersion.mockResolvedValue("0.4.2");
    mountDialog(vi.fn());
    await vi.waitFor(() => expect(el!.textContent).toContain("0.4.2"));

    clickButton("检查更新");
    await vi.waitFor(() => expect(el!.textContent).toContain("已是最新版本"));
    expect(el!.textContent).not.toContain("更新到");
  });

  it("latest 解析为空串：显示「无法获取」，无更新按钮（防御分支：Rust Result 失败走 reject 不会给空串）", async () => {
    mocks.latestGwsVersion.mockResolvedValue("");
    mountDialog(vi.fn());
    await vi.waitFor(() => expect(el!.textContent).toContain("0.4.2"));

    clickButton("检查更新");
    await vi.waitFor(() => expect(el!.textContent).toContain("无法获取"));
    expect(el!.textContent).not.toContain("更新到");
  });

  it("latestGwsVersion reject：err 提示（真实桥远端不可达即此形态），远端行不出现", async () => {
    mocks.latestGwsVersion.mockRejectedValue("获取最新版本失败: curl 退出码 7");
    mountDialog(vi.fn());
    await vi.waitFor(() => expect(el!.textContent).toContain("0.4.2"));

    clickButton("检查更新");
    await vi.waitFor(() =>
      expect(el!.textContent).toContain("检查更新失败: 获取最新版本失败: curl 退出码 7"),
    );
    expect(el!.textContent).not.toContain("远端最新"); // latest 复位 null，行隐藏
    expect(el!.textContent).not.toContain("更新到");
  });

  it("检查在途：按钮「检查中…」且禁用，关闭仍可用（检查无副作用）；完成后恢复", async () => {
    let release!: (v: string) => void;
    mocks.latestGwsVersion.mockImplementation(() => new Promise<string>((r) => (release = r)));
    mountDialog(vi.fn());
    await vi.waitFor(() => expect(el!.textContent).toContain("0.4.2"));

    clickButton("检查更新");
    await vi.waitFor(() => expect(button("检查中…")).toBeTruthy());
    expect(button("检查中…").disabled).toBe(true);
    expect(button("关闭").disabled).toBe(false); // 检查在途允许关闭

    release("0.5.0");
    await vi.waitFor(() => expect(el!.textContent).toContain("0.5.0"));
    expect(button("检查更新").disabled).toBe(false);
  });

  it("latest 复位序列：首查成功出更新按钮后重查失败 → err 提示，远端行与「更新到」按钮消失", async () => {
    mocks.latestGwsVersion
      .mockResolvedValueOnce("0.5.0")
      .mockRejectedValueOnce("获取最新版本失败: curl 退出码 7");
    mountDialog(vi.fn());
    await vi.waitFor(() => expect(el!.textContent).toContain("0.4.2"));

    clickButton("检查更新");
    await vi.waitFor(() => expect(button("更新到 0.5.0")).toBeTruthy()); // 首查成功：更新按钮出现

    clickButton("检查更新");
    await vi.waitFor(() =>
      expect(el!.textContent).toContain("检查更新失败: 获取最新版本失败: curl 退出码 7"),
    );
    expect(el!.textContent).not.toContain("远端最新"); // latest 复位 null，远端行隐藏
    expect(el!.textContent).not.toContain("更新到"); // 更新按钮随之消失
    expect(el!.querySelector(".error")).toBeTruthy();
  });

  it("check 双击守卫：同步第二击被入口守卫拦截，latestGwsVersion 只调一次", async () => {
    mountDialog(vi.fn());
    await vi.waitFor(() => expect(el!.textContent).toContain("0.4.2"));

    // 两击同任务同步派发，落在 Vue patch 滞后窗口内（disabled 尚未落到 DOM），第二击
    // 须靠 check 入口守卫的 checking 分支拦截——与 update 双击守卫对称
    clickButton("检查更新");
    clickButton("检查更新");
    await new Promise((r) => setTimeout(r, 0));
    expect(mocks.latestGwsVersion).toHaveBeenCalledTimes(1);
    await vi.waitFor(() => expect(el!.textContent).toContain("远端最新")); // 首次检查正常完成
  });
});

describe("AboutDialog 一键更新", () => {
  it("update：args 于 hub.path，exit 前 version 未被重查（waitDone 锁时序）；在途禁关闭/禁点 mask；完成后刷新版本并显示已是最新", async () => {
    mocks.runGws
      .mockResolvedValueOnce({ code: 0, output: "gws 0.4.2" }) // 挂载时当前版本
      .mockResolvedValueOnce({ code: 0, output: "gws 0.5.0" }); // 更新结束后重查
    const closed = vi.fn();
    mountDialog(closed);
    await vi.waitFor(() => expect(el!.textContent).toContain("0.4.2"));

    clickButton("检查更新");
    await vi.waitFor(() => expect(button("更新到 0.5.0")).toBeTruthy());

    clickButton("更新到 0.5.0");
    await vi.waitFor(() => expect(mocks.runGwsStream).toHaveBeenCalledWith(["update"], "/hub"));
    // 宏任务 flush：exec 链（runGwsStream→listen→replayOutput）全是已决微任务，此刻已走完、
    // 而 exit 事件尚未发出——若无 waitDone，loadCurrent 已重查 version（第 2 次 runGws）
    await new Promise((r) => setTimeout(r, 0));
    expect(mocks.runGws).toHaveBeenCalledTimes(1);

    // 更新在途 disabled 矩阵：更新/关闭/检查均禁用，点 mask 不 emit close
    await vi.waitFor(() => expect(button("更新中…")).toBeTruthy());
    expect(button("更新中…").disabled).toBe(true);
    expect(button("关闭").disabled).toBe(true);
    expect(button("检查更新").disabled).toBe(true);
    clickMask();
    expect(closed).not.toHaveBeenCalled();

    await exitWith(1, 0);
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenNthCalledWith(2, ["version"], "/hub"));
    await vi.waitFor(() => expect(el!.textContent).toContain("已是最新版本")); // current 已刷成 0.5.0 与 latest 相等
    expect(el!.textContent).not.toContain("更新到"); // 更新按钮消失
    expect(button("关闭").disabled).toBe(false); // updating 结束恢复可关闭
    expect(closed).not.toHaveBeenCalled();
  });

  it("exec reject（IPC 失败）：catch 不崩、err 提示、updating 复位可关闭可重试，不重查版本", async () => {
    mocks.runGwsStream.mockRejectedValueOnce(new Error("invoke 失败"));
    const closed = vi.fn();
    mountDialog(closed);
    await vi.waitFor(() => expect(el!.textContent).toContain("0.4.2"));

    clickButton("检查更新");
    await vi.waitFor(() => expect(button("更新到 0.5.0")).toBeTruthy());
    clickButton("更新到 0.5.0");

    await vi.waitFor(() => expect(el!.textContent).toContain("Error: invoke 失败"));
    expect(mocks.runGws).toHaveBeenCalledTimes(1); // throw 跳过 loadCurrent，未重查版本
    expect(button("关闭").disabled).toBe(false); // finally 复位 updating
    expect(button("更新到 0.5.0").disabled).toBe(false); // 按钮恢复可重试
    expect(closed).not.toHaveBeenCalled();
  });

  it("update 非零退出：err 提示不静默、版本重查保持旧值、「更新到」按钮复现可重试", async () => {
    mocks.runGws
      .mockResolvedValueOnce({ code: 0, output: "gws 0.4.2" }) // 挂载时当前版本
      .mockResolvedValueOnce({ code: 0, output: "gws 0.4.2" }); // 更新失败后重查仍是旧版本
    const closed = vi.fn();
    mountDialog(closed);
    await vi.waitFor(() => expect(el!.textContent).toContain("0.4.2"));

    clickButton("检查更新");
    await vi.waitFor(() => expect(button("更新到 0.5.0")).toBeTruthy());
    clickButton("更新到 0.5.0");
    await exitWith(1, 1); // 非零退出：state → failed

    await vi.waitFor(() => expect(el!.textContent).toContain("gws update 未成功（详见输出面板）"));
    expect(el!.querySelector(".error")).toBeTruthy();
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenNthCalledWith(2, ["version"], "/hub")); // 仍重查版本
    expect(el!.textContent).toContain("0.4.2"); // current 保持旧版本
    expect(el!.textContent).not.toContain("已是最新版本");
    expect(button("更新到 0.5.0").disabled).toBe(false); // 更新按钮复现，可重试
    expect(button("关闭").disabled).toBe(false); // updating 已复位
    expect(closed).not.toHaveBeenCalled();
  });

  it("双击守卫：首击在途（updating）时同步第二击不重复 exec", async () => {
    const closed = vi.fn();
    mountDialog(closed);
    await vi.waitFor(() => expect(el!.textContent).toContain("0.4.2"));
    clickButton("检查更新");
    await vi.waitFor(() => expect(button("更新到 0.5.0")).toBeTruthy());

    clickButton("更新到 0.5.0");
    // 两击同任务同步派发，落在 Vue patch 滞后窗口内——disabled 尚未落到 DOM、第二击真正
    // 派发，只能靠入口守卫 updating 拦截（首击已同步置 updating=true）
    clickButton("更新到 0.5.0");
    await vi.waitFor(() => expect(mocks.runGwsStream).toHaveBeenCalledTimes(1));
    expect(mocks.runGwsStream).toHaveBeenCalledWith(["update"], "/hub");

    await exitWith(1, 0);
    await vi.waitFor(() => expect(mocks.runGws).toHaveBeenCalledTimes(2)); // 恰一次更新后重查
  });

  it("isRunning（外部命令在跑）：检查/更新按钮禁用；同步点击落在 patch 滞后窗口内被入口守卫拦截，关闭不受影响", async () => {
    const closed = vi.fn();
    mountDialog(closed);
    await vi.waitFor(() => expect(el!.textContent).toContain("0.4.2"));
    clickButton("检查更新");
    await vi.waitFor(() => expect(button("更新到 0.5.0")).toBeTruthy());

    // 直接写入 cmd store 模拟外部命令在跑（updating/checking 均为 false，隔离 isRunning 项）
    const cmd = useCmdStore();
    cmd.current = { id: 99, label: "gws sync", output: "", state: "running", code: null };

    // 同步 click 落在 Vue 重渲染前的窗口内：disabled 尚未落到 DOM，click 真正派发（happy-dom
    // 对已 disabled 按钮的 click() 不派发事件，await nextTick 后再点是空转断言）；此时 store
    // 已同步更新（isRunning=true），更新是否发出只能靠 update 入口守卫兜底
    button("更新到 0.5.0").click(); // 不 await nextTick：disabled 未落到 DOM，click 真正派发
    await new Promise((r) => setTimeout(r, 0));
    expect(mocks.runGwsStream).not.toHaveBeenCalled();

    expect(button("检查更新").disabled).toBe(true);
    expect(button("更新到 0.5.0").disabled).toBe(true);
    expect(button("关闭").disabled).toBe(false); // 关闭仅受 updating 约束

    cmd.current = null;
    await nextTick();
    expect(button("检查更新").disabled).toBe(false);
    expect(button("更新到 0.5.0").disabled).toBe(false);
  });
});

describe("AboutDialog 关闭行为", () => {
  it("空闲：点 mask/关闭按钮 emit close；点 dialog 本体不关闭（click.self）", async () => {
    const closed = vi.fn();
    mountDialog(closed);
    await vi.waitFor(() => expect(el!.textContent).toContain("0.4.2"));

    el!.querySelector<HTMLElement>(".dialog")!.click(); // 冒泡到 mask 但 target 非本体
    expect(closed).not.toHaveBeenCalled();

    clickMask();
    expect(closed).toHaveBeenCalledTimes(1);

    clickButton("关闭");
    expect(closed).toHaveBeenCalledTimes(2);
  });
});
