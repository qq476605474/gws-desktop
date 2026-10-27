// @vitest-environment happy-dom
//! 「导入需求」的挂载测试 —— 重点是**粘贴对方发来的配置**那条路。
//!
//! 实测反馈纠正过一版：这个场景属于**导入**（拉取已推送到远程的分支），
//! 不是新建（那条分支还不存在）。而且最要紧的一点是：
//! **配置里点名的仓库，我本地没有时要自动补拉** —— 界面必须先把这件事说清楚。
//!
//! 这类「接线」问题（v-if 挂错、按钮没接上、标签少 import）纯函数测试抓不到，
//! 所以按用户的操作走一遍。`check-vue-components.mjs` 兜另一类（少了 import 的标签）。

import { mount } from "@vue/test-utils";
import { NMessageProvider } from "naive-ui";
import { createPinia, setActivePinia } from "pinia";
import { h } from "vue";
import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(null) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

vi.mock("../bridge", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../bridge")>();
  return { ...actual, wsShareParse: vi.fn(), wsShareImport: vi.fn(), wsGet: vi.fn() };
});

import * as api from "../bridge";
import GetWsDialog from "./GetWsDialog.vue";
import { useAppStore } from "../stores/app";

function mountDialog() {
  const pinia = createPinia();
  setActivePinia(pinia);
  // 本地只有这两个仓库；配置里点名的 demo-api 本地没有
  useAppStore().repos = [
    { name: "doc-hub", main_branch: "main", usages: [],
    env_usages: [], remote_url: "", is_doc: true },
    { name: "demo-payment", main_branch: "main", usages: [],
    env_usages: [], remote_url: "", is_doc: false },
  ];
  return mount(
    { render: () => h(NMessageProvider, null, { default: () => h(GetWsDialog) }) },
    { global: { plugins: [pinia], stubs: { teleport: true } } },
  );
}

const flush = () => new Promise((r) => setTimeout(r, 0));

const PAYLOAD = {
  schema: 1,
  branch: "feature-20261013-private-agent-v3",
  title: "示例需求v1.3",
  base: ["ws:feature-20260901-smart-reach"],
  doc_family: "示例需求",
  doc_hub: "doc-hub",
  notes: "提测环境：dev",
  links: [{ label: "需求", url: "https://tapd.example/1" }],
  modules: [
    { name: "demo-payment", url: "https://git/x/p.git" },
    { name: "demo-api", url: "https://git/x/a.git" },
  ],
};

describe("导入需求 · 粘贴对方发来的配置", () => {
  it("识别后表单填好，本地缺的仓库要明说会补拉", async () => {
    const w = mountDialog();
    (api.wsShareParse as ReturnType<typeof vi.fn>).mockResolvedValue(PAYLOAD);

    await w.find("textarea").setValue(`----- gws 需求配置 -----\n${JSON.stringify(PAYLOAD)}`);
    await w.findAll("button").find((b) => b.text().includes("识别配置"))!.trigger("click");
    await flush();
    await w.vm.$nextTick();

    // 分支名逐字照搬（它是目录名，必须一致）
    expect(w.find("input[value='feature-20261013-private-agent-v3']").exists()).toBe(true);
    expect(w.find("input[value='示例需求v1.3']").exists()).toBe(true);
    expect(w.find("input[value='示例需求']").exists()).toBe(true);
    // 本地没有的那个要标出来 —— 否则人不知道还要等克隆
    expect(w.text()).toContain("本地没有，拉取时自动补建");
    expect(w.text()).toContain("demo-api");
    // **文档库也要列出来**：它不在 modules 里（用「需求分类」表达），不显示的话
    // 人一看「我有 4 个仓库，怎么只复制了 3 个」（实测反馈）
    expect(w.text()).toContain("doc-hub");
    expect(w.text()).toContain(PAYLOAD.doc_family);
    // 拉取按钮可用
    const btn = w.findAll("button").find((b) => b.text().includes("拉取"))!;
    expect(btn.attributes("disabled")).toBeUndefined();
  });

  it("识别失败要说清楚（粘了半截 / 不是配置），且不留下半份状态", async () => {
    const w = mountDialog();
    (api.wsShareParse as ReturnType<typeof vi.fn>).mockRejectedValue(
      new Error("这段内容不完整：JSON 没有闭合的 `}`"),
    );

    await w.find("textarea").setValue("随便粘了点什么 {");
    await w.findAll("button").find((b) => b.text().includes("识别配置"))!.trigger("click");
    await flush();
    await w.vm.$nextTick();

    expect(w.text()).toContain("没有闭合");
    // 失败后应回到普通导入（没有「按对方发来的配置拉取」那套）
    expect(w.text()).not.toContain("按对方发来的配置拉取");
  });

  it("不给配置时，还是按远程分支名导入（老路径没被弄坏）", async () => {
    const w = mountDialog();
    // 按钮要**每次重新找**：NButton 重渲染后元素可能被换掉，拿着旧 wrapper 读属性会读到旧的
    const pull = () => w.findAll("button").find((b) => b.text().includes("拉取"))!;
    // 没填分支名 → 不可点
    expect(pull().attributes("disabled")).toBeDefined();

    await w.find("input[placeholder^='如 feature-20261010-收银台改版']").setValue("feature-x");
    await w.vm.$nextTick();
    expect(pull().attributes("disabled")).toBeUndefined();
    // 没有粘贴区那份状态
    expect(w.text()).not.toContain("按对方发来的配置拉取");
  });
});