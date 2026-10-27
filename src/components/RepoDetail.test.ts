// @vitest-environment happy-dom
//! 「仓库详情」的挂载测试 —— 钉住**点「取消」到底丢不丢改动**。
//!
//! # 为什么值得挂一次
//!
//! 实测反馈：表单填了内容，关掉重开上次的内容还在。这个组件里踩的是更隐蔽的
//! 一种：第一版**直接编「已保存的值」**（`notes` / `links`），于是点「取消」
//! 改动也留在界面上，下次点「修改」看到的是上次改了一半的内容 ——
//! 人会以为「保存过了」。
//!
//! 现在编辑走独立草稿（`editNotes` / `editLinks`），进编辑时从已保存值复制。
//! 规则很简单，但**接线**（模板绑的是草稿还是已保存值）只有挂起来才看得见。

import { mount } from "@vue/test-utils";
import { NMessageProvider } from "naive-ui";
import { createPinia, setActivePinia } from "pinia";
import { h } from "vue";
import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(null) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

const { REPO, MATRIX } = vi.hoisted(() => ({
  REPO: {
    name: "demo-api",
    main_branch: "main",
    usages: ["示例需求（feature-20261013-demo）"],
    env_usages: ["envs/dev", "envs/pre"],
    remote_url: "https://gitlab.example.com/group/demo-api.git",
    is_doc: false,
  },
  /** 已保存的备注与链接 —— 「取消」之后必须还是这两个值。 */
  MATRIX: [
    {
      name: "demo-api",
      gitlab: "",
      cells: [["dev", { url: "https://dev.example.com", domain: "dev.example.com" }]],
      notes: "已保存的备注",
      links: [{ label: "tapd", url: "https://tapd.example.com/1" }],
    },
  ],
}));

vi.mock("../bridge", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../bridge")>();
  return {
    ...actual,
    deployMatrix: vi.fn().mockResolvedValue(MATRIX),
    deploySetRepoInfo: vi.fn().mockResolvedValue(undefined),
    envBranchOf: vi.fn().mockResolvedValue("dev-202610"),
    openUrl: vi.fn(),
    copyText: vi.fn().mockResolvedValue(undefined),
  };
});

import * as api from "../bridge";
import RepoDetail from "./RepoDetail.vue";
import { useAppStore } from "../stores/app";

const flush = () => new Promise((r) => setTimeout(r, 0));

function mountDetail() {
  const pinia = createPinia();
  setActivePinia(pinia);
  const app = useAppStore();
  app.hub = { root: "/tmp/hub" } as never;
  // 需求列表：`usages` 给的是显示名，跳转时要靠它还原出真正的分支名
  app.workspaces = [
    { branch: "feature-20261013-demo", title: "示例需求" } as never,
  ];
  return mount(
    {
      render: () =>
        h(NMessageProvider, null, { default: () => h(RepoDetail, { repo: REPO as never }) }),
    },
    { global: { plugins: [pinia], stubs: { teleport: true } } },
  );
}

const btn = (w: ReturnType<typeof mountDetail>, label: string) =>
  w.findAll("button").find((b) => b.text().trim() === label);

describe("仓库详情 · 自定义信息的编辑", () => {
  it("点「取消」要把改动丢掉，再点「修改」看到的还是已保存的值", async () => {
    const w = mountDetail();
    await flush();
    await w.vm.$nextTick();

    expect(w.text(), "先显示已保存的备注").toContain("已保存的备注");

    await btn(w, "修改")!.trigger("click");
    const area = w.find("textarea");
    await area.setValue("改了一半又不想改了");

    await btn(w, "取消")!.trigger("click");
    await w.vm.$nextTick();

    expect(w.text(), "取消后应回到已保存的值").toContain("已保存的备注");
    expect(w.text(), "取消后不该留着半截改动").not.toContain("改了一半又不想改了");

    // 再进编辑：草稿要从**已保存的**值重新复制，而不是上次改动残留
    await btn(w, "修改")!.trigger("click");
    await w.vm.$nextTick();
    expect(
      (w.find("textarea").element as HTMLTextAreaElement).value,
      "重开编辑应是已保存的备注",
    ).toBe("已保存的备注");
  });

  it("「保存」才真的写下去（取消与保存不是一回事）", async () => {
    const w = mountDetail();
    await flush();
    await w.vm.$nextTick();

    await btn(w, "修改")!.trigger("click");
    await w.find("textarea").setValue("这次要保存");
    await btn(w, "保存")!.trigger("click");
    await flush();
    await w.vm.$nextTick();

    const arg = (api.deploySetRepoInfo as ReturnType<typeof vi.fn>).mock.calls[0]?.[0] as {
      notes: string;
    };
    expect(arg.notes, "保存时写的应是草稿里的内容").toBe("这次要保存");
  });
});

describe("仓库详情 · 被谁用着", () => {
  it("需求与环境分开显示，环境那栏是标识（不再带 envs/ 前缀）", async () => {
    const w = mountDetail();
    await flush();
    await w.vm.$nextTick();

    expect(w.text(), "需求引用要显示「需求名（分支）」").toContain("示例需求（feature-20261013-demo）");
    expect(w.text(), "环境标识照原样显示").toContain("dev");
    expect(w.text(), "不该再把 envs/ 这种路径前缀露出来").not.toContain("envs/dev");
  });

  it("脚注说的是「被需求用着才拦」，不是「环境引用也会拒绝」", async () => {
    // 实测反馈：环境里也能删（需求没引用的话）—— 旧文案是错的。
    const w = mountDetail();
    await flush();
    await w.vm.$nextTick();

    expect(w.text()).toContain("删除仓库时若还被某个需求用着会被拒绝");
    expect(w.text()).toContain("环境里的检出会跟着一起清掉");
  });

  it("点需求那条要跳到**分支名**，不是把「需求名（分支名）」整串当分支发出去", async () => {
    // 后端给的是显示名「示例需求（feature-…）」；第一版直接把它当分支名 emit，
    // 真跳过去时 Workspace::open 会报「分支不存在」。
    const w = mountDetail();
    await flush();
    await w.vm.$nextTick();

    const chip = w.findAll("button").find((b) => b.text().includes("示例需求（"));
    expect(chip, "该有需求那一条").toBeTruthy();
    expect((chip!.element as HTMLButtonElement).disabled, "核对得上就该能点").toBe(false);

    await chip!.trigger("click");
    // emit 挂在 RepoDetail 上（外层只是 Provider 壳），所以要从组件本身取
    expect(w.findComponent(RepoDetail).emitted("gotoWs")?.[0], "发出去的必须是纯分支名").toEqual([
      "feature-20261013-demo",
    ]);
  });

  it("分支名核对不上时不跳（不给一个点了必然报错的按钮）", async () => {
    const w = mountDetail();
    await flush();
    await w.vm.$nextTick();
    // 需求已被删掉/改名：本地列表里已经没有这条分支
    useAppStore().workspaces = [];
    await w.vm.$nextTick();

    const chip = w.findAll("button").find((b) => b.text().includes("示例需求（"));
    expect((chip!.element as HTMLButtonElement).disabled, "核对不上就该灰掉").toBe(true);
  });
});