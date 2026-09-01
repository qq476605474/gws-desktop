import { describe, expect, it } from "vitest";
import { joinPath, normalizePath } from "./path";

describe("normalizePath", () => {
  it("unix 路径仅去尾斜杠", () => {
    expect(normalizePath("/Users/foo/dev/")).toBe("/Users/foo/dev");
  });
  it("Windows 混合分隔符压成单反斜杠（用户设置实际出现的形态）", () => {
    expect(normalizePath("C:\\tools\\gws\\/test-hub2")).toBe("C:\\tools\\gws\\test-hub2");
  });
  it("正斜杠 Windows 路径转反斜杠风格", () => {
    expect(normalizePath("C:/tools/gws/test-hub2")).toBe("C:\\tools\\gws\\test-hub2");
  });
  it("连续反斜杠压成一个", () => {
    expect(normalizePath("C:\\a\\\\b")).toBe("C:\\a\\b");
  });
  it("UNC 前缀保留", () => {
    expect(normalizePath("//server/share/dir")).toBe("\\\\server\\share\\dir");
  });
  it("尾部反斜杠去掉", () => {
    expect(normalizePath("C:\\tools\\gws\\")).toBe("C:\\tools\\gws");
  });
  it("空串与空白原样返回空", () => {
    expect(normalizePath("")).toBe("");
    expect(normalizePath("  ")).toBe("");
  });
});

describe("joinPath", () => {
  it("unix 风格 base 用 / 拼接", () => {
    expect(joinPath("/hub", "ws", "feat-x")).toBe("/hub/ws/feat-x");
  });
  it("Windows 风格 base 用 \\ 拼接", () => {
    expect(joinPath("C:\\tools\\gws\\test-hub2", "ws", "feat-x")).toBe("C:\\tools\\gws\\test-hub2\\ws\\feat-x");
  });
  it("base 尾部多余分隔符先清掉", () => {
    expect(joinPath("/hub/", "ws")).toBe("/hub/ws");
    expect(joinPath("C:\\hub\\", "ws")).toBe("C:\\hub\\ws");
  });
  it("段首尾分隔符剥离，空段跳过", () => {
    expect(joinPath("/hub", "/ws/", "x")).toBe("/hub/ws/x");
    expect(joinPath("/hub", "", "ws")).toBe("/hub/ws");
  });
});
