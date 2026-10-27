// @vitest-environment happy-dom
//! 桥接层的回归测试。
//!
//! 钉住的是一类**具体踩过的坑**：打开目录 / 文件 / 链接 / 终端这几个命令
//! 「成功了界面不变」，一旦失败就什么都没发生 —— 实测反馈就是
//! 「环境页点『文件管理』没有反应」。所以这里必须证明：
//! **失败会弹提示，而且不会变成 unhandledrejection**。

import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}));

import { errText, openInFinder, openInTerminal, openPath, openUrl } from "./bridge";
import { installNotifier } from "./lib/notify";

/** 收提示用的替身。 */
function captureMessages() {
  const shown: string[] = [];
  installNotifier({
    error: (t) => shown.push(`error:${t}`),
    warning: (t) => shown.push(`warning:${t}`),
    success: (t) => shown.push(`success:${t}`),
  });
  return shown;
}

beforeEach(() => {
  invoke.mockReset();
});

describe("打开类命令的失败必须看得见", () => {
  // 后端抛的是 `CommandError` 对象（不是 Error 实例），最容易漏的一处
  const missingDir = { message: "目录不存在：/hub/envs/dev/xxx（可能还没检出到这里）" };

  it("open_in_finder 失败时弹提示，且 Promise 不 reject", async () => {
    const shown = captureMessages();
    invoke.mockRejectedValueOnce(missingDir);

    // 调用点在模板里（`@click="openInFinder(...)"`），没有 catch 的地方 ——
    // 一旦 reject 就成 unhandledrejection，界面上依旧什么都没发生
    await expect(openInFinder("/hub/envs/dev/xxx")).resolves.toBeUndefined();
    expect(shown).toEqual([`error:${missingDir.message}`]);
  });

  it("open_path / open_url / open_in_terminal 同样弹提示", async () => {
    for (const call of [
      () => openPath("/nope"),
      () => openUrl("https://gitlab.example.com/x"),
      () => openInTerminal("/nope", "system"),
    ]) {
      const shown = captureMessages();
      invoke.mockRejectedValueOnce({ message: "打不开" });
      await call();
      expect(shown).toEqual(["error:打不开"]);
    }
  });

  it("成功时不弹任何东西（这类操作成功本来就没有可见结果）", async () => {
    const shown = captureMessages();
    invoke.mockResolvedValueOnce(undefined);

    await openInFinder("/hub/envs/dev/xxx");
    expect(shown).toEqual([]);
    expect(invoke).toHaveBeenCalledWith("open_in_finder", {
      path: "/hub/envs/dev/xxx",
    });
  });

  it("后端返回的不是字符串而是对象时，errText 取得到 message", () => {
    expect(errText({ message: "目录不存在：/x" })).toBe("目录不存在：/x");
    expect(errText(new Error("boom"))).toBe("boom");
    expect(errText("plain")).toBe("plain");
  });
});