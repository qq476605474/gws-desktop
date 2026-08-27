// @vitest-environment happy-dom
// 无 @vue/test-utils：手动 createApp().mount() 挂载 SFC；组件不依赖 store/IPC，无需 mock
import { afterEach, describe, expect, it, vi } from "vitest";
import { createApp, nextTick, type App } from "vue";
import SyncMainDialog from "./SyncMainDialog.vue";

let app: App | null = null;
let el: HTMLElement | null = null;

/** 挂载弹窗，close/run 经 props 探针收集 */
function mountDialog(props: { defaultFrom?: string } = {}) {
  const close = vi.fn();
  const run = vi.fn<(from: string) => void>();
  el = document.createElement("div");
  document.body.appendChild(el);
  app = createApp(SyncMainDialog, { ...props, onClose: close, onRun: run });
  app.mount(el);
  return { close, run };
}

function fromInput(): HTMLInputElement {
  const input = el!.querySelector<HTMLInputElement>("input");
  if (!input) throw new Error("from 输入框未找到");
  return input;
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
});

describe("SyncMainDialog", () => {
  it("渲染：标题、说明行、from 输入框（placeholder 写明留空=创建时基线）", () => {
    mountDialog();
    expect(el!.textContent).toContain("同步最新代码");
    expect(el!.textContent).toContain("把远程基线最新提交合进当前需求分支");
    expect(fromInput().placeholder).toContain("留空 = 创建时基线");
    expect(fromInput().value).toBe("");
  });

  it("defaultFrom 预填来源输入框", () => {
    mountDialog({ defaultFrom: "origin/dev" });
    expect(fromInput().value).toBe("origin/dev");
  });

  it("输入 from 后点开始同步：run 带输入值、close 随即发出（弹窗即关）", async () => {
    const { close, run } = mountDialog();
    fromInput().value = "release-2.0";
    fromInput().dispatchEvent(new Event("input"));
    await nextTick();
    clickButton("开始同步");
    expect(run).toHaveBeenCalledWith("release-2.0");
    expect(close).toHaveBeenCalledTimes(1);
  });

  // gws 的 --from 值支持逗号分隔链（resolve_chain_base 按优先级取第一个存在的远端 ref）：
  // GUI 归一空格/逗号输入，与 NewWorkspaceDialog 的基线来源输入一致
  it("多来源归一：空格/逗号分隔输入都拼成 a,b（顺序即优先级）", async () => {
    const { run } = mountDialog();
    fromInput().value = "阶段2 阶段1";
    fromInput().dispatchEvent(new Event("input"));
    await nextTick();
    clickButton("开始同步");
    expect(run).toHaveBeenCalledWith("阶段2,阶段1");
  });

  it("多来源归一：逗号分隔与混合分隔同效；纯空格视同留空", async () => {
    const { run } = mountDialog();
    fromInput().value = "阶段2, 阶段1";
    fromInput().dispatchEvent(new Event("input"));
    await nextTick();
    clickButton("开始同步");
    expect(run).toHaveBeenCalledWith("阶段2,阶段1");

    const again = mountDialog();
    again.run.mockClear();
    el!.querySelector<HTMLInputElement>("input")!.value = "   ";
    el!.querySelector<HTMLInputElement>("input")!.dispatchEvent(new Event("input"));
    await nextTick();
    clickButton("开始同步");
    expect(again.run).toHaveBeenCalledWith("");
  });

  it("留空直接开始同步：run 带空串（默认创建时基线）", () => {
    const { run } = mountDialog();
    clickButton("开始同步");
    expect(run).toHaveBeenCalledWith("");
  });

  it("取消 / 点 mask：只发 close、不发 run", () => {
    const { close, run } = mountDialog();
    clickButton("取消");
    expect(close).toHaveBeenCalledTimes(1);
    expect(run).not.toHaveBeenCalled();
    (el!.querySelector(".mask") as HTMLElement).click(); // click.self：目标为 mask 自身
    expect(close).toHaveBeenCalledTimes(2);
    expect(run).not.toHaveBeenCalled();
  });
});
