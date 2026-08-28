// @vitest-environment happy-dom
// 无 @vue/test-utils：手动 createApp().mount() 挂载 SFC；gws-bridge mock 模式参考 SyncMainDialog.test.ts
import { afterEach, describe, expect, it, vi } from "vitest";
import { createApp, nextTick, type App } from "vue";
import { createPinia, setActivePinia } from "pinia";

const mocks = vi.hoisted(() => ({
  runGws: vi.fn<(args: string[], cwd: string) => Promise<{ code: number | null; output: string }>>(),
}));

vi.mock("../lib/gws-bridge", () => ({
  runGws: mocks.runGws,
}));

import { useHubStore } from "../stores/hub";
import MergeDialog from "./MergeDialog.vue";

let app: App | null = null;
let el: HTMLElement | null = null;

/** 挂载弹窗（hub store 提供 env ls 的 cwd）；close/run 经 props 探针收集 */
async function mountDialog(mode: "local" | "push" = "local") {
  const pinia = createPinia();
  setActivePinia(pinia);
  const hub = useHubStore();
  hub.setHub("/hub");
  const close = vi.fn();
  const run = vi.fn<(env: string) => void>();
  el = document.createElement("div");
  document.body.appendChild(el);
  app = createApp(MergeDialog, { mode, onClose: close, onRun: run });
  app.use(pinia);
  app.mount(el);
  await nextTick();
  return { close, run };
}

function clickButton(text: string) {
  const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
    .find((b) => b.textContent?.trim() === text);
  if (!btn) throw new Error(`按钮「${text}」未找到`);
  btn.click();
}

afterEach(() => {
  app?.unmount();
  app = null;
  el?.remove();
  el = null;
  vi.clearAllMocks();
});

const ENV_LS = "  环境分支\n  ○ dev (3 模块)\n  ● pre (3 模块)\n";

describe("MergeDialog", () => {
  it("挂载即实时取 env ls（cwd=hub.path，不用 hub.envs 快照），渲染环境选项", async () => {
    mocks.runGws.mockResolvedValue({ code: 0, output: ENV_LS });
    await mountDialog();
    expect(mocks.runGws).toHaveBeenCalledWith(["env", "ls"], "/hub");
    await vi.waitFor(() => {
      expect(el!.textContent).toContain("dev");
      expect(el!.textContent).toContain("pre");
    });
  });

  it("未选环境提交禁用；选中后点提交：run 带环境名、close 随即发出（弹窗即关）", async () => {
    mocks.runGws.mockResolvedValue({ code: 0, output: ENV_LS });
    const { close, run } = await mountDialog("push");
    await vi.waitFor(() => expect(el!.querySelectorAll(".envs button").length).toBe(2));
    const submit = () => Array.from(el!.querySelectorAll<HTMLButtonElement>(".actions button"))
      .find((b) => b.textContent?.trim() === "合并并推送")!;
    expect(submit().disabled).toBe(true);
    clickButton("pre");
    await nextTick();
    expect(submit().disabled).toBe(false);
    clickButton("合并并推送");
    expect(run).toHaveBeenCalledWith("pre");
    expect(close).toHaveBeenCalledTimes(1);
  });

  it("无环境：空态提示，不渲染选项", async () => {
    mocks.runGws.mockResolvedValue({ code: 1, output: "  环境分支\n  (暂无环境…)\n" });
    await mountDialog();
    await vi.waitFor(() => expect(el!.textContent).toContain("暂无环境"));
    expect(el!.querySelectorAll(".envs button")).toHaveLength(0);
  });

  it("env ls spawn 失败（code null）：展示错误与重试；重试成功恢复选项", async () => {
    mocks.runGws.mockResolvedValueOnce({ code: null, output: "启动失败: gws 未安装" });
    await mountDialog();
    await vi.waitFor(() => expect(el!.textContent).toContain("启动失败"));
    mocks.runGws.mockResolvedValueOnce({ code: 0, output: ENV_LS });
    clickButton("重试");
    await vi.waitFor(() => expect(el!.querySelectorAll(".envs button").length).toBe(2));
  });

  it("取消 / 点 mask：只发 close、不发 run", async () => {
    mocks.runGws.mockResolvedValue({ code: 0, output: ENV_LS });
    const { close, run } = await mountDialog();
    clickButton("取消");
    expect(close).toHaveBeenCalledTimes(1);
    (el!.querySelector(".mask") as HTMLElement).click(); // click.self：目标为 mask 自身
    expect(close).toHaveBeenCalledTimes(2);
    expect(run).not.toHaveBeenCalled();
  });
});
