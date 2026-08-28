import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { confirmBox, answerConfirm, confirmState } from "./confirm";

beforeEach(() => {
  confirmState.pending = null;
});

afterEach(() => {
  confirmState.pending = null;
});

describe("confirmBox", () => {
  it("挂起至 answerConfirm：确认 → resolve(true)", async () => {
    const p = confirmBox("移除环境 dev？");
    await vi.waitFor(() => expect(confirmState.pending?.message).toBe("移除环境 dev？"));
    answerConfirm(true);
    await expect(p).resolves.toBe(true);
    expect(confirmState.pending).toBeNull();
  });

  it("取消 → resolve(false)，状态复位", async () => {
    const p = confirmBox("继续吗？");
    await vi.waitFor(() => expect(confirmState.pending).toBeTruthy());
    answerConfirm(false);
    await expect(p).resolves.toBe(false);
    expect(confirmState.pending).toBeNull();
  });

  it("并发防御：已有挂起问题再问 → 直接按取消（false），原问题不受影响", async () => {
    const first = confirmBox("第一个问题");
    await vi.waitFor(() => expect(confirmState.pending).toBeTruthy());
    const second = confirmBox("第二个问题");
    await expect(second).resolves.toBe(false); // 不覆盖首问
    expect(confirmState.pending?.message).toBe("第一个问题");
    answerConfirm(true);
    await expect(first).resolves.toBe(true);
  });
});
