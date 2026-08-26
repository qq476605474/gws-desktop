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
  it("span 内 HTML 载荷：转义且非徽标多字符后空格保留", () => {
    // `<script>&` 是多字符内容，不满足徽标条件，空格保留（不再吞掉）
    expect(ansiToHtml("\u001b[31m<script>&\u001b[0m x")).toBe(
      '<span class="c31">&lt;script&gt;&amp;</span> x'
    );
  });
  it("多字符 span 后的空格保留（gws drop 场景）", () => {
    expect(ansiToHtml("\u001b[34mgws drop\u001b[0m 清理")).toBe(
      '<span class="c34">gws drop</span> 清理'
    );
  });
  it("孤儿重置码不吞空格", () => {
    expect(ansiToHtml("x\u001b[0m y")).toBe("x y");
  });
});
