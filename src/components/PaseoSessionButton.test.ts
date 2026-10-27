// @vitest-environment happy-dom
//! 开 AI 会话按钮的接线测试。
//!
//! 两种模式共用同一个弹窗，只差有没有 `branch`：
//! - 有分支 → 需求会话（`paseo_open_session`），不显示「会话标题」；
//! - 没分支 → **自由会话**（`paseo_open_free_session`），标题可以自己填。
//!
//! 最要紧的是**别把两种请求接错**：接错了会在 Hub 根目录开一个标题是
//! 「临时会话」的标签页，或者反过来对着需求开会话却带着空分支。

import { mount } from "@vue/test-utils";
import { NMessageProvider } from "naive-ui";
import { createPinia, setActivePinia } from "pinia";
import { h } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(null) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

vi.mock("../bridge", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../bridge")>();
  return {
    ...actual,
    paseoProviders: vi.fn(),
    paseoOpenSession: vi.fn(),
    paseoOpenFreeSession: vi.fn(),
  };
});

import * as api from "../bridge";
import { useAppStore } from "../stores/app";
import PaseoSessionButton from "./PaseoSessionButton.vue";

const BRANCH = "bugfix-20261020-ca-mobile-vault";

const ok = {
  branch: "",
  cwd: "/hub",
  agent_id: "agt_1",
  opened: true,
  message: "已在 Paseo 打开会话 agt_1",
};

function mountButton(props: { label: string; branch?: string }) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useAppStore().hub = { root: "/hub" } as never;
  return mount(
    { render: () => h(NMessageProvider, null, { default: () => h(PaseoSessionButton, props) }) },
    { global: { plugins: [pinia], stubs: { teleport: true } } },
  );
}

const flush = () => new Promise((r) => setTimeout(r, 0));

async function openDialog(w: ReturnType<typeof mountButton>) {
  await w.find("button").trigger("click");
  await flush();
  await w.vm.$nextTick();
}

async function start(w: ReturnType<typeof mountButton>) {
  const btn = w.findAll("button").find((b) => b.text() === "启动")!;
  await btn.trigger("click");
  await flush();
  await w.vm.$nextTick();
}

beforeEach(() => {
  vi.clearAllMocks();
  (api.paseoProviders as ReturnType<typeof vi.fn>).mockResolvedValue(["claude", "codex"]);
  (api.paseoOpenSession as ReturnType<typeof vi.fn>).mockResolvedValue({ ...ok, branch: BRANCH });
  (api.paseoOpenFreeSession as ReturnType<typeof vi.fn>).mockResolvedValue(ok);
});

describe("开 AI 会话", () => {
  it("自由会话：填了标题就带着标题、用第一个 provider 开", async () => {
    const w = mountButton({ label: "开 AI 会话" });
    await openDialog(w);

    expect(w.text()).toContain("会话标题");
    const input = w.find("input[placeholder]");
    await input.setValue("查线上日志");

    await start(w);

    expect((api.paseoOpenFreeSession as ReturnType<typeof vi.fn>).mock.calls[0]).toEqual([
      "查线上日志",
      "claude",
    ]);
    expect((api.paseoOpenSession as ReturnType<typeof vi.fn>).mock.calls.length).toBe(0);
  });

  it("自由会话：标题留空也照开（后端起名「临时会话」）", async () => {
    const w = mountButton({ label: "开 AI 会话" });
    await openDialog(w);
    await start(w);

    expect((api.paseoOpenFreeSession as ReturnType<typeof vi.fn>).mock.calls[0]).toEqual([
      "",
      "claude",
    ]);
  });

  it("需求会话：带分支开，且没有「会话标题」", async () => {
    const w = mountButton({ label: "派发Paseo任务", branch: BRANCH });
    await openDialog(w);

    expect(w.text()).not.toContain("会话标题");

    await start(w);

    expect((api.paseoOpenSession as ReturnType<typeof vi.fn>).mock.calls[0]).toEqual([
      BRANCH,
      "claude",
    ]);
    expect((api.paseoOpenFreeSession as ReturnType<typeof vi.fn>).mock.calls.length).toBe(0);
  });

  it("没有可用 provider 时：启动按钮禁用，并说明去哪儿查", async () => {
    (api.paseoProviders as ReturnType<typeof vi.fn>).mockResolvedValue([]);
    const w = mountButton({ label: "开 AI 会话" });
    await openDialog(w);

    expect(w.text()).toContain("没有可用的 provider");
    expect(w.text()).toContain("paseo provider ls");
    const btn = w.findAll("button").find((b) => b.text() === "启动")!;
    expect(btn.attributes("disabled")).toBeDefined();
    expect((api.paseoOpenFreeSession as ReturnType<typeof vi.fn>).mock.calls.length).toBe(0);
  });

  it("paseo 没装好时：弹窗里保留真实错误，而不是只说没 provider", async () => {
    // 实测反馈：Windows 上只看到「没有可用的 provider」，看不出是没找到 paseo。
    (api.paseoProviders as ReturnType<typeof vi.fn>).mockRejectedValue(
      "没有找到 paseo。请先安装 Paseo 桌面端，再在「集成」→「CLI 文档」→「命令行」里安装 CLI",
    );
    const w = mountButton({ label: "开 AI 会话" });
    await openDialog(w);

    expect(w.text()).toContain("没有找到 paseo");
    expect(w.text()).toContain("集成");
    expect(w.text()).toContain("CLI 文档");
    expect(w.text()).not.toContain("没有可用的 provider");
  });
});
