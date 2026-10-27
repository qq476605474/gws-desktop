// @vitest-environment happy-dom
//! App.vue 的**挂载**测试。
//!
//! # 为什么专门测「能不能挂载」
//!
//! 实测踩过：给 `App.vue` 加了个 `useMessage()` 想弹提示，结果**整个界面白屏** ——
//! 因为 `App.vue` 正是 `NMessageProvider` 的**提供方**，`useMessage()` 只能用在它
//! 内部的后代组件里，在外面调会直接抛错、把整棵组件树带崩。
//!
//! 而这类错误 **类型检查（vue-tsc）和构建（vite）都不报**，只在运行时炸。
//! 所以这里挂一次：挂了就说明会白屏。
//! （另有一道静态检查 `scripts/check-provider-hooks.mjs` 兜同一类问题。）

import { mount } from "@vue/test-utils";
import { createPinia } from "pinia";
import { describe, expect, it, vi } from "vitest";

// Tauri 的 IPC 在测试环境里不存在，一律挡掉
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(null) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock("@tauri-apps/api/app", () => ({
  getVersion: vi.fn().mockResolvedValue("1.0.5"),
}));
// 偏好存储（plugin-store）也读 Tauri IPC，挡掉 —— 它的 `get()` 内部会把 IPC 结果
// 当 `[值, 是否存在]` 解构，兜底的 `invoke` 返回 null 会炸成未处理拒绝。
vi.mock("@tauri-apps/plugin-store", () => ({
  load: vi.fn().mockResolvedValue({
    get: vi.fn().mockResolvedValue(undefined),
    set: vi.fn().mockResolvedValue(undefined),
  }),
}));

// 兜底：Tauri 真实的 `invoke` 会读 `window.__TAURI_INTERNALS__.invoke`。
// 万一有哪个调用点没被上面的 mock 拦住（比如别的包内部直接引了 core），
// 它就会以「读 undefined 的 invoke」炸成**未处理拒绝**，把 `npm test` 带成失败。
// 这里给个空实现，保证漏网调用也只是静静地返回 null。
(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
  invoke: async () => null,
  transformCallback: (cb: unknown) => cb,
};

import App from "./App.vue";
import { busy } from "./busy";
import { useAppStore } from "./stores/app";
import type { HubInfo } from "./types";

const flush = () => new Promise((r) => setTimeout(r, 0));

describe("App.vue", () => {
  it("能挂载不抛错（抛错 = 打开就是白屏）", () => {
    const wrapper = mount(App, { global: { plugins: [createPinia()] } });
    expect(wrapper.exists()).toBe(true);
  });

  it("左上角是 hub 名字（点开有菜单）+ 版本号，左下角只剩「开 AI 会话」", async () => {
    // 实测反馈：左下角挤了「开 AI 会话 / 版本号 / 切换 hub / 构建时间」四样，
    // 又丑又乱；版本号和切换 hub 应该跟 hub 名字一起放左上角。
    // 再一轮反馈：左上角的「切换 hub」裸链接和整宽更新胶囊不好看，改成
    // hub 名字下拉菜单 + 安静的版本号。
    const pinia = createPinia();
    const store = useAppStore(pinia);
    const hub: HubInfo = {
      root: "/tmp/gws-test-hub",
      name: "测试 hub",
      envs: [["dev", "开发"]],
      repo_count: 0,
      workspace_count: 0,
      dochub_name: "doc-hub",
      dochub_ready: false,
      dochub_branch: null,
      branch_prefix: "feature",
      branch_date_digits: 8,
      migration_notes: [],
      quick_links: [],
      git_name: "",
      git_email: "",
    };
    store.setHub(hub);

    const wrapper = mount(App, { global: { plugins: [pinia] } });
    await flush();
    await wrapper.vm.$nextTick();
    await flush();
    await wrapper.vm.$nextTick();

    const brand = wrapper.find(".brand");
    expect(
      brand.find(".brand-head .version-badge").text(),
      "版本号跟标题同一行、放右边",
    ).toBe("v1.0.5");
    expect(brand.find(".hub-trigger").exists(), "hub 名字在下一行、点开是菜单").toBe(true);
    expect(brand.text(), "hub 名字旁边不该再有裸「切换 hub」链接").not.toContain("切换 hub");
    expect(brand.find(".update-bar").exists(), "已是最新时不该出现更新条").toBe(false);

    const foot = wrapper.find(".sider-foot");
    expect(foot.text(), "左下角只留开 AI 会话").toContain("开 AI 会话");
    expect(foot.find("button.linkish").exists(), "切换 hub 不该再在左下角").toBe(false);
    expect(foot.text(), "构建时间没必要显示").not.toContain("构建");
    expect(wrapper.find(".build-stamp").exists()).toBe(false);
  });

  it("遮罩只给 withBusy 的慢操作，普通请求不翻出上一次的旧进度", async () => {
    const wrapper = mount(App, { global: { plugins: [createPinia()] } });

    // bridge.tracked 的普通请求：只加 count。修复前遮罩看 count，于是把上一次
    // 「补齐环境 pre」的标题和日志一起弹出来（实测：点「开 AI 会话」就冒出来）
    busy.count = 1;
    busy.mask = 0;
    busy.label = "补齐环境 pre";
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".busy-mask").exists(), "普通请求不该盖遮罩").toBe(false);

    busy.mask = 1;
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".busy-mask").exists(), "慢操作要盖遮罩").toBe(true);

    busy.count = 0;
    busy.mask = 0;
    busy.label = "";
  });
});
