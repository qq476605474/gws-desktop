// @vitest-environment happy-dom
// 无 @vue/test-utils：手动 createApp().mount() 挂载 SFC；
// router / plugin-dialog mock 模式参考 src/components/WorkspaceDetail.test.ts
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createApp, type App } from "vue";
import { createPinia, setActivePinia } from "pinia";

const mocks = vi.hoisted(() => ({
  confirm: vi.fn<(message: string) => Promise<boolean>>(),
  navigate: vi.fn<(v: "startup" | "main") => void>(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: mocks.confirm,
}));

// 只 mock navigate：currentView 保持真实模块（不触发视图切换副作用）
vi.mock("../router", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../router")>();
  return { ...actual, navigate: mocks.navigate };
});

import { useHubStore } from "../stores/hub";
import TopBar from "./TopBar.vue";

let app: App | null = null;
let el: HTMLElement | null = null;

function mountTopBar() {
  const pinia = createPinia();
  setActivePinia(pinia);
  const hub = useHubStore();
  hub.setHub("/hub");
  el = document.createElement("div");
  document.body.appendChild(el);
  app = createApp(TopBar, { tab: "ws" });
  app.use(pinia);
  app.mount(el);
}

/** TopBar 的 hub 路径按钮（文案为 hub.path + ▾） */
function hubButton(): HTMLButtonElement {
  const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
    .find((b) => b.textContent?.trim() === "/hub ▾");
  if (!btn) throw new Error("hub 路径按钮未找到");
  return btn;
}

beforeEach(() => {
  mocks.confirm.mockResolvedValue(false);
});

afterEach(() => {
  app?.unmount();
  app = null;
  el?.remove();
  el = null;
  vi.clearAllMocks();
});

describe("TopBar hub 路径按钮防误触", () => {
  it("按钮渲染 hub.path ▾", () => {
    mountTopBar();
    expect(hubButton().textContent?.trim()).toBe("/hub ▾");
  });

  it("点击先 confirm；取消 → 不导航", async () => {
    mountTopBar();
    mocks.confirm.mockResolvedValue(false);
    hubButton().click();
    await vi.waitFor(() => expect(mocks.confirm).toHaveBeenCalledTimes(1));
    expect(mocks.confirm.mock.calls[0]![0]).toContain("切换");
    // 等一拍确认没有任何导航
    await new Promise((r) => setTimeout(r, 0));
    expect(mocks.navigate).not.toHaveBeenCalled();
  });

  it("confirm 确认 → navigate('startup')", async () => {
    mountTopBar();
    mocks.confirm.mockResolvedValue(true);
    hubButton().click();
    await vi.waitFor(() => expect(mocks.navigate).toHaveBeenCalledWith("startup"));
    expect(mocks.navigate).toHaveBeenCalledTimes(1);
  });

  it("confirm reject（异常）→ 按取消处理，不导航", async () => {
    mountTopBar();
    mocks.confirm.mockRejectedValue(new Error("dialog 不可用"));
    hubButton().click();
    await vi.waitFor(() => expect(mocks.confirm).toHaveBeenCalledTimes(1));
    await new Promise((r) => setTimeout(r, 0));
    expect(mocks.navigate).not.toHaveBeenCalled();
  });

  it("tab 按钮不经 confirm（防误触只针对 hub 切换）", async () => {
    mountTopBar();
    const reposBtn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
      .find((b) => b.textContent?.trim() === "仓库")!;
    reposBtn.click();
    await new Promise((r) => setTimeout(r, 0));
    expect(mocks.confirm).not.toHaveBeenCalled();
    expect(mocks.navigate).not.toHaveBeenCalled();
  });
});
