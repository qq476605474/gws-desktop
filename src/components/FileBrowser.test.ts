// @vitest-environment happy-dom
//! 「对比代码」的挂载测试 —— 钉住那颗**最贵的雷**：请求根本没发出去。
//!
//! 实测反馈：选「与主干」永远是「差异 (0)」，本地明明改了一堆。
//! 根因是「与主干」的基准值是 `null`（意思是"后端自己探测主干"），
//! 而加载逻辑把 `null` 当成"目标没选全"→ **一个请求都不发**。
//!
//! 规则本身在 `lib/compareBase.ts` 有单测；这里钉的是**接线**：
//! 选「与主干」时，`browseChanges` **必须真的被调用**（而且 base 传 null）。
//!
//! 另外钉住默认目标：「与本地」= HEAD（未提交的改动）。需求分支还没推送时
//! `origin/<分支>` 不存在，「与远程」只剩一句报错 —— 那正是实测反馈里
//! 「本地明明改了东西却识别不出来」的原因。

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
    browseChanges: vi.fn().mockResolvedValue([]),
    browseCommits: vi.fn().mockResolvedValue([]),
    browseAheadBehind: vi.fn().mockResolvedValue([0, 0]),
    listEntries: vi.fn().mockResolvedValue([]),
    wsBaseline: vi.fn().mockResolvedValue({
      reference: "origin/feature-20261001-upstream",
      missed: [],
      fell_back_to_main: false,
    }),
  };
});

import * as api from "../bridge";
import FileBrowser from "./FileBrowser.vue";

function mountBrowser(props: Record<string, unknown>) {
  const pinia = createPinia();
  setActivePinia(pinia);
  return mount(
    {
      render: () =>
        h(NMessageProvider, null, {
          default: () =>
            h(FileBrowser, {
              dir: "/hub/ws/feature-x/demo-api",
              repo: "demo-api",
              branch: "feature-x",
              label: "demo-api",
              defaultMode: "changed",
              ...props,
            }),
        }),
    },
    { global: { plugins: [pinia], stubs: { teleport: true } } },
  );
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe("对比代码 · 与主干", () => {
  it("必须真的发请求（base 传 null = 让后端探测主干），而不是当成「没选全」跳过", async () => {
    const w = mountBrowser({});
    await flush();
    await w.vm.$nextTick();

    // 切到「与主干」
    const main = w.findAll("button").find((b) => b.text().includes("与主干"))!;
    expect(main, "该有「与主干」这个选项").toBeTruthy();
    await main.trigger("click");
    await flush();
    await flush();

    const calls = (api.browseChanges as ReturnType<typeof vi.fn>).mock.calls;
    expect(calls.length, "「与主干」也要发请求 —— 这就是那个 bug").toBeGreaterThan(0);
    const last = calls[calls.length - 1];
    expect(last![0]).toBe("/hub/ws/feature-x/demo-api");
    expect(last![1]).toBe("demo-api");
    expect(last![2]).toBeNull(); // null = 后端自己探测主干
    // 领先/落后也要算（后端探测主干）
    expect((api.browseAheadBehind as ReturnType<typeof vi.fn>).mock.calls.length).toBeGreaterThan(0);
  });

  it("与基线：先让后端解析 ref，再拿这个 ref 去列改动", async () => {
    const w = mountBrowser({ wsBranch: "feature-x" });
    await flush();
    (api.browseChanges as ReturnType<typeof vi.fn>).mockClear();

    const baseline = w.findAll("button").find((b) => b.text().includes("与基线"))!;
    expect(baseline, "需求上下文里该有「与基线」").toBeTruthy();
    await baseline.trigger("click");
    await flush();
    await flush();

    expect(api.wsBaseline).toHaveBeenCalledWith("feature-x", "demo-api");
    const calls = (api.browseChanges as ReturnType<typeof vi.fn>).mock.calls;
    expect(calls.length, "基线解析完必须真的去列改动").toBeGreaterThan(0);
    expect(calls[calls.length - 1]![2]).toBe("origin/feature-20261001-upstream");
  });

  it("没有需求上下文（比如仓库页）→ 不显示「与基线」", async () => {
    const w = mountBrowser({});
    await flush();
    expect(w.findAll("button").some((b) => b.text().includes("与基线"))).toBe(false);
  });

  it("目标没选全（与环境但没选环境）→ **不发**请求，免得把结果挂在错的标题下", async () => {
    const w = mountBrowser({});
    await flush();
    (api.browseChanges as ReturnType<typeof vi.fn>).mockClear();

    const env = w.findAll("button").find((b) => b.text().includes("与环境"))!;
    await env.trigger("click");
    await flush();
    await flush();

    expect((api.browseChanges as ReturnType<typeof vi.fn>).mock.calls.length).toBe(0);
  });
});

describe("对比代码 · 与本地（默认）", () => {
  it("挂载就用 HEAD 列改动（分支没推送也能看），按钮排在「与远程」左边", async () => {
    (api.browseChanges as ReturnType<typeof vi.fn>).mockClear();
    const w = mountBrowser({});
    await flush();
    await flush();

    const calls = (api.browseChanges as ReturnType<typeof vi.fn>).mock.calls;
    expect(calls.length, "默认目标也要真的发请求").toBeGreaterThan(0);
    expect(calls[calls.length - 1]![2]).toBe("HEAD");

    const labels = w.findAll(".target button").map((b) => b.text());
    const local = labels.indexOf("与本地");
    const remote = labels.indexOf("与远程");
    expect(local, "该有「与本地」这个选项").toBeGreaterThanOrEqual(0);
    expect(local, "「与本地」要在「与远程」左边").toBeLessThan(remote);
  });
});