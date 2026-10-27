// @vitest-environment happy-dom
//! 「提 MR」弹窗里**能不能选到 doc-hub** 的挂载测试。
//!
//! # 为什么值得专门挂一次
//!
//! 实测反馈：提 MR 时列表里根本没有 doc-hub，于是**少提了一个** MR。
//! 根因有两层，一层在 core（`mr::selected_modules` 用错了候选集，Rust 那边
//! 已用 `dochub.rs` 的集成测试钉住），另一层就在这个组件里：
//! 详情页有两份模块列表 —— `modules` 把文档库滤掉了（界面上它只出现在
//! 「文档」那一行，不能出现两次），`opModules` 才带着它。**提 MR 传错了那份**。
//!
//! 这种「传错变量」的 bug 纯函数测试抓不到、类型检查也抓不到（两份都是
//! `ModuleStatus[]`），只能按用户的操作走一遍：打开提 MR → 看勾选列表里有没有它。

import { mount } from "@vue/test-utils";
import { NDialogProvider, NMessageProvider } from "naive-ui";
import { createPinia, setActivePinia } from "pinia";
import { h } from "vue";
import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(null) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

// `vi.mock` 的工厂会被提升到文件顶部，里面引用的顶层变量那时还没初始化 ——
// 所以夹具要用 `vi.hoisted` 一起提上去（vitest 的标准做法）。
const { BRANCH, FAKE_DETAIL, FAKE_SHELL } = vi.hoisted(() => {
  const BRANCH = "feature-20261013-private-agent-v3";

  /** 一份「挂了文档库」的需求详情：代码模块 1 个 + 文档库 1 个。 */
  const FAKE_DETAIL = {
    meta: {
      schema: 2,
      branch: BRANCH,
      title: "示例需求v1.3",
      prefix: "feature",
      created: "2026-10-14",
      base: [],
      base_resolved: {},
      modules: [{ name: "demo-api", present: true }],
      doc: { family: "示例需求", period: BRANCH },
      links: [],
      notes: "",
      merged: {},
      mr: {},
    },
    status: {
      branch: BRANCH,
      title: "示例需求v1.3",
      modules: [
        {
          module: "demo-api",
          present: true,
          dirty: 0,
          pushed: true,
          ahead: 0,
          behind: 0,
          ahead_of_main: 3,
          error: null,
        },
        // 文档库那条由 core 的 status() 一并给出（它不在 meta.modules 里）
        {
          module: "doc-hub",
          present: true,
          dirty: 0,
          pushed: true,
          ahead: 0,
          behind: 0,
          ahead_of_main: 4,
          error: null,
        },
      ],
    },
    doc: null,
  };

  /** 第一屏：只要元数据 + 模块名（不打 git）。 */
  const FAKE_SHELL = {
    meta: {
      ...FAKE_DETAIL.meta,
      notes: "这里是我的备注",
      links: [{ label: "需求地址", url: "https://example.com/x" }],
    },
    modules: [{ name: "demo-api", present: true }],
    doc: true,
  };

  return { BRANCH, FAKE_DETAIL, FAKE_SHELL };
});

vi.mock("../bridge", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../bridge")>();
  return {
    ...actual,
    wsDetail: vi.fn().mockResolvedValue(FAKE_DETAIL),
    wsShell: vi.fn().mockResolvedValue(FAKE_SHELL),
    wsProbeRemote: vi.fn().mockResolvedValue([]),
    deployMatrix: vi.fn().mockResolvedValue([]),
    openUrl: vi.fn(),
    // 让 MrPanel 走「已连通」那条分支 —— 没连通时它只显示一句提示，
    // 勾选列表根本不渲染（那样这个测试就白测了）
    gitlabStatus: vi.fn().mockResolvedValue({
      base_url: "https://gitlab.example.com",
      bound: true,
      has_hub: true,
      source: "配置",
      has_stored_token: true,
      store_path: "",
      user: { id: 1, username: "tester", name: "测试" },
      error: null,
    }),
    mrReviewers: vi.fn().mockResolvedValue({
      frontend: "alice",
      backend: "bob",
      source: "全局配置",
    }),
    // 「撤销改动 / 强制回退」的预览与执行：这两个操作要能**带上文档库**
    wsRevertPlan: vi.fn().mockResolvedValue({ modules: [], skipped: [] }),
    wsResetPlan: vi.fn().mockResolvedValue({ modules: [], skipped: [] }),
    // 「改分支名」的影响面预览
    branchRenamePlan: vi.fn().mockResolvedValue({
      old_branch: BRANCH,
      new_branch: "feature-20261013-renamed",
      modules: [
        {
          module: "demo-api",
          project: "group/demo-api",
          remote_branch: BRANCH,
          merged_to_main: false,
          local_unpushed: 0,
          exists: true,
        },
      ],
      problems: [],
      notice: [],
    }),
    // 开Paseo会话：先列 provider，选一个（默认第一个）再启动
    paseoProviders: vi.fn().mockResolvedValue(["claude", "qoder"]),
    paseoOpenSession: vi.fn().mockResolvedValue({
      branch: BRANCH,
      cwd: "/tmp/hub",
      agent_id: "agent-1",
      opened: true,
      message: "已在 Paseo 打开会话 agent-1",
    }),
  };
});

