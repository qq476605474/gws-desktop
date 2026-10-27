// @vitest-environment happy-dom
//! 「用编辑器打开」按钮的挂载测试。
//!
//! 钉住三件事：
//! 1. 没设置时用 IDEA（用户指定的默认值）；
//! 2. 设置改了就按设置的来（并且把自定义命令一起传给后端）；
//! 3. 没有路径时按钮禁用 —— 否则点了只会看到一个"目录不存在"的报错。

import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";

import EditorButton from "./EditorButton.vue";
import * as api from "../bridge";
import { useAppStore } from "../stores/app";

vi.mock("../bridge", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../bridge")>();
  return { ...actual, openInEditor: vi.fn().mockResolvedValue(undefined) };
});

describe("EditorButton", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it("没设置时用 IDEA，点击调用 open_in_editor", async () => {
    const w = mount(EditorButton, { props: { path: "/tmp/repo" } });
    expect(w.text()).toContain("用编辑器打开");
    await w.trigger("click");
    expect(api.openInEditor).toHaveBeenCalledWith("/tmp/repo", "idea", "");
  });

  it("设置成其它编辑器后按设置传参", async () => {
    const app = useAppStore();
    app.editor = "vscode";
    const w = mount(EditorButton, { props: { path: "/tmp/repo" } });
    await w.trigger("click");
    expect(api.openInEditor).toHaveBeenCalledWith("/tmp/repo", "vscode", "");
  });

  it("自定义命令会一并传给后端", async () => {
    const app = useAppStore();
    app.editor = "custom";
    app.editorCommand = "idea {path}";
    const w = mount(EditorButton, { props: { path: "/tmp/repo" } });
    await w.trigger("click");
    expect(api.openInEditor).toHaveBeenCalledWith("/tmp/repo", "custom", "idea {path}");
  });

  it("没有路径时禁用", () => {
    const w = mount(EditorButton, { props: { path: "" } });
    expect(w.find("button").attributes("disabled")).toBeDefined();
  });
});
