// @vitest-environment happy-dom
import { mount } from "@vue/test-utils";
import { describe, expect, it, vi } from "vitest";

import HelpView from "./HelpView.vue";

describe("使用帮助", () => {
  it("把日常流程和常见问题都摆出来（装好后不用再去仓库翻文档）", () => {
    const w = mount(HelpView);
    const text = w.text();
    for (const key of [
      "新建 / 导入需求",
      "需求列表与需求详情",
      "合并到环境与合并检查",
      "对比代码与浏览文件",
      "提 MR",
      "快捷入口（自己配）",
      "基线",
      "需求文档",
      "派发Paseo任务",
      "开 AI 会话",
      "更新",
      "GitLab 令牌去哪儿申请",
      "为什么一直显示「未推送」",
    ]) {
      expect(text, `缺少「${key}」`).toContain(key);
    }
  });

  it("有目录：每一条都落到真实小节上", () => {
    const w = mount(HelpView);
    const links = w.findAll(".toc a");
    expect(links.length, "目录要够详细").toBeGreaterThanOrEqual(14);
    for (const a of links) {
      const id = a.attributes("href")!.slice(1);
      expect(w.find(`#${id}`).exists(), `缺少小节 ${id}`).toBe(true);
    }
  });

  it("点目录会滚到对应小节", async () => {
    const spy = vi.fn();
    Element.prototype.scrollIntoView = spy as never;
    const w = mount(HelpView, { attachTo: document.body });
    await w.findAll(".toc a")[0]!.trigger("click");
    expect(spy).toHaveBeenCalledTimes(1);
    w.unmount();
  });

  it("说明 Agent 在外部改完为什么不自动冒出来（回前台刷新）", () => {
    const text = mount(HelpView).text();
    expect(text).toContain("切回来时会自动刷新");
    expect(text, "窗口没失焦时要有手动入口").toContain("右上角的「刷新」");
  });

  it("细节要写清楚：对比目标的默认、快捷入口要自己配、MR 标题可改", () => {
    const text = mount(HelpView).text();
    expect(text).toContain("默认「与本地」");
    expect(text, "要说清快捷入口是用户自己配的").toContain("设置 → 快捷入口");
    expect(text, "没配就隐藏，不留点了没用的入口").toContain("一条都不配");
    expect(text).toContain("需求上线-需求名");
  });

  it("复制目录类按钮统一叫「复制目录」，不再出现「本地路径」", () => {
    const text = mount(HelpView).text();
    expect(text).toContain("复制目录");
    expect(text, "同一个动作不能两个名字").not.toContain("本地路径");
  });

  it("打开系统文件管理器的按钮统一叫「打开目录」，不再出现「文件管理」", () => {
    const text = mount(HelpView).text();
    expect(text).toContain("打开目录");
    expect(text, "「文件管理」容易被理解成应用内的文件页").not.toContain("文件管理");
  });

  it("新建需求默认只拉文档，skill 也必须先问用户", () => {
    const text = mount(HelpView).text();
    expect(text, "模块默认要与界面一致").toContain("默认只勾文档库");
    expect(text, "AI skill 不允许臆造参数").toContain("所有字段先问用户");
    expect(text, "选填也要问，说明默认").toContain("不填用默认");
    expect(text, "开场说明不该罗列阅读清单").toContain("不罗列要读的文件");
  });
});
