// @vitest-environment happy-dom
//! 「提 MR」标题/描述可编辑的接线测试。
//!
//! 实测反馈 #11：默认标题是后端拼的，界面上却看不到也改不了。
//! 这里钉住三件事：打开时用 `mr_defaults` 预填、编辑后原样提交、
//! 预览回来的最终值**不覆盖**用户已经改过的内容。

import { mount } from "@vue/test-utils";
import { NDialogProvider, NMessageProvider } from "naive-ui";
import { h } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(null) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

// `vi.mock` 的工厂会被提升，里面用到的常量必须一起 `vi.hoisted` 提上去
const { BRANCH, DEFAULT_TITLE, DEFAULT_DESC, fakePlan } = vi.hoisted(() => {
  const BRANCH = "feature-20261013-private-agent-v3";
  const DEFAULT_TITLE = "需求上线-示例需求v1.3";
  const DEFAULT_DESC = "## 需求\n\n默认描述";
  function fakePlan(over: Record<string, unknown> = {}) {
    return {
      branch: BRANCH,
      title: DEFAULT_TITLE,
      description: DEFAULT_DESC,
      reviewers: { frontend: "alice", backend: "bob", source: "全局配置" },
      rows: [],
      skipped: [],
      registered: {},
      is_dry_run: true,
      ...over,
    };
  }
  return { BRANCH, DEFAULT_TITLE, DEFAULT_DESC, fakePlan };
});

vi.mock("../bridge", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../bridge")>();
  return {
    ...actual,
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
    mrDefaults: vi.fn().mockResolvedValue({ title: DEFAULT_TITLE, description: DEFAULT_DESC }),
    mrPlan: vi.fn().mockResolvedValue(fakePlan({ title: "后端最终标题" })),
    mrCreate: vi.fn().mockResolvedValue(fakePlan({ title: "后端创建标题" })),
    mrSaveReviewers: vi.fn().mockResolvedValue(null),
  };
});

import MrPanel from "./MrPanel.vue";
import * as api from "../bridge";

const mods = [
  {
    module: "demo-api",
    present: true,
    dirty: 0,
    pushed: true,
    ahead: 0,
    behind: 0,
    ahead_of_main: 1,
    error: null,
  },
];

function mountPanel() {
  return mount(
    {
      render: () =>
        h(NMessageProvider, null, {
          default: () =>
            h(NDialogProvider, null, {
              default: () => h(MrPanel, { branch: BRANCH, modules: mods }),
            }),
        }),
    },
    { global: { stubs: { teleport: true } } },
  );
}

const flush = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => vi.clearAllMocks());

describe("提 MR 的标题与描述", () => {
  it("打开时用 mr_defaults 预填，而不是空白", async () => {
    const w = mountPanel();
    await flush();
    await w.vm.$nextTick();

    const title = w.find('input[placeholder^="留空则用默认 需求上线"]');
    const desc = w.find('textarea[placeholder="留空则用默认描述"]');
    expect(title.exists(), "该有标题输入框").toBe(true);
    expect((title.element as HTMLInputElement).value).toBe(DEFAULT_TITLE);
    expect((desc.element as HTMLTextAreaElement).value).toBe(DEFAULT_DESC);
  });

  it("改过的标题/描述原样传给 mrCreate（不再二次加工）", async () => {
    const w = mountPanel();
    await flush();
    await w.vm.$nextTick();

    await w.find('input[placeholder^="留空则用默认 需求上线"]').setValue("自定义标题");
    await w.find('textarea[placeholder="留空则用默认描述"]').setValue("自定义描述");

    const create = w.findAll("button").find((b) => b.text().includes("创建 MR"))!;
    await create.trigger("click");
    await flush();

    const arg = (api.mrCreate as ReturnType<typeof vi.fn>).mock.calls[0]![0];
    expect(arg.title).toBe("自定义标题");
    expect(arg.description).toBe("自定义描述");
  });

  it("没改时传 null（后端用默认），预览回来的最终值回填", async () => {
    const w = mountPanel();
    await flush();
    await w.vm.$nextTick();

    const preview = w.findAll("button").find((b) => b.text().includes("预览"))!;
    await preview.trigger("click");
    await flush();

    const arg = (api.mrPlan as ReturnType<typeof vi.fn>).mock.calls[0]![0];
    expect(arg.title).toBeNull();
    expect(arg.description).toBeNull();
    // 没改过 → 用后端返回的最终值回填
    const title = w.find('input[placeholder^="留空则用默认 需求上线"]');
    expect((title.element as HTMLInputElement).value).toBe("后端最终标题");
  });

  it("改了之后，预览返回的最终值**不覆盖**用户输入", async () => {
    const w = mountPanel();
    await flush();
    await w.vm.$nextTick();

    const title = w.find('input[placeholder^="留空则用默认 需求上线"]');
    await title.setValue("我的标题");
    await w.findAll("button").find((b) => b.text().includes("预览"))!.trigger("click");
    await flush();

    expect((title.element as HTMLInputElement).value).toBe("我的标题");
    const arg = (api.mrPlan as ReturnType<typeof vi.fn>).mock.calls[0]![0];
    expect(arg.title).toBe("我的标题");
  });
});
