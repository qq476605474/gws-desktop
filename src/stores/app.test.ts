// @vitest-environment happy-dom
//! 列表刷新的接线测试。
//!
//! Agent（Paseo）是另一个应用，改完需求/模块没有通知；桌面端的做法是
//! **窗口回到前台时重读**。这里钉住三件事：
//! - 刷新成功后 `refreshTick` 要 +1（详情页靠它重读）；
//! - 并发刷新共用同一个请求（聚焦和 onMounted 很容易撞在一起）；
//! - 2 秒内的重复聚焦不重复打 git。

import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(null) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

vi.mock("../bridge", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../bridge")>();
  return {
    ...actual,
    repoList: vi.fn().mockResolvedValue([]),
    envList: vi.fn().mockResolvedValue([]),
    wsList: vi.fn().mockResolvedValue([]),
    hubInfo: vi.fn().mockResolvedValue({ root: "/hub", name: "hub" }),
  };
});

import * as api from "../bridge";
import { useAppStore } from "./app";

const calls = (fn: unknown) => (fn as ReturnType<typeof vi.fn>).mock.calls.length;

/** 把已有的 promise 链走完（桥接函数都是立即 resolve 的）。 */
async function settle() {
  for (let i = 0; i < 6; i++) await Promise.resolve();
}

function store() {
  const pinia = createPinia();
  setActivePinia(pinia);
  const app = useAppStore();
  app.hub = { root: "/hub", name: "hub" } as never;
  return app;
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-10-21T10:00:00"));
});

afterEach(() => vi.useRealTimers());

describe("列表刷新", () => {
  it("成功后 refreshTick 加一（打开着的详情页靠它重读）", async () => {
    const app = store();
    expect(app.refreshTick).toBe(0);

    await app.refreshAll();

    expect(app.refreshTick).toBe(1);
    expect(calls(api.repoList)).toBe(1);
    expect(app.loadError).toBe("");
  });

  it("并发刷新共用同一个请求", async () => {
    const app = store();

    await Promise.all([app.refreshAll(), app.refreshAll(), app.refreshAll()]);

    expect(calls(api.repoList)).toBe(1);
    expect(app.refreshTick).toBe(1);
  });

  it("聚焦触发的刷新有 2 秒去抖：来回切窗口不会反复打 git", async () => {
    const app = store();
    await app.refreshAll();
    expect(calls(api.repoList)).toBe(1);

    app.refreshIfStale();
    await settle();
    expect(calls(api.repoList), "2 秒内不重复").toBe(1);

    vi.setSystemTime(new Date("2026-10-21T10:00:02.500"));
    app.refreshIfStale();
    await settle();
    expect(calls(api.repoList), "过了 2 秒才真的刷新").toBe(2);
    expect(app.refreshTick).toBe(2);
  });

  it("没有 hub 时什么都不发", async () => {
    const pinia = createPinia();
    setActivePinia(pinia);
    const app = useAppStore();

    app.refreshIfStale();
    await app.refreshAll();

    expect(calls(api.repoList)).toBe(0);
    expect(app.refreshTick).toBe(0);
  });
});
