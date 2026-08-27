// @vitest-environment happy-dom
// 无 @vue/test-utils：手动 createApp().mount() 挂载 SFC（模式参考 AboutDialog.test.ts）；
// plugin-store mock 模式参考 src/stores/settings.test.ts
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createApp, nextTick, type App } from "vue";
import { createPinia, setActivePinia } from "pinia";

const mocks = vi.hoisted(() => {
  const values = new Map<string, unknown>();
  const get = vi.fn(async (key: string) => values.get(key));
  const set = vi.fn(async (_key: string, _value: unknown) => {});
  const terminalOptions = vi.fn<() => Promise<{ id: string; label: string }[]>>();
  return { values, get, set, load: vi.fn(async () => ({ get, set })), terminalOptions };
});

vi.mock("@tauri-apps/plugin-store", () => ({
  load: mocks.load,
  Store: class Store {},
}));

vi.mock("../lib/gws-bridge", () => ({
  terminalOptions: mocks.terminalOptions,
}));

import { useSettingsStore } from "../stores/settings";
import SettingsDialog from "./SettingsDialog.vue";

let app: App | null = null;
let el: HTMLElement | null = null;

/** 模拟 Rust terminal_options 的返回（macOS 场景：system + 已装 iTerm2 的典型集） */
function mockTerms() {
  mocks.terminalOptions.mockResolvedValue([
    { id: "system", label: "跟随系统（当前 iTerm2）" },
    { id: "iTerm2", label: "iTerm2" },
    { id: "Terminal.app", label: "Terminal.app" },
  ]);
}

/** 挂载设置弹窗：先 init settings store（注册持久化 watch + applyTheme），close 经 props 探针收集。
 *  终端候选经 onMounted 异步拉取，挂载后多冲一轮微任务等 terms 就位 */
async function mountDialog(onClose: () => void) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const settings = useSettingsStore();
  await settings.init();
  el = document.createElement("div");
  document.body.appendChild(el);
  app = createApp(SettingsDialog, { onClose });
  app.use(pinia);
  app.mount(el);
  await nextTick();
  await new Promise((resolve) => setTimeout(resolve, 0));
  await nextTick();
  return settings;
}

/** 模板固定两个 select：第 0 个=外观主题，第 1 个=终端偏好 */
function selectAt(i: number): HTMLSelectElement {
  const sels = el!.querySelectorAll<HTMLSelectElement>("select");
  if (sels.length !== 2) throw new Error(`期望 2 个 select，实际 ${sels.length}`);
  return sels[i]!;
}

function changeSelect(sel: HTMLSelectElement, value: string) {
  sel.value = value;
  sel.dispatchEvent(new Event("change"));
}

// nextTick 冲刷 Vue 调度队列（v-model 写 store ref），setTimeout(0) 再冲刷 store watch 的
// applyTheme 与链式 await 微任务（模式同 settings.test.ts 的 flush）
async function flush() {
  await nextTick();
  await new Promise((resolve) => setTimeout(resolve, 0));
}

beforeEach(() => {
  mocks.values.clear();
  vi.clearAllMocks();
  mockTerms(); // 默认给一份 macOS 候选，个别用例按需覆盖
  document.documentElement.removeAttribute("data-theme");
});

afterEach(() => {
  app?.unmount();
  app = null;
  el?.remove();
  el = null;
});

describe("SettingsDialog 渲染", () => {
  it("两个 select 初值绑定 store 当前值；终端候选来自 Rust terminal_options（OS 相关、动态）", async () => {
    mocks.values.set("theme", "dark");
    mocks.values.set("terminal", "iTerm2");
    await mountDialog(vi.fn());
    expect(selectAt(0).value).toBe("dark");
    expect(selectAt(1).value).toBe("iTerm2");
    expect(Array.from(selectAt(0).options).map((o) => o.value)).toEqual(["light", "dark", "macos"]);
    expect(Array.from(selectAt(1).options).map((o) => o.value)).toEqual([
      "system", "iTerm2", "Terminal.app",
    ]);
  });

  it("无持久化时用默认值：theme=light、terminal=system", async () => {
    await mountDialog(vi.fn());
    expect(selectAt(0).value).toBe("light");
    expect(selectAt(1).value).toBe("system");
  });

  it("存量偏好不在当前 OS 候选里（跨平台迁移）：回退 system 防空白 select", async () => {
    mocks.values.set("terminal", "Warp"); // 候选里无 Warp（未装）
    const settings = await mountDialog(vi.fn());
    expect(selectAt(1).value).toBe("system");
    expect(settings.terminal).toBe("system");
  });

  it("terminal_options 失败（IPC 异常）：保留 system 兜底项，select 仍可用", async () => {
    mocks.terminalOptions.mockRejectedValue(new Error("ipc down"));
    await mountDialog(vi.fn());
    expect(Array.from(selectAt(1).options).map((o) => o.value)).toEqual(["system"]);
    expect(selectAt(1).value).toBe("system");
  });
});

describe("SettingsDialog 修改设置", () => {
  it("切换主题 select：settings.theme 更新、data-theme 立即切换（applyTheme 由 store watch 驱动）、持久化写回", async () => {
    const settings = await mountDialog(vi.fn());
    expect(document.documentElement.getAttribute("data-theme")).toBe("light"); // init 已应用
    changeSelect(selectAt(0), "macos");
    await flush();
    expect(settings.theme).toBe("macos");
    expect(document.documentElement.getAttribute("data-theme")).toBe("macos");
    expect(mocks.set).toHaveBeenCalledWith("theme", "macos");
  });

  it("切换终端 select：settings.terminal 更新并写回", async () => {
    const settings = await mountDialog(vi.fn());
    changeSelect(selectAt(1), "iTerm2");
    await flush();
    expect(settings.terminal).toBe("iTerm2");
    expect(mocks.set).toHaveBeenCalledWith("terminal", "iTerm2");
  });
});

describe("SettingsDialog 关闭", () => {
  it("点「完成」或 mask 本体 emit close；点 dialog 本体不关闭（click.self）", async () => {
    const closed = vi.fn();
    await mountDialog(closed);

    el!.querySelector<HTMLElement>(".dialog")!.click(); // 冒泡到 mask 但 target 非本体
    expect(closed).not.toHaveBeenCalled();

    el!.querySelector<HTMLElement>(".mask")!.click();
    expect(closed).toHaveBeenCalledTimes(1);

    const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
      .find((b) => b.textContent?.trim() === "完成");
    if (!btn) throw new Error("按钮「完成」未找到");
    btn.click();
    expect(closed).toHaveBeenCalledTimes(2);
  });
});
