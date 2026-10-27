//! 全局忙碌状态与**实时进度**。
//!
//! # 为什么要实时进度
//!
//! 「拉 22 个仓库」「同步环境」要跑几十秒。只给一个转圈的话，用户分不清
//! 是在跑还是卡死了——这是实际使用反馈里最影响体验的一条。
//!
//! core 的慢操作会通过 Tauri 事件（`gws://progress`）逐项上报，这里负责
//! 收下来并维护一份可见的进度日志。

import { reactive } from "vue";
import { listen } from "@tauri-apps/api/event";

/** core `ProgressEvent` 的三种形态（serde 把 Rust 枚举序列化成单键对象）。 */
export type ProgressEvent =
  | { Start: { total: number; label: string } }
  | { Step: { done: number; total: number; item: string; ok: boolean; detail: string } }
  /**
   * 同一件事的进度刷新：**替换**上一条，不追加。
   *
   * 只带一句完整的话（`item`）—— 头部那个「x / y」计数不动，
   * 因为这条不是"第几项"（实测反馈：只有更新进度要原地替换，别处照旧）。
   */
  | { Tick: { item: string } }
  | { Done: { label: string } };

export interface ProgressLine {
  text: string;
  ok: boolean;
  /** 进度刷新行：下一条 Tick 会**替换**它，而不是往后追加。 */
  tick?: boolean;
}

/** 日志最多保留多少条——再多也没人看，而且会把弹窗撑爆。 */
const MAX_LINES = 500;

export const busy = reactive({
  /** 正在进行的后端请求数（`bridge.tracked` 维护），**不控制遮罩**。 */
  count: 0,
  /**
   * 需要遮罩的慢操作数（`withBusy` 维护）。
   *
   * 遮罩只看这个：普通请求（开 AI 会话、保存配置）也拉遮罩的话，会把上一次
   * 慢操作留下的标题和日志一起翻出来——实测弹出了早已跑完的「补齐环境 pre」。
   */
  mask: 0,
  /** 当前在做什么（没在跑时为空）。 */
  label: "",
  /** 已完成 / 总数。`total === 0` 表示总数未知。 */
  done: 0,
  total: 0,
  /** 进度日志，最新的在最后。 */
  lines: [] as ProgressLine[],
});

export function isBusy(): boolean {
  return busy.mask > 0;
}

/** 清空上一轮操作的进度。 */
function resetProgress(label: string) {
  busy.label = label;
  busy.done = 0;
  busy.total = 0;
  busy.lines = [];
}

/** 包一段带提示文字的慢操作。**遮罩只认它**。 */
export async function withBusy<T>(label: string, fn: () => Promise<T>): Promise<T> {
  busy.count++;
  busy.mask++;
  resetProgress(label);
  try {
    return await fn();
  } finally {
    busy.count--;
    busy.mask--;
    if (busy.mask === 0) {
      busy.label = "";
      // 旧日志也清掉：下次普通请求不该把上一次的进度翻出来
      busy.lines = [];
      busy.done = 0;
      busy.total = 0;
    }
  }
}

function push(text: string, ok: boolean) {
  busy.lines.push({ text, ok });
  if (busy.lines.length > MAX_LINES) {
    busy.lines.splice(0, busy.lines.length - MAX_LINES);
  }
}

/**
 * 进度刷新：**替换**上一条进度行，不追加。
 *
 * 实测反馈：下载一个 5.8 MB 的包，日志一路往后追加了几十条 —— 那种"数字在变"
 * 的东西本来就该原地刷。上一条不是进度行（比如刚打完一项的 ✓）就新起一行。
 */
export function tickForTest(text: string) {
  tick(text);
}

function tick(text: string) {
  const last = busy.lines[busy.lines.length - 1];
  if (last?.tick) {
    last.text = text;
  } else {
    busy.lines.push({ text, ok: true, tick: true });
  }
}

/**
 * 把一条后端进度事件并进 `busy` 状态。
 *
 * **抽成导出的纯函数**（`initProgressListener` 只是把它接到事件上）：单测要能
 * 直接喂事件走**同一条**逻辑。原来测试里自己抄了一份 if-else，于是改了这里
 * 测试照样绿 —— 那种"测试"等于没测。
 */
export function applyProgress(p: ProgressEvent): void {
  if ("Start" in p) {
    busy.label = p.Start.label;
    busy.total = p.Start.total;
    busy.done = 0;
    busy.lines = [];
  } else if ("Tick" in p) {
    // **只替换那一行**：done/total 是"第几项"的语义，下载进度不参与
    tick(p.Tick.item);
  } else if ("Step" in p) {
    const s = p.Step;
    busy.done = s.done;
    busy.total = s.total;
    push(s.detail ? `${s.item}  ${s.detail}` : s.item, s.ok);
  } else {
    // Done：不清 label，让「完成」那一瞬仍能看到是哪件事完成了
    busy.label = p.Done.label;
  }
}

/**
 * 订阅后端进度事件。**只订阅一次**（在 App 挂载时调用）。
 *
 * 事件名是固定的，靠内容区分操作——所以能在命令返回之前就开始接收，
 * 这正是「实时」的前提。
 */
export async function initProgressListener(): Promise<() => void> {
  return listen<ProgressEvent>("gws://progress", (e) => applyProgress(e.payload));
}