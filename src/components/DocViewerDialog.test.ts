// @vitest-environment happy-dom
// 纯展示组件（无 store/桥依赖）：createApp().mount() 直挂，props + close 探针即可
import { afterEach, describe, expect, it, vi } from "vitest";
import { createApp, type App } from "vue";
import DocViewerDialog from "./DocViewerDialog.vue";

let app: App | null = null;
let el: HTMLElement | null = null;

function mountViewer(onClose: () => void) {
  el = document.createElement("div");
  document.body.appendChild(el);
  app = createApp(DocViewerDialog, {
    fileName: "技术方案.md",
    path: "/hub/docs/2026-08-18-checkout-revamp/技术方案.md",
    content: "---\ntitle: 技术方案\n---\n\n# 标题\n\n- 要点一\n",
    onClose,
  });
  app.mount(el);
}

afterEach(() => {
  app?.unmount();
  app = null;
  el?.remove();
  el = null;
});

describe("DocViewerDialog 纯展示", () => {
  it("标题渲染文件名与路径（muted 小字），内容区 pre 保留换行与缩进", () => {
    mountViewer(vi.fn());

    expect(el!.textContent).toContain("技术方案.md");
    expect(el!.textContent).toContain("/hub/docs/2026-08-18-checkout-revamp/技术方案.md");
    const pre = el!.querySelector("pre")!;
    expect(pre).toBeTruthy();
    // 换行与缩进原样保留（white-space: pre-wrap），不被折叠成单行
    expect(pre.textContent).toContain("---\ntitle: 技术方案\n---");
    expect(pre.textContent).toContain("\n\n# 标题\n\n- 要点一\n");
  });

  it("关闭：点 mask 与关闭按钮均 emit close；点 dialog 本体不关闭（click.self）", () => {
    const closed = vi.fn();
    mountViewer(closed);

    el!.querySelector<HTMLElement>(".dialog")!.click(); // 冒泡到 mask 但 target 非本体
    expect(closed).not.toHaveBeenCalled();

    el!.querySelector<HTMLElement>(".mask")!.click();
    expect(closed).toHaveBeenCalledTimes(1);

    const btn = Array.from(el!.querySelectorAll<HTMLButtonElement>("button"))
      .find((b) => b.textContent?.trim() === "关闭");
    if (!btn) throw new Error("关闭按钮未找到");
    btn.click();
    expect(closed).toHaveBeenCalledTimes(2);
  });
});
