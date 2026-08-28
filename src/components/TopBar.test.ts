// @vitest-environment happy-dom
// 手动 createApp().mount() 挂载 SFC；plugin-dialog open/hubExists mock，
// 切换行为（hub.path 变化触发 App.vue 的 MainView :key 重挂载）由 App 层保证
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createApp, nextTick, type App } from "vue";
import { createPinia, setActivePinia } from "pinia";

const mocks = vi.hoisted(() => ({
  open: vi.fn<() => Promise<string | string[] | null>>(),
  hubExists: vi.fn<(p: string) => Promise<boolean>>(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: mocks.open,
}));

vi.mock("../lib/gws-bridge", () => ({
  hubExists: mocks.hubExists,
}));

import { useHubStore } from "../stores/hub";
import TopBar from "./TopBar.vue";

let app: App | null = null;
let el: HTMLElement | null = null;

function mountTopBar(onOpenAbout?: () => void) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const hub = useHubStore();
  hub.setHub("/hub");
  el = document.createElement("div");
  document.body.appendChild(el);
  app = createApp(TopBar, { tab: "ws", onOpenAbout });
  app.use(pinia);
  app.mount(el);
}

/** 打开切换弹窗（v-if 渲染须等 nextTick） */
async function openSwitchDialog() {
  const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
    .find((b) => b.textContent?.trim() === "切换");
  if (!btn) throw new Error("「切换」按钮未找到");
  btn.click();
  await nextTick();
  const dialog = el!.querySelector(".dialog");
  if (!dialog) throw new Error("切换弹窗未打开");
  return dialog;
}

/** 弹窗内文本输入框填值 */
function setHubPath(value: string) {
  const input = el!.querySelector<HTMLInputElement>(".dialog input");
  if (!input) throw new Error("弹窗输入框未找到");
  input.value = value;
  input.dispatchEvent(new Event("input"));
}

function clickDialogButton(text: string) {
  const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>(".dialog button"))
    .find((b) => b.textContent?.trim() === text);
  if (!btn) throw new Error(`弹窗按钮「${text}」未找到`);
  btn.click();
}

beforeEach(() => {
  mocks.hubExists.mockResolvedValue(true);
});

afterEach(() => {
  app?.unmount();
  app = null;
  el?.remove();
  el = null;
  vi.clearAllMocks();
});

describe("TopBar 切换按钮", () => {
  it("顶栏只渲染「切换」二字（不再占位展示长路径）；hub 路径在 title 提示里保留", async () => {
    mountTopBar();
    const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
      .find((b) => b.textContent?.trim() === "切换")!;
    expect(btn).toBeTruthy();
    expect(btn.title).toBe("/hub");
    expect(el!.textContent).not.toContain("/hub ▾");
  });

  it("点击开弹窗：回显当前 hub 路径；tab 按钮不触发弹窗", async () => {
    mountTopBar();
    await openSwitchDialog();
    expect(el!.querySelector<HTMLInputElement>(".dialog input")!.value).toBe("/hub");

    const reposBtn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
      .find((b) => b.textContent?.trim() === "仓库")!;
    reposBtn.click();
    await nextTick();
    // tab 切换是 emit，不碰弹窗（弹窗仍开着）
    expect(el!.querySelector(".dialog")).toBeTruthy();
  });
});

describe("SwitchHubDialog", () => {
  it("切换到新 hub：校验通过 → setHub + lastHub 记忆 + 关窗（hub.path 变化令 MainView 重挂载）", async () => {
    mountTopBar();
    const { useSettingsStore } = await import("../stores/settings");
    const settings = useSettingsStore();
    const hub = useHubStore();
    await openSwitchDialog();

    setHubPath("/new/hub");
    await nextTick();
    clickDialogButton("打开 Hub");
    await vi.waitFor(() => expect(el!.querySelector(".dialog")).toBeNull());
    expect(hub.path).toBe("/new/hub");
    expect(settings.lastHub).toBe("/new/hub");
  });

  it("非法目录：hubExists false → 错误提示、不关窗、hub 不变", async () => {
    mountTopBar();
    const hub = useHubStore();
    mocks.hubExists.mockResolvedValue(false);
    await openSwitchDialog();

    setHubPath("/not/a/hub");
    await nextTick();
    clickDialogButton("打开 Hub");
    await vi.waitFor(() => expect(el!.textContent).toContain("该目录不是 gws hub"));
    expect(el!.querySelector(".dialog")).toBeTruthy();
    expect(hub.path).toBe("/hub");
  });

  it("浏览按钮：open 选中的目录回填输入框", async () => {
    mountTopBar();
    mocks.open.mockResolvedValue("/picked/dir");
    await openSwitchDialog();

    clickDialogButton("浏览…");
    await vi.waitFor(() => expect(el!.querySelector<HTMLInputElement>(".dialog input")!.value).toBe("/picked/dir"));
  });

  it("同路径 no-op：路径未改直接打开 → 关窗、不动 store（setHub 清列表但 MainView 不重挂载会留空页面）", async () => {
    mountTopBar();
    const hub = useHubStore();
    hub.workspaces = [{ name: "ws-a", title: "", stage: "dev", modules: 1, branch: "b" }];
    await openSwitchDialog();

    // 不改输入（回显即当前 hub）直接点打开
    clickDialogButton("打开 Hub");
    await vi.waitFor(() => expect(el!.querySelector(".dialog")).toBeNull());
    expect(hub.path).toBe("/hub");
    expect(mocks.hubExists).not.toHaveBeenCalled();
  });

  it("取消关窗：hub 不变", async () => {
    mountTopBar();
    const hub = useHubStore();
    await openSwitchDialog();

    setHubPath("/other");
    await nextTick();
    clickDialogButton("取消");
    await vi.waitFor(() => expect(el!.querySelector(".dialog")).toBeNull());
    expect(hub.path).toBe("/hub");
  });
});

describe("TopBar About 入口", () => {
  it("按钮文案「当前版本」：点击 emit open-about", async () => {
    const opened = vi.fn();
    mountTopBar(opened);
    const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
      .find((b) => b.textContent?.trim() === "当前版本");
    expect(btn).toBeTruthy();
    btn!.click();
    await new Promise((r) => setTimeout(r, 0));
    expect(opened).toHaveBeenCalledTimes(1);
  });
});