import WsDetail from "./WsDetail.vue";
import ModulePicker from "./ModulePicker.vue";
import * as api from "../bridge";
import { useAppStore } from "../stores/app";

/** 测试用的快捷入口 —— 开源版不预设，这些都由 hub 配置提供。 */
const RELEASE_LINKS = [
  { label: "发布系统", url: "https://deploy.example.com/tools" },
  { label: "监控大盘", url: "https://grafana.example.com/d/main" },
];

function mountDetail() {
  const pinia = createPinia();
  setActivePinia(pinia);
  const app = useAppStore();
  app.repos = [
    { name: "doc-hub", main_branch: "main", usages: [],
    env_usages: [], remote_url: "", is_doc: true },
    { name: "demo-api", main_branch: "main", usages: [],
    env_usages: [], remote_url: "", is_doc: false },
  ];
  // 开Paseo会话的弹窗要显示工作目录（hub 根）
  app.hub = {
    root: "/tmp/hub",
    name: "hub",
    envs: [],
    repo_count: 2,
    workspace_count: 1,
    dochub_name: "doc-hub",
    dochub_ready: true,
    dochub_branch: "main",
    branch_prefix: "feature",
    branch_date_digits: 8,
    migration_notes: [],
    quick_links: RELEASE_LINKS,
    git_name: "张三",
    git_email: "zhangsan@example.com",
  };
  return mount(
    {
      render: () =>
        h(NMessageProvider, null, {
          default: () => h(NDialogProvider, null, { default: () => h(WsDetail, { branch: BRANCH }) }),
        }),
    },
    { global: { plugins: [pinia], stubs: { teleport: true } } },
  );
}

/** 等 wsDetail 那份 promise 落地。 */
const flush = () => new Promise((r) => setTimeout(r, 0));

