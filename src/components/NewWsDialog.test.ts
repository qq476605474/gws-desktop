// @vitest-environment happy-dom
//! 「新建需求」弹窗的挂载测试。
//!
//! # 为什么要挂一次，而不只测纯函数
//!
//! 实测反馈：**选了「完整分支名」，上面那栏「名称」还在，还必填** ——
//! 规则本身（`lib/newWsForm.ts`）单测已经钉住了，坏的是**接线**：
//! 模板上 `v-if` 挂错条件、或者 `canCreate` 没换成新规则，纯函数测试
//! 一条都不会红。所以这里按用户的操作走一遍：切到「完整分支名」→
//! 那一栏消失 → 只填分支名时「创建」可点。
//!
//! 注意 NewWsDialog 里用了 `useMessage()`，**必须包在 `NMessageProvider` 里**，
//! 否则一挂载就抛错（这正是白屏那类事故的形状）。

import { mount } from "@vue/test-utils";
import { NMessageProvider } from "naive-ui";
import { createPinia, setActivePinia } from "pinia";
import { h } from "vue";
import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(null) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

vi.mock("../bridge", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../bridge")>();
  return {
    ...actual,
    wsNew: vi.fn().mockResolvedValue({ branch: "feature-20261021-x", doc: null }),
  };
});

import NewWsDialog from "./NewWsDialog.vue";
import * as api from "../bridge";
import { useAppStore } from "../stores/app";

function mountDialog() {
  const pinia = createPinia();
  setActivePinia(pinia);
  // 工作区至少要有一个仓库可选，否则「创建」恒不可点
  useAppStore().repos = [
    {
      name: "doc-hub",
      main_branch: "main",
      usages: [],
    env_usages: [],
      remote_url: "",
      is_doc: true,
    },
    {
      name: "demo-payment",
      main_branch: "main",
      usages: [],
    env_usages: [],
      remote_url: "",
      is_doc: false,
    },
  ];

  return mount(
    { render: () => h(NMessageProvider, null, { default: () => h(NewWsDialog) }) },
    {
      global: {
        plugins: [pinia],
        // 弹窗本体是 teleport 出去的，测试里让它原地渲染，find 才找得到
        stubs: { teleport: true },
      },
    },
  );
}

/** 按 placeholder 前缀找一个输入框。 */
const inputByPlaceholder = (w: ReturnType<typeof mountDialog>, prefix: string) =>
  w.find(`input[placeholder^="${prefix}"]`);

const createButton = (w: ReturnType<typeof mountDialog>) =>
  w.findAll("button").find((b) => b.text().includes("创建"))!;

const flush = () => new Promise((r) => setTimeout(r, 0));

/** 点一下「完整分支名」那个选择框。 */
async function pickFullBranchMode(w: ReturnType<typeof mountDialog>) {
  const box = w.findAll(".n-checkbox").find((c) => c.text().includes("完整分支名"))!;
  await box.trigger("click");
  await w.vm.$nextTick();
}

describe("NewWsDialog", () => {
  it("默认是前缀组合：有「分支后缀」这一栏，且没填就不能建", () => {
    const w = mountDialog();
    expect(inputByPlaceholder(w, "只写后半段").exists()).toBe(true);
    expect(createButton(w).attributes("disabled")).toBeDefined();
  });

  it("切到「完整分支名」→「分支后缀」整栏消失，也不再拦着提交", async () => {
    const w = mountDialog();
    await pickFullBranchMode(w);

    expect(inputByPlaceholder(w, "只写后半段").exists()).toBe(false);
    // 这一栏没了，却还要求它 —— 就是用户遇到的 bug
    expect(createButton(w).attributes("disabled")).toBeDefined();

    await inputByPlaceholder(w, "如 feature-20261010").setValue("feature-20261010-revamp");
    expect(createButton(w).attributes("disabled")).toBeUndefined();
  });

  it("后缀里已经写了日期 → 有提示，且提交的是**去重后的完整分支**", async () => {
    // 实测坏分支：`bugfix-20261020-20261020-ca-mobile-vault-2.3-bugfix`
    const w = mountDialog();
    await inputByPlaceholder(w, "只写后半段").setValue(
      "20261020-ca-mobile-vault-2.3-bugfix",
    );
    await w.vm.$nextTick();

    expect(w.find(".date-warn").exists(), "要告诉用户日期被自动去掉了").toBe(true);

    await createButton(w).trigger("click");
    await flush();

    const arg = (api.wsNew as ReturnType<typeof vi.fn>).mock.calls[0]![0];
    expect(arg.branch).toMatch(/^feature-\d{8}-ca-mobile-vault-2\.3-bugfix$/);
    expect(arg.branch.match(/\d{8}/g)?.length, "只能有一个日期").toBe(1);
  });

  it("中文名那一栏在最上面，且说明白了它才是给人看的名字", () => {
    const w = mountDialog();
    const labels = w.findAll("label").map((l) => l.text());
    expect(labels[0]).toBe("需求名称");
    expect(labels).toContain("分支后缀");
    // 「名称」这个含糊叫法不该再出现
    expect(labels).not.toContain("名称");
    expect(labels).not.toContain("中文备注");
  });

  it("先选分支前缀，再填分支后缀", () => {
    // 实测反馈：拼接是从左往右（前缀-日期-后缀），表单就该按这个顺序排
    const w = mountDialog();
    const labels = w.findAll("label").map((l) => l.text());
    const prefixAt = labels.indexOf("分支前缀");
    const suffixAt = labels.indexOf("分支后缀");
    expect(prefixAt, "该有「分支前缀」").toBeGreaterThanOrEqual(0);
    expect(suffixAt).toBeGreaterThan(prefixAt);
    // 单独一个「前缀」太含糊，已改叫「分支前缀」
    expect(labels).not.toContain("前缀");
  });
});