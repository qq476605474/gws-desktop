// @vitest-environment happy-dom
//! 左上角**更新提示条**的挂载测试。
//!
//! 钉的是**接线**：什么时候挂出那条提示、点开之后「下载并更新」真的发请求、
//! 失败要看得见。规则本身（版本比较）在 core 有单测，那是另一层。
//!
//! 版本号本身由 `VersionBadge` 一直显示，见 `VersionBadge.test.ts`；这里只测
//! 「有新版才出现的那一条」。

import { mount } from "@vue/test-utils";
import { NMessageProvider } from "naive-ui";
import { h } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(null) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock("@tauri-apps/api/app", () => ({
  getVersion: vi.fn().mockResolvedValue("1.0.5"),
}));

vi.mock("../bridge", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../bridge")>();
  return {
    ...actual,
    checkUpdate: vi.fn(),
    updateInstall: vi.fn(),
    restartApp: vi.fn().mockResolvedValue(undefined),
  };
});

import * as api from "../bridge";
import UpdatePill from "./UpdatePill.vue";

const base = {
  current: "1.0.2",
  release_url: "",
  releases_url: "https://gitlab.example.com/demo-pub/gws/-/releases",
  asset: "gws-desktop_1.0.2_aarch64.dmg",
};

function mountPill() {
  return mount(
    { render: () => h(NMessageProvider, null, { default: () => h(UpdatePill) }) },
    { global: { stubs: { teleport: true } } },
  );
}

const flush = () => new Promise((r) => setTimeout(r, 0));

// 每个用例从干净的 mock 计数开始：`restartApp` 是在 800ms 后调的，
// 不清的话上一条用例的定时器会算进下一条（踩过）
beforeEach(() => vi.clearAllMocks());

describe("更新提示", () => {
  it("已是最新 → **什么都不渲染**（版本号由 VersionBadge 常驻显示）", async () => {
    (api.checkUpdate as ReturnType<typeof vi.fn>).mockResolvedValue({
      ...base,
      state: "up_to_date",
      latest: null,
    });
    const w = mountPill();
    await flush();
    await w.vm.$nextTick();
    expect(w.find(".update-bar").exists(), "没新版不该占位").toBe(false);
    expect(w.text(), "版本号不在这里重复显示").not.toContain("v1.0.5");
  });

  it("有新版 → 一条**单行**提示，点开是更新对话框", async () => {
    (api.checkUpdate as ReturnType<typeof vi.fn>).mockResolvedValue({
      ...base,
      state: "available",
      latest: "1.0.3",
    });
    const w = mountPill();
    await flush();
    await w.vm.$nextTick();

    const bars = w.findAll(".update-bar");
    expect(bars.length, "只挂一条，不要整宽大胶囊堆成两行").toBe(1);
    expect(bars[0].text()).toContain("有新版本 1.0.3");

    await bars[0].trigger("click");
    await w.vm.$nextTick();
    expect(w.text()).toContain("当前版本");
    expect(w.text()).toContain("下载并更新");
  });

  it("state 是 available 但 latest 是空 → 不显示（没版本号可提示）", async () => {
    (api.checkUpdate as ReturnType<typeof vi.fn>).mockResolvedValue({
      ...base,
      state: "available",
      latest: null,
    });
    const w = mountPill();
    await flush();
    await w.vm.$nextTick();
    expect(w.find(".update-bar").exists()).toBe(false);
  });

  it("点「下载并更新」要真的发请求；装好（restart）后退出应用交给重启助手", async () => {
    (api.checkUpdate as ReturnType<typeof vi.fn>).mockResolvedValue({
      ...base,
      state: "available",
      latest: "1.0.3",
    });
    (api.updateInstall as ReturnType<typeof vi.fn>).mockResolvedValue({
      version: "1.0.3",
      next: "restart",
      package: "/tmp/gws-update/gws-desktop_1.0.3_aarch64.dmg",
      message: "已替换应用，正在重启…",
      command: null,
    });
    const w = mountPill();
    await flush();
    await w.vm.$nextTick();
    await w.find(".update-bar").trigger("click");
    await w.vm.$nextTick();

    const btn = w.findAll("button").find((b) => b.text().includes("下载并更新"))!;
    await btn.trigger("click");
    await flush();

    expect((api.updateInstall as ReturnType<typeof vi.fn>).mock.calls.length).toBe(1);
    // 退出由前端触发（core 那边已经挂好了重启助手）
    await new Promise((r) => setTimeout(r, 900));
    expect((api.restartApp as ReturnType<typeof vi.fn>).mock.calls.length).toBe(1);
  });

  it("装失败要把原因显示出来（而不是静默）", async () => {
    (api.checkUpdate as ReturnType<typeof vi.fn>).mockResolvedValue({
      ...base,
      state: "available",
      latest: "1.0.3",
    });
    (api.updateInstall as ReturnType<typeof vi.fn>).mockRejectedValue(
      new Error("下载的安装包校验不过（现有安装未改动）"),
    );
    const w = mountPill();
    await flush();
    await w.vm.$nextTick();
    await w.find(".update-bar").trigger("click");
    await w.vm.$nextTick();
    await w.findAll("button").find((b) => b.text().includes("下载并更新"))!.trigger("click");
    await flush();
    await w.vm.$nextTick();

    expect(w.text()).toContain("校验不过");
    expect((api.restartApp as ReturnType<typeof vi.fn>).mock.calls.length).toBe(0);
  });

  it("Linux 那条（manual）：要把**命令**单独摆出来，而不是混在一段话里", async () => {
    // 实测反馈：命令混在提示里，人得自己挑那截，小白不知道怎么下手
    (api.checkUpdate as ReturnType<typeof vi.fn>).mockResolvedValue({
      ...base,
      state: "available",
      latest: "1.0.3",
    });
    (api.updateInstall as ReturnType<typeof vi.fn>).mockResolvedValue({
      version: "1.0.3",
      next: "manual",
      package: "/tmp/gws-update/gws_1.0.3_amd64.deb",
      message: "安装包已下好并校验通过",
      command: "sudo dpkg -i /tmp/gws-update/gws_1.0.3_amd64.deb",
    });
    const w = mountPill();
    await flush();
    await w.vm.$nextTick();
    await w.find(".update-bar").trigger("click");
    await w.vm.$nextTick();
    await w.findAll("button").find((b) => b.text().includes("下载并更新"))!.trigger("click");
    await flush();
    await w.vm.$nextTick();

    // 命令单独一行、可整条选中；并且有「复制命令」按钮
    expect(w.find(".cmd").text()).toBe("sudo dpkg -i /tmp/gws-update/gws_1.0.3_amd64.deb");
    expect(w.findAll("button").some((b) => b.text().includes("复制命令"))).toBe(true);
  });

  it("检查失败（没网）→ 不报错、不挂提示", async () => {
    (api.checkUpdate as ReturnType<typeof vi.fn>).mockRejectedValue(new Error("连接超时"));
    const w = mountPill();
    await flush();
    await w.vm.$nextTick();
    expect(w.find(".update-bar").exists(), "查不到新版就安静地不出现").toBe(false);
    expect(w.text()).not.toContain("连接超时");
  });
});