describe("提 MR 弹窗的模块勾选", () => {
  it("列表里要有 doc-hub（文档也要跟着提 MR）", async () => {
    const w = mountDetail();
    await flush();
    await w.vm.$nextTick();

    // 打开「提 MR」——MR 面板在弹窗里，`display-directive=if`，不打开就不会挂载
    const mrBtn = w.findAll("button").find((b) => b.text().includes("提 MR"));
    expect(mrBtn, "该有「提 MR」按钮").toBeTruthy();
    await mrBtn!.trigger("click");
    await w.vm.$nextTick();
    await flush();
    await w.vm.$nextTick();

    const picker = w.findComponent(ModulePicker);
    expect(picker.exists(), "提 MR 弹窗里应有模块勾选").toBe(true);
    const names = picker.props("modules").map((m: { module: string }) => m.module);
    expect(names).toContain("doc-hub");
    expect(names).toContain("demo-api");
  });

  it("「增加模块」的候选里不该再出现**已经拉下来的** doc-hub", async () => {
    // 实测反馈：文档库已经拉下来了，「增加模块」里却还列着它。
    // 根因还是 `modules`（不含文档库那份）与 `allModules`（含）混用 ——
    // 拿前者判断"在不在工作区里"必然漏判。
    const w = mountDetail();
    await flush();
    await w.vm.$nextTick();

    const add = w.findAll("button").find((b) => b.text().includes("增加模块"));
    expect(add, "该有「增加模块」按钮").toBeTruthy();
    await add!.trigger("click");
    await w.vm.$nextTick();

    const picker = w.findComponent(ModulePicker);
    expect(picker.exists()).toBe(true);
    const names = picker.props("modules").map((m: { module: string }) => m.module);
    expect(names).not.toContain("doc-hub");
  });

  it("「撤销改动」也要作用到 doc-hub（不能把它漏掉）", async () => {
    // 实测反馈：撤销改动 / 强制回退会把 doc-hub 过滤掉 —— 详情页里
    // `modules`（不含文档库）与 `opModules`（含）两份列表混用，
    // 这两个操作原来传的是**不含**文档库那份，于是文档的改动永远不会被撤销。
    const w = mountDetail();
    await flush();
    await w.vm.$nextTick();

    const btn = w.findAll("button").find((b) => b.text().includes("撤销改动"));
    expect(btn, "该有「撤销改动」按钮").toBeTruthy();
    await btn!.trigger("click");
    await w.vm.$nextTick();
    await flush();
    await w.vm.$nextTick();

    const only = (api.wsRevertPlan as ReturnType<typeof vi.fn>).mock.calls[0]?.[1] as {
      only: string[];
    };
    expect(only?.only, "预览时就要把文档库带上").toContain("doc-hub");
    expect(only.only).toContain("demo-api");
  });

  it("「强制回退」也要作用到 doc-hub", async () => {
    const w = mountDetail();
    await flush();
    await w.vm.$nextTick();

    const btn = w.findAll("button").find((b) => b.text().includes("强制回退"));
    expect(btn, "该有「强制回退」按钮").toBeTruthy();
    await btn!.trigger("click");
    await w.vm.$nextTick();
    await flush();
    await w.vm.$nextTick();

    const only = (api.wsResetPlan as ReturnType<typeof vi.fn>).mock.calls[0]?.[1] as {
      only: string[];
    };
    expect(only?.only, "预览时就要把文档库带上").toContain("doc-hub");
  });
});
describe("快捷入口", () => {
  it("hub 配了快捷入口 → 点「快捷入口」弹出这些入口，点一下真的跳", async () => {
    // 实测反馈：发版时要退回总览页点；需求里就该有入口。
    const w = mountDetail();
    await flush();
    await w.vm.$nextTick();

    const btn = w
      .findAll("button")
      .find((b) => b.text().trim() === "快捷入口");
    expect(btn, "该有「快捷入口」按钮").toBeTruthy();
    await btn!.trigger("click");
    await w.vm.$nextTick();

    const links = w.findAll(".pub-btns button");
    expect(links.length, "每个入口一个按钮").toBe(RELEASE_LINKS.length);
    expect(links.map((b) => b.text())).toEqual(RELEASE_LINKS.map((l) => l.label));

    await links[0]!.trigger("click");
    expect(api.openUrl).toHaveBeenCalledWith(RELEASE_LINKS[0]!.url);
    // 点完就关：打开新页面后弹窗不该还挡在前面（实测反馈）
    await w.vm.$nextTick();
    expect(w.findAll(".pub-btns").length, "点一个地址后弹窗应关闭").toBe(0);
  });

  it("hub 没配快捷入口 → 整个「快捷入口」按钮都不显示", async () => {
    // 开源版不预设任何地址：没配就隐藏，不留一个点了没用的入口。
    const w = mountDetail();
    await flush();
    await w.vm.$nextTick();

    const app = useAppStore();
    app.hub!.quick_links = [];
    await w.vm.$nextTick();

    const btn = w.findAll("button").find((b) => b.text().trim() === "快捷入口");
    expect(btn, "没配快捷入口时不该有「发布」按钮").toBeFalsy();
  });
});

describe("复制目录", () => {
  it("详情顶部的按钮就叫「复制目录」，不再叫「本地路径」", async () => {
    // 实测反馈：同一个动作，需求行叫「复制目录」、详情里叫「本地路径」，
    // 名字对不上，以为是两个功能。
    const w = mountDetail();
    await flush();
    await w.vm.$nextTick();

    const labels = w.findAll("button").map((b) => b.text().trim());
    expect(labels).toContain("复制目录");
    expect(labels).not.toContain("本地路径");
  });

  it("打开系统文件管理器的按钮叫「打开目录」，不再叫「文件管理」", async () => {
    // 「文件管理」容易被理解成应用内的文件页；这里打开的其实是系统文件管理器。
    const w = mountDetail();
    await flush();
    await w.vm.$nextTick();

    const labels = w.findAll("button").map((b) => b.text().trim());
    expect(labels, "顶部和文档/模块行都要有这个入口").toContain("打开目录");
    expect(labels, "同一个动作不能两个名字").not.toContain("文件管理");
  });
});

