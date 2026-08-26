import { describe, it, expect } from "vitest";
import { stripAnsi, ansiToHtml } from "./ansi";

describe("stripAnsi", () => {
  it("剥离颜色码", () => {
    expect(stripAnsi("\u001b[31m✗\u001b[0m 出错")).toBe("✗ 出错");
  });
  it("无颜色码原样返回", () => {
    expect(stripAnsi("gws 0.4.2")).toBe("gws 0.4.2");
  });
});

describe("ansiToHtml", () => {
  it("红色转 span.red", () => {
    expect(ansiToHtml("\u001b[31m✗\u001b[0m fail")).toBe('<span class="c31">✗</span>fail');
  });
  it("绿色 32", () => {
    expect(ansiToHtml("\u001b[32m✓ ok")).toBe('<span class="c32">✓ ok</span>');
  });
  it("暗色 2（重置前含内容）", () => {
    expect(ansiToHtml("a\u001b[2mb\u001b[0m")).toBe('a<span class="c2">b</span>');
  });
  it("HTML 转义普通文本", () => {
    expect(ansiToHtml("a<b>&c")).toBe("a&lt;b&gt;&amp;c");
  });
});
