// @vitest-environment happy-dom
//! 「用编辑器打开」按钮的挂载测试。
//!
//! 钉住四件事：
//! 1. 一个都没配 → 弹提示让人去设置里配，而不是点了没反应；
//! 2. 只配一个 → 直接用它打开（预设和自定义命令各一条）；
//! 3. 配了多个 → 按配置顺序列出全部，点哪个用哪个；
//! 4. 没有路径时按钮禁用 —— 否则点了只会看到一个"目录不存在"的报错。

import { mount } from "@vue/test-utils";
import { NDropdown } from "naive-ui";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";

import EditorButton from "./EditorButton.vue";
import * as api from "../bridge";
import { installNotifier } from "../lib/notify";
import { useAppStore } from "../stores/app";

vi.mock("../bridge", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../bridge")>();
  return { ...actual, openInEditor: vi.fn().mockResolvedValue(undefined) };
});

const warnings: string[] = [];

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  warnings.length = 0;
  installNotifier({
    error: () => {},
    warning: (t) => warnings.push(t),
    success: () => {},
  });
});

function entry(id: string, command = "", label = "") {
  return {
    key: `${id}-${command}`,
    id,
    label: label || (id === "custom" ? "" : id),
    command,
  };
}

describe("EditorButton", () => {
  it("一个都没配：提示去设置里添加，不调用后端", async () => {
    const w = mount(EditorButton, { props: { path: "/tmp/repo" } });
    await w.trigger("click");

    expect(warnings).toHaveLength(1);
    expect(warnings[0]).toContain("常用编辑器");
    expect(api.openInEditor).not.toHaveBeenCalled();
  });

  it("只配了一个预设：直接打开", async () => {
    const app = useAppStore();
    app.editors = [entry("idea", "", "IntelliJ IDEA")];

    const w = mount(EditorButton, { props: { path: "/tmp/repo" } });
    await w.trigger("click");

    expect(api.openInEditor).toHaveBeenCalledWith("/tmp/repo", "idea", "");
  });

  it("只配了一个自定义命令：命令一并传给后端", async () => {
    const app = useAppStore();
    app.editors = [entry("custom", "idea {path}")];

    const w = mount(EditorButton, { props: { path: "/tmp/repo" } });
    await w.trigger("click");

    expect(api.openInEditor).toHaveBeenCalledWith("/tmp/repo", "custom", "idea {path}");
  });

  it("配了多个：按配置顺序列出，点哪个用哪个", async () => {
    const app = useAppStore();
    app.editors = [
      entry("rebased", "", "Rebased"),
      entry("custom", "zed {path}"),
      entry("idea", "", "IntelliJ IDEA"),
    ];

    const w = mount(EditorButton, { props: { path: "/tmp/repo" } });
    const dd = w.findComponent(NDropdown);
    const options = dd.props("options") as { key: string; label: string }[];

    // 顺序 = 设置里的顺序；自定义条目显示命令本身
    expect(options.map((o) => o.label)).toEqual(["Rebased", "zed {path}", "IntelliJ IDEA"]);

    // 点按钮只是展开菜单，不直接打开
    await w.find("button").trigger("click");
    expect(api.openInEditor).not.toHaveBeenCalled();

    dd.vm.$emit("select", options[2].key);
    expect(api.openInEditor).toHaveBeenCalledWith("/tmp/repo", "idea", "");
  });

  it("自定义条目没填命令时不算数：只有一个有效项就直接打开", async () => {
    const app = useAppStore();
    app.editors = [entry("custom", ""), entry("zed", "", "Zed")];

    const w = mount(EditorButton, { props: { path: "/tmp/repo" } });
    await w.trigger("click");

    expect(api.openInEditor).toHaveBeenCalledWith("/tmp/repo", "zed", "");
  });

  it("没有路径时禁用", () => {
    const w = mount(EditorButton, { props: { path: "" } });
    expect(w.find("button").attributes("disabled")).toBeDefined();
  });
});