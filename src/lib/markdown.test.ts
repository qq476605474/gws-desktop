// @vitest-environment jsdom
// DOMPurify 在 happy-dom 下清洗结果不正确（会放过 script / 事件属性），
// 这里单独用 jsdom；其它单测仍用更快的 happy-dom。
import { describe, expect, it } from "vitest";

import {
  decodeRef,
  isExternalLink,
  isMarkdownPath,
  renderMarkdown,
  resolveAssetPath,
} from "./markdown";

describe("isMarkdownPath", () => {
  it("识别 md / markdown 扩展名", () => {
    expect(isMarkdownPath("需求文档/需求文档.md")).toBe(true);
    expect(isMarkdownPath("README.MD")).toBe(true);
    expect(isMarkdownPath("x.markdown")).toBe(true);
  });

  it("普通文件与空值不算", () => {
    expect(isMarkdownPath("main.rs")).toBe(false);
    expect(isMarkdownPath("a.md.txt")).toBe(false);
    expect(isMarkdownPath(null)).toBe(false);
  });
});

describe("renderMarkdown", () => {
  it("把 mermaid 代码块抽成占位符", () => {
    const { html, mermaid } = renderMarkdown(
      "# 标题\n\n```mermaid\ngraph TD\nA-->B\n```\n\n正文\n",
    );
    expect(mermaid).toEqual([{ id: "mb0", code: "graph TD\nA-->B" }]);
    expect(html).toContain('data-mermaid-id="mb0"');
    expect(html).not.toContain("graph TD");
    expect(html).toContain("<h1");
    expect(html).toContain("正文");
  });

  it("多个 mermaid 块各自编号", () => {
    const { mermaid } = renderMarkdown(
      "```mermaid\ngraph TD\nA-->B\n```\n\n```mermaid\nsequenceDiagram\nA->>B: hi\n```\n",
    );
    expect(mermaid.map((m) => m.id)).toEqual(["mb0", "mb1"]);
    expect(mermaid[1].code).toContain("sequenceDiagram");
  });

  it("清掉脚本和事件属性", () => {
    const { html } = renderMarkdown(
      '# hi\n\n<script>alert(1)</script>\n\n<img src="x" onerror="alert(1)">\n',
    );
    expect(html).not.toContain("<script");
    expect(html.toLowerCase()).not.toContain("onerror");
  });

  it("渲染 GFM 表格", () => {
    const { html } = renderMarkdown("| 目录 | 放什么 |\n|---|---|\n| `需求文档/` | 原型 |\n");
    expect(html).toContain("<table");
    expect(html).toContain("<th>目录</th>");
  });
});

describe("resolveAssetPath", () => {
  it("相对当前 md 所在目录解析", () => {
    expect(resolveAssetPath("docs/tech/a.md", "img/x.png")).toBe("docs/tech/img/x.png");
    expect(resolveAssetPath("docs/tech/a.md", "./img/x.png")).toBe("docs/tech/img/x.png");
    expect(resolveAssetPath("docs/tech/a.md", "../img/x.png")).toBe("docs/img/x.png");
  });

  it("去掉查询串和锚点", () => {
    expect(resolveAssetPath("a.md", "img/x.png?raw=1#frag")).toBe("img/x.png");
  });

  it("越界、绝对路径和外链不处理", () => {
    expect(resolveAssetPath("a.md", "../x.png")).toBe("");
    expect(resolveAssetPath("a.md", "/abs/x.png")).toBe("");
    expect(resolveAssetPath("a.md", "https://x/y.png")).toBe("");
    expect(resolveAssetPath("a.md", "data:image/png;base64,xx")).toBe("");
  });
});

describe("decodeRef / isExternalLink", () => {
  it("解码中文文件名", () => {
    expect(decodeRef("%E4%B8%AD%E6%96%87.png")).toBe("中文.png");
  });

  it("只在 http / mailto 时算外链", () => {
    expect(isExternalLink("https://git.example.com")).toBe(true);
    expect(isExternalLink("mailto:a@b.com")).toBe(true);
    expect(isExternalLink("./a.md")).toBe(false);
    expect(isExternalLink("#section")).toBe(false);
  });
});