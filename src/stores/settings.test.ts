// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { nextTick } from "vue";

const mocks = vi.hoisted(() => {
  const values = new Map<string, unknown>();
  const get = vi.fn(async (key: string) => values.get(key));
  const set = vi.fn(async (_key: string, _value: unknown) => {});
  return { values, get, set, load: vi.fn(async () => ({ get, set })) };
});

vi.mock("@tauri-apps/plugin-store", () => ({
  load: mocks.load,
  Store: class Store {},
}));

import { useSettingsStore } from "./settings";

// nextTick 冲刷 Vue 调度队列，setTimeout(0) 再冲刷 watcher 回调里链式 await 的微任务
async function flush() {
  await nextTick();
  await new Promise((resolve) => setTimeout(resolve, 0));
}

beforeEach(() => {
  setActivePinia(createPinia());
  mocks.values.clear();
  vi.clearAllMocks();
  document.documentElement.removeAttribute("data-theme");
});

describe("settings store 持久化", () => {
  it("init 读取持久化值并应用主题", async () => {
    mocks.values.set("lastHub", "/a/b");
    mocks.values.set("terminal", "Warp");
    mocks.values.set("theme", "dark");
    const store = useSettingsStore();
    await store.init();
    expect(mocks.load).toHaveBeenCalledWith("settings.json", { autoSave: true });
    expect(store.lastHub).toBe("/a/b");
    expect(store.terminal).toBe("Warp");
    expect(store.theme).toBe("dark");
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
  });

  it("init 无持久化值时用默认值", async () => {
    const store = useSettingsStore();
    await store.init();
    expect(store.lastHub).toBe("");
    expect(store.terminal).toBe("system");
    expect(store.theme).toBe("light");
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
  });

  it("修改 theme → data-theme 更新并写回 store", async () => {
    const store = useSettingsStore();
    await store.init();
    store.theme = "macos";
    await flush();
    expect(document.documentElement.getAttribute("data-theme")).toBe("macos");
    expect(mocks.set).toHaveBeenCalledWith("theme", "macos");
  });

  it("init 幂等：顺序两次调用只 load 一次", async () => {
    const store = useSettingsStore();
    await store.init();
    await store.init();
    expect(mocks.load).toHaveBeenCalledTimes(1);
  });

  it("init 并发幂等：Promise.all 并发两次只 load 一次", async () => {
    const store = useSettingsStore();
    await Promise.all([store.init(), store.init()]);
    expect(mocks.load).toHaveBeenCalledTimes(1);
  });

  it("init 失败清缓存可重试：首次 rejects，再次调用重新 load 并成功", async () => {
    mocks.load.mockRejectedValueOnce(new Error("load failed"));
    const store = useSettingsStore();
    await expect(store.init()).rejects.toThrow("load failed");
    await store.init();
    expect(mocks.load).toHaveBeenCalledTimes(2);
    expect(store.theme).toBe("light");
  });

  it("init 加载阶段不写回：读取持久化值后 set 未被调用", async () => {
    mocks.values.set("lastHub", "/a/b");
    mocks.values.set("terminal", "Warp");
    mocks.values.set("theme", "dark");
    const store = useSettingsStore();
    await store.init();
    await flush();
    expect(mocks.set).not.toHaveBeenCalled();
  });

  it("修改 lastHub → set(\"lastHub\", ...) 被调用", async () => {
    const store = useSettingsStore();
    await store.init();
    store.lastHub = "/new/hub";
    await flush();
    expect(mocks.set).toHaveBeenCalledWith("lastHub", "/new/hub");
  });
});
