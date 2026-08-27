import { beforeEach, describe, expect, it, vi } from "vitest";

// busy 测试须走真实 gws-bridge（busy 包裹在其 runGws 里），只 mock 底层 invoke
const mocks = vi.hoisted(() => ({
  invoke: vi.fn<(cmd: string, args: Record<string, unknown>) => Promise<unknown>>(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

import { runGws } from "./gws-bridge";
import { busyCount } from "./busy";

type RunResult = { code: number | null; output: string };

beforeEach(() => {
  busyCount.value = 0;
  vi.clearAllMocks();
});

describe("runGws 的 busy 包裹", () => {
  it("在途 busyCount=1，resolve 后归零", async () => {
    let release!: (v: RunResult) => void;
    mocks.invoke.mockImplementationOnce(() => new Promise<RunResult>((r) => (release = r)));

    const p = runGws(["ls"], "/hub");
    expect(busyCount.value).toBe(1); // invoke 未决：在途计数已 +1

    release({ code: 0, output: "" });
    await p;
    expect(busyCount.value).toBe(0);
  });

  it("reject 也归零（finally 保证）", async () => {
    mocks.invoke.mockRejectedValueOnce(new Error("invoke 失败"));
    await expect(runGws(["ls"], "/hub")).rejects.toThrow("invoke 失败");
    expect(busyCount.value).toBe(0);
  });

  it("并发多个在途按计数增减（如 hub.refreshAll 三条 ls 并行）", async () => {
    const releases: Array<(v: RunResult) => void> = [];
    mocks.invoke.mockImplementation(
      () => new Promise<RunResult>((r) => releases.push(r)),
    );

    const p1 = runGws(["ls"], "/hub");
    const p2 = runGws(["repo", "ls"], "/hub");
    const p3 = runGws(["env", "ls"], "/hub");
    expect(busyCount.value).toBe(3);

    releases[0]!({ code: 0, output: "" });
    await p1;
    expect(busyCount.value).toBe(2);

    releases[1]!({ code: 0, output: "" });
    releases[2]!({ code: 0, output: "" });
    await Promise.all([p2, p3]);
    expect(busyCount.value).toBe(0);
  });
});
