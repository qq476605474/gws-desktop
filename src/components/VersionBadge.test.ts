// @vitest-environment happy-dom
//! 左上角常驻版本号的测试。
//!
//! 实测需求：不管有没有新版，都得能看见自己装的是哪一版。
//! 以前版本号和更新提示挤在一个胶囊里，有新版时文案一换，版本号反而没了。

import { mount } from "@vue/test-utils";
import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/app", () => ({ getVersion: vi.fn() }));

import { getVersion } from "@tauri-apps/api/app";
import VersionBadge from "./VersionBadge.vue";

const flush = () => new Promise((r) => setTimeout(r, 0));

describe("版本号", () => {
  it("拿到版本就显示 v<版本>", async () => {
    (getVersion as ReturnType<typeof vi.fn>).mockResolvedValue("1.0.5");
    const w = mount(VersionBadge);
    await flush();
    await w.vm.$nextTick();
    expect(w.find(".version-badge").text()).toBe("v1.0.5");
  });

  it("读不到版本（浏览器预览 / IPC 异常）→ 显示 v—，不崩", async () => {
    (getVersion as ReturnType<typeof vi.fn>).mockRejectedValue(new Error("no ipc"));
    const w = mount(VersionBadge);
    await flush();
    await w.vm.$nextTick();
    expect(w.find(".version-badge").text()).toBe("v—");
  });
});