describe("派发Paseo任务", () => {
  it("按钮在需求详情顶部（不在模块操作行），先选 provider 再启动", async () => {
    const w = mountDetail();
    await flush();
    await w.vm.$nextTick();

    const btn = w.findAll("button").find((b) => b.text().includes("派发Paseo任务"));
    expect(btn, "该有「派发Paseo任务」按钮").toBeTruthy();
    // 它是需求级动作：放在头部，不跟模块的 git 操作挤在一排（实测反馈）
    expect(btn!.element.closest(".head"), "按钮应在头部").toBeTruthy();
    expect(btn!.element.closest(".ops"), "按钮不该在模块操作行").toBeFalsy();

    await btn!.trigger("click");
    // 等 `paseoProviders` 回来（弹窗先转圈，回来才列出单选）
    await flush();
    await w.vm.$nextTick();

    const text = w.text();
    expect(text, "弹窗里要列出可用 provider").toContain("claude");
    expect(text, "弹窗里要列出可用 provider").toContain("qoder");
    expect(text, "要说明工作目录是 hub 根").toContain("/tmp/hub");

    const ok = w.findAll("button").find((b) => b.text().trim() === "启动");
    expect(ok, "弹窗里该有「启动」").toBeTruthy();
    await ok!.trigger("click");
    await flush();
    await w.vm.$nextTick();

    // 默认选第一个（必选）；分支与 provider 一起传给后端
    expect(api.paseoOpenSession).toHaveBeenCalledWith(BRANCH, "claude");
  });

  it("provider 列表为空时「启动」不可点", async () => {
    (api.paseoProviders as ReturnType<typeof vi.fn>).mockResolvedValue([]);
    const w = mountDetail();
    await flush();
    await w.vm.$nextTick();

    const btn = w.findAll("button").find((b) => b.text().includes("派发Paseo任务"));
    await btn!.trigger("click");
    await flush();
    await w.vm.$nextTick();

    const ok = w.findAll("button").find((b) => b.text().trim() === "启动");
    expect(ok, "弹窗里该有「启动」").toBeTruthy();
    expect(ok!.attributes("disabled"), "没选 provider 时不能启动").toBeDefined();
    expect(w.text(), "要给出可照做的提示").toContain("paseo provider ls");
  });
});

describe("两段式加载：数字没回来之前也要有界面", () => {
  it("第一屏（元数据 + 模块名）到了就把模块行画出来，数字那三格显示「…」", async () => {
    // 实测反馈：「每次进去需求还是会转一会儿圈，卡住一会儿」——
    // 各模块那三列要起十几个 git 进程（Windows 上一次一两秒）。
    // 所以元数据先渲染，数字随后填；**期间不能显示 0**（0 会被读成真没改动）。
    (api.wsDetail as ReturnType<typeof vi.fn>).mockReturnValue(new Promise(() => {}));
    const w = mountDetail();
    await flush();
    await w.vm.$nextTick();

    // 表头与模块行都在（不用等数字）
    expect(w.text()).toContain("demo-api");
    // 备注与自定义信息也必须在第一屏（它们在 meta 里，本地随手就有）——
    // 带上 `detail` 做条件的话，这一块会晚一步冒出来把表格整个往下挤（实测反馈）
    expect(w.text(), "备注要在第一屏").toContain("这里是我的备注");
    expect(w.text(), "自定义信息要在第一屏").toContain("需求地址");
    expect(w.text(), "数字还没回来 → 显示「…」").toContain("…");
    // 操作按钮也该在第一屏就能用（存在与否是查目录得出的）
    const push = w.findAll("button").find((b) => b.text().includes("推送"));
    expect(push, "第一屏就该有操作按钮").toBeTruthy();
    expect((push!.element as HTMLButtonElement).disabled, "有模块就该能点").toBe(false);
  });
});

