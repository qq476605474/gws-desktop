//! 忙碌状态与进度日志的语义。
//!
//! 钉两件事：① 进度行要**原地替换**（下载进度一路追加过几十条）；
//! ② 计数进出成对、跑完要归零（不然遮罩永远不消失）。
//!
//! **「后台运行」那套已经移除**（实测反馈：点过它再去做别的会变成两件事并行），
//! 所以这里不再有"收起/展开"的用例。

import { beforeEach, describe, expect, it } from "vitest";

import { applyProgress, busy, withBusy, type ProgressEvent } from "./busy";

// `busy` 是模块级单例，用例之间会串（上一条的日志留在 lines 里）—— 每条前清干净
beforeEach(() => {
  busy.lines = [];
  busy.count = 0;
  busy.mask = 0;
  busy.label = "";
  busy.done = 0;
  busy.total = 0;
});

/** 喂一条后端事件。
 *
 * 走 `applyProgress` 这个**真身**（`initProgressListener` 只是把它接到事件上）——
 * 原来这里自己抄了一份 if-else，于是 `busy.ts` 改了测试照样绿，等于没测。 */
function feed(events: ProgressEvent[]) {
  for (const p of events) applyProgress(p);
}

describe("进度日志：进度行要**原地替换**，不能一路追加", () => {
  it("连着来 20 条 Tick，日志只该有一条在变的行", async () => {
    // 实测反馈：下载 5.8 MB 追加了几十条日志
    feed([{ Start: { total: 20, label: "下载" } }]);
    for (let i = 1; i <= 20; i++) {
      await feed([{ Tick: { item: `下载 gws-desktop.dmg  ${i * 0.29} MB / 5.8 MB` } }]);
    }
    expect(busy.lines.length, "20 条进度只该剩一行").toBe(1);
    expect(busy.lines[0].text).toContain("5.8 MB");
  });

  it("进度行之后来一条 Step，要**新起一行**（那是新的一项，不是刷新）", async () => {
    feed([{ Start: { total: 2, label: "下载并安装" } }]);
    await feed([{ Tick: { item: "下载  50%" } }]);
    // Step 是**新的一项**，不能被上一条 Tick 吃掉
    await feed([{ Step: { done: 1, total: 2, item: "校验通过", ok: true, detail: "" } }]);
    // 上一条已经不是进度行了 → 这次的 Tick 得**新起一行**（不能回头改那条 ✓）
    await feed([{ Tick: { item: "下载  90%" } }]);
    expect(busy.lines.map((l) => l.text)).toEqual(["下载  50%", "校验通过", "下载  90%"]);
  });
});

describe("忙碌计数：进出成对、跑完归零", () => {
  it("跑的时候 count/mask = 1，跑完归零且清空 label（否则遮罩永远不消失）", async () => {
    let during = 0;
    let maskDuring = 0;
    await withBusy("慢操作", async () => {
      during = busy.count;
      maskDuring = busy.mask;
    });
    expect(during).toBe(1);
    expect(maskDuring, "慢操作要拉起遮罩").toBe(1);
    expect(busy.count).toBe(0);
    expect(busy.mask).toBe(0);
    expect(busy.label).toBe("");
  });

  it("抛错也要归零（不然遮罩会一直盖着）", async () => {
    await expect(withBusy("会炸的操作", async () => { throw new Error("炸了"); })).rejects.toThrow("炸了");
    expect(busy.count).toBe(0);
    expect(busy.mask).toBe(0);
  });

  it("普通请求不拉遮罩：开 AI 会话时不能冒出上一次的「补齐环境 pre」", async () => {
    // 上一次慢操作结束后仍留着旧日志（修复前遮罩只看 count，普通请求一来就全露出来）
    busy.label = "补齐环境 pre";
    busy.lines = [{ text: "doc-hub（已存在，不动）", ok: true }];

    // `bridge.tracked` 只动 count
    busy.count++;
    expect(busy.mask, "开 AI 会话这类普通请求不该盖遮罩").toBe(0);
    busy.count--;
  });

  it("慢操作结束后旧进度清干净，下一轮只会看到自己的标题", async () => {
    await withBusy("第一件事", async () => {
      applyProgress({ Start: { total: 1, label: "补齐环境 pre" } });
      applyProgress({ Step: { done: 1, total: 1, item: "doc-hub（已存在，不动）", ok: true, detail: "" } });
    });
    expect(busy.lines).toEqual([]);
    expect(busy.label).toBe("");

    let seen = "";
    await withBusy("第二件事", async () => {
      seen = busy.label;
    });
    expect(seen).toBe("第二件事");
  });
});
