//! 常用编辑器列表的纯函数测试。
//!
//! 重点钉住「配置文件是用户能改的」这条边界：坏数据不能让界面渲染时抛错。

import { describe, expect, it } from "vitest";

import {
  commandForProgram,
  displayLabel,
  isUsable,
  normalizeEditors,
  programLabel,
  usableEditors,
  withCatalogLabels,
} from "./editors";

describe("常用编辑器列表", () => {
  it("自定义条目没填命令就不算可用", () => {
    expect(isUsable({ key: "a", id: "custom", label: "", command: "  " })).toBe(false);
    expect(isUsable({ key: "a", id: "custom", label: "", command: " zed {path} " })).toBe(true);
    expect(isUsable({ key: "a", id: "idea", label: "IDEA", command: "" })).toBe(true);
  });

  it("按配置顺序过滤，自定义空命令被跳过", () => {
    const list = [
      { key: "1", id: "zed", label: "Zed", command: "" },
      { key: "2", id: "custom", label: "自定义命令", command: "" },
      { key: "3", id: "custom", label: "自定义命令", command: "idea {path}" },
    ];
    expect(usableEditors(list).map((e) => e.key)).toEqual(["1", "3"]);
  });

  it("显示名：选程序时记住的名字优先，旧配置退回命令本身", () => {
    expect(
      displayLabel({ key: "a", id: "custom", label: "IntelliJ IDEA", command: 'open -a "x"' }),
    ).toBe("IntelliJ IDEA");
    expect(displayLabel({ key: "a", id: "custom", label: "", command: " zed {path} " })).toBe(
      "zed {path}",
    );
    expect(displayLabel({ key: "a", id: "custom", label: "自定义命令", command: "idea" })).toBe(
      "idea",
    );
    expect(displayLabel({ key: "a", id: "custom", label: "", command: "" })).toBe("未命名程序");
    expect(displayLabel({ key: "a", id: "idea", label: "IntelliJ IDEA", command: "" })).toBe(
      "IntelliJ IDEA",
    );
  });

  it("从程序路径取短名字", () => {
    expect(programLabel("/Applications/IntelliJ IDEA.app")).toBe("IntelliJ IDEA");
    expect(programLabel("/Applications/Zed.app/")).toBe("Zed");
    expect(programLabel("/usr/local/bin/idea")).toBe("idea");
    expect(programLabel(String.raw`C:\Program Files\JetBrains\IntelliJ IDEA\bin\idea64.exe`)).toBe(
      "idea",
    );
    expect(programLabel("/opt/Rebased-x86_64.AppImage")).toBe("Rebased-x86_64");
  });

  it("选中的程序转成可执行命令：.app 走 open -a，其余用引号包路径", () => {
    expect(commandForProgram("/Applications/IntelliJ IDEA.app")).toBe(
      'open -a "/Applications/IntelliJ IDEA.app"',
    );
    expect(commandForProgram(String.raw`C:\Program Files\Zed\zed.exe`)).toBe(
      String.raw`"C:\Program Files\Zed\zed.exe"`,
    );
  });

  it("坏数据被丢掉；缺 key / 重复 key 会补新的", () => {
    const out = normalizeEditors([
      null,
      "nope",
      { id: "" },
      { id: "idea", label: "IntelliJ IDEA" },
      { id: "zed", key: "dup" },
      { id: "rebased", key: "dup" },
      { id: "custom", command: 42 },
    ]);

    expect(out).toHaveLength(4);
    expect(out[0]).toMatchObject({ id: "idea", label: "IntelliJ IDEA", command: "" });
    expect(out[1].key).toBe("dup"); // 第一次出现保留
    expect(out[2].key).not.toBe("dup"); // 重复的补一个新的
    expect(out[3]).toMatchObject({ id: "custom", label: "自定义命令", command: "" });
    expect(new Set(out.map((e) => e.key)).size).toBe(4);
  });

  it("非数组一律当成空配置", () => {
    expect(normalizeEditors(undefined)).toEqual([]);
    expect(normalizeEditors({ id: "idea" })).toEqual([]);
  });

  it("用后端目录的名字覆盖预设显示名，自定义条目不动", () => {
    const out = withCatalogLabels(
      [
        { key: "1", id: "idea", label: "idea", command: "" },
        { key: "2", id: "custom", label: "自定义命令", command: "zed {path}" },
      ],
      [{ id: "idea", label: "IntelliJ IDEA" }],
    );
    expect(out[0].label).toBe("IntelliJ IDEA");
    expect(out[1]).toMatchObject({ id: "custom", label: "自定义命令", command: "zed {path}" });
  });
});