describe("改分支名", () => {
  /** 等防抖（400ms）过去、预览重算完。 */
  const settle = async (w: ReturnType<typeof mountDetail>) => {
    await new Promise((r) => setTimeout(r, 450));
    await w.vm.$nextTick();
    await flush();
    await w.vm.$nextTick();
  };

  const openRename = async (w: ReturnType<typeof mountDetail>) => {
    const btn = w.findAll("button").find((b) => b.text().trim() === "改分支名");
    expect(btn, "该有「改分支名」按钮").toBeTruthy();
    await btn!.trigger("click");
    return w.vm.$nextTick();
  };

  it("新分支名一改，影响面就跟着重算（不能停在「请先填入」）", async () => {
    // 实测反馈问到的正是这条路：预览是**挂载时**算一次的，而打开时输入框是空的，
    // 于是永远停在「请先填入新分支名」，人点「继续」时根本没看到会发生什么。
    const w = mountDetail();
    await flush();
    await w.vm.$nextTick();
    await openRename(w);
    await flush();
    await w.vm.$nextTick();

    expect(w.text(), "打开时先提示还没填").toContain("请先填入新分支名");

    const input = w.find(".n-modal input, input");
    await input.setValue("feature-20261013-renamed");
    await settle(w);

    expect(w.text(), "填完要算出真正的影响面").toContain("分支名:");
    expect(w.text(), "要列出会处理哪些模块").toContain("demo-api");
  });

  it("名字还是空的（或影响面还没重算完）时，不让点「继续」", async () => {
    const w = mountDetail();
    await flush();
    await w.vm.$nextTick();
    await openRename(w);
    await flush();
    await w.vm.$nextTick();

    const next = w.findAll("button").find((b) => b.text().trim() === "继续");
    expect(next, "预览那一步该有「继续」").toBeTruthy();
    expect(next!.attributes("disabled"), "没填新名字不能继续").toBeDefined();

    // 填上之后（等防抖算完）才放开
    await (w.find(".n-modal input, input")).setValue("feature-20261013-renamed");
    await settle(w);
    const next2 = w.findAll("button").find((b) => b.text().trim() === "继续");
    expect(next2!.attributes("disabled"), "影响面算完就该能继续").toBeUndefined();
  });

  it("关掉再打开，输入框是空的（不残留上次填的名字）", async () => {
    const w = mountDetail();
    await flush();
    await w.vm.$nextTick();
    await openRename(w);
    await flush();
    await w.vm.$nextTick();
    await (w.find(".n-modal input, input")).setValue("feature-20261013-renamed");
    await settle(w);

    const cancel = w.findAll("button").find((b) => b.text().trim() === "取消");
    await cancel!.trigger("click");
    await w.vm.$nextTick();
    await openRename(w);
    await flush();
    await w.vm.$nextTick();

    const input = w.find(".n-modal input, input");
    expect((input.element as HTMLInputElement).value, "重开应是干净的").toBe("");
    expect(w.text(), "预览也要回到「还没填」").toContain("请先填入新分支名");
  });
});

describe("回到前台刷新", () => {
  it("refreshTick 变了就重读详情；需求被改名（不在列表里）就退回列表", async () => {
    // Agent 里用 skill 改需求是**外部**改动，界面收不到通知；
    // 窗口重新聚焦时列表会刷新，详情页靠 refreshTick 跟着重读。
    (api.wsDetail as ReturnType<typeof vi.fn>).mockResolvedValue(FAKE_DETAIL);
    const w = mountDetail();
    await flush();
    await w.vm.$nextTick();

    const app = useAppStore();
    const before = (api.wsDetail as ReturnType<typeof vi.fn>).mock.calls.length;
    app.workspaces = [{ branch: BRANCH } as never];
    app.refreshTick += 1;
    await flush();
    await w.vm.$nextTick();

    expect(
      (api.wsDetail as ReturnType<typeof vi.fn>).mock.calls.length,
      "列表刷新后详情也要重读",
    ).toBeGreaterThan(before);
    expect(w.findComponent(WsDetail).emitted("close"), "还在列表里就别关").toBeFalsy();

    // 被改名：列表里换成新分支，旧分支已经不存在
    app.workspaces = [{ branch: "feature-renamed" } as never];
    app.refreshTick += 1;
    await flush();
    await w.vm.$nextTick();

    expect(w.findComponent(WsDetail).emitted("close"), "旧分支不在了要退回列表").toBeTruthy();
  });
});
