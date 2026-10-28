// @vitest-environment happy-dom
//! 左上角 hub 菜单的挂载测试。
//!
//! 实测反馈：以前改名是名字旁的铅笔、「切换 hub」是左下角一行裸链接，
//! 又碎又乱。现在低频操作都收进 hub 名字的下拉菜单，这里按用户的操作
//! 走一遍：点名字 → 菜单三项 → 各自的接线。

import { mount } from "@vue/test-utils";
import { NMessageProvider } from "naive-ui";
import { createPinia } from "pinia";
import { h } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(null) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

vi.mock("../bridge", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../bridge")>();
  return {
    ...actual,
    openInFinder: vi.fn().mockResolvedValue(undefined),
    hubRename: vi.fn(),
    copyText: vi.fn().mockResolvedValue(undefined),
  };
});

import * as api from "../bridge";
import { useAppStore } from "../stores/app";
import type { HubInfo } from "../types";
import HubMenu from "./HubMenu.vue";

const HUB: HubInfo = {
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

const flush = () => new Promise((r) => setTimeout(r, 0));

function mountMenu() {
  const pinia = createPinia();
  const store = useAppStore(pinia);
  store.setHub(HUB);
  const w = mount(
    { render: () => h(NMessageProvider, null, { default: () => h(HubMenu) }) },
    { global: { plugins: [pinia], stubs: { teleport: true } } },
  );
  return { w, store };
}

/** 打开下拉菜单。 */
async function openMenu(w: ReturnType<typeof mountMenu>["w"]) {
  await w.find(".hub-trigger").trigger("click");
  await flush();
  await w.vm.$nextTick();
}

/** 菜单项（按可见文字找）。 */
function option(w: ReturnType<typeof mountMenu>["w"], label: string) {
  const item = w.findAll(".n-dropdown-option").find((el) => el.text().includes(label));
  expect(item, `菜单里要有「${label}」`).toBeTruthy();
  return item!;
}

/** 点菜单项：select 的监听挂在内部 body 上，事件要打在它身上才会冒到菜单根。 */
async function clickOption(w: ReturnType<typeof mountMenu>["w"], label: string) {
  const item = option(w, label);
  const body = item.find(".n-dropdown-option-body");
  await (body.exists() ? body : item).trigger("click");
  await flush();
  await w.vm.$nextTick();
}

beforeEach(() => vi.clearAllMocks());

describe("hub 菜单", () => {
  it("名字本身就是入口：显示 hub 名，点开有四项低频操作", async () => {
    const { w } = mountMenu();
    const trigger = w.find(".hub-trigger");
    expect(trigger.text()).toContain("测试 hub");
    // 灰色裸文字看不出能点：入口要有下拉语义和可见箭头
    expect(trigger.attributes("aria-haspopup"), "入口要声明是下拉菜单").toBe("menu");
    expect(trigger.find(".chev").exists(), "入口要有一个看得见的下拉箭头").toBe(true);

    await openMenu(w);
    const text = w.findAll(".n-dropdown-option").map((el) => el.text());
    expect(text.join("|")).toContain("改个名字");
    expect(text.join("|")).toContain("切换 hub");
    expect(text.join("|")).toContain("打开 hub 目录");
    expect(text.join("|")).toContain("复制 hub 目录");
    // 「访达」是 Mac 专有词，不该出现在菜单里
    expect(text.join("|")).not.toContain("访达");
  });

  it("「复制 hub 目录」把 hub 根目录写进剪贴板", async () => {
    const { w } = mountMenu();
    await openMenu(w);
    await clickOption(w, "复制 hub 目录");
    expect((api.copyText as ReturnType<typeof vi.fn>).mock.calls[0]).toEqual([HUB.root]);
  });

  it("「打开 hub 目录」调 openInFinder，路径是 hub 根目录", async () => {
    const { w } = mountMenu();
    await openMenu(w);
    await clickOption(w, "打开 hub 目录");
    expect((api.openInFinder as ReturnType<typeof vi.fn>).mock.calls[0]?.[0]).toBe(HUB.root);
  });

  it("「切换 hub」清空当前 hub（回到启动页）", async () => {
    const { w, store } = mountMenu();
    await openMenu(w);
    await clickOption(w, "切换 hub");
    expect(store.hub).toBeNull();
  });

  it("「改个名字」预填当前名字，保存调 hubRename 并更新界面", async () => {
    (api.hubRename as ReturnType<typeof vi.fn>).mockResolvedValue({ ...HUB, name: "新名字" });
    const { w, store } = mountMenu();
    await openMenu(w);
    await clickOption(w, "改个名字");

    const input = w.find("input");
    expect(input.exists(), "要点开就能看到名字输入框").toBe(true);
    expect((input.element as HTMLInputElement).value).toBe("测试 hub");

    await input.setValue("新名字");
    await w.findAll("button").find((b) => b.text().includes("保存"))!.trigger("click");
    await flush();
    await w.vm.$nextTick();

    expect((api.hubRename as ReturnType<typeof vi.fn>).mock.calls[0]?.[0]).toBe("新名字");
    expect(store.hub?.name).toBe("新名字");
  });
});
