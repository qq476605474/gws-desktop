import { listen } from "@tauri-apps/api/event";
import { defineStore } from "pinia";
import { reactive, ref, watch } from "vue";
import { replayOutput, respondConfirm, runGwsStream } from "../lib/gws-bridge";

export interface CmdRun {
  id: number;
  label: string;
  output: string;
  state: "running" | "confirm" | "done" | "failed";
  code: number | null;
}

/** exec/execDialog 的可选参数。confirmTimeoutMs：stdout 静默多久后弹"等待确认"。 */
export interface ExecOpts {
  confirmTimeoutMs?: number;
}

export const useCmdStore = defineStore("cmd", () => {
  const current = ref<CmdRun | null>(null);
  const history = ref<CmdRun[]>([]);
  const confirmPending = ref<{ runId: number; question: string } | null>(null);
  /** CmdDialog 当前展示的 run（操作类命令的弹窗式执行）；null = 无弹窗 */
  const dialogRun = ref<CmdRun | null>(null);
  /** 弹窗保持计数：>0 时关闭按钮禁用（多命令序列如 AddModuleDialog 整段期间保持弹窗） */
  const holdCount = ref(0);

  async function exec(label: string, args: string[], cwd: string, opts?: ExecOpts): Promise<CmdRun> {
    // 契约（依赖后端 run_gws_stream 两条不变量）：
    // 1) invoke 必须在子进程退出前返回 runId，否则下方 exit 订阅会错过 gws-exit 事件；
    // 2) 订阅完成前已发出的 gws-output/gws-exit/gws-confirm 事件须由后端缓存回放（见计划任务 5）。
    const runId = await runGwsStream(args, cwd, opts?.confirmTimeoutMs);
    // reactive 包装：事件回调闭包持有 run 本身，若为普通对象，
    // 回调里 run.output += ... 会绕过 ref 内部的代理而不触发 UI 更新。
    const run: CmdRun = reactive({ id: runId, label, output: "", state: "running", code: null });
    current.value = run;
    // 三个订阅在 run 终态后拆除（见 exit handler 内的延迟清理）：长会话中监听表不随 exec 次数线性增长
    const unlisteners: Array<() => void> = [];
    try {
      unlisteners.push(await listen<{ chunk: string }>(`gws-output:${runId}`, (e) => {
        run.output += e.payload.chunk;
      }));
      unlisteners.push(await listen<{ code: number | null }>(`gws-exit:${runId}`, (e) => {
        run.code = e.payload.code;
        run.state = e.payload.code === 0 ? "done" : "failed";
        // confirm 态下进程自行退出：清掉指向已死 runId 的挂起确认，避免僵尸弹窗
        if (confirmPending.value?.runId === runId) confirmPending.value = null;
        history.value.unshift(run);
        // 延迟清理（不在 handler 内同步拆）的时序论证：
        // a) Rust 侧 waiter 等读者排空才记 Exit、所有事件持锁按序 emit → Exit 恒为该 run
        //    的最后一个事件，此后再无事件（watchdog 见 finished 即停、run 随即被清理）；
        // b) 回放不受影响：replayOutput 在三个订阅注册完成之后才被调用，exit 事件（无论
        //    回放还是直发）只可能在其后送达，exit handler 触发时不存在“尚未回放”的事件；
        // c) 但 JS 侧投递是异步的，同 run 先 emit 的 output 与 exit 的到达间无同步屏障——
        //    让出一个宏任务（setTimeout 0），确保已在途的投递全部落地后才拆除三个订阅。
        setTimeout(() => {
          for (const off of unlisteners) off();
        }, 0);
      }));
      unlisteners.push(await listen<{ question: string }>(`gws-confirm:${runId}`, (e) => {
        run.state = "confirm";
        confirmPending.value = { runId, question: e.payload.question };
      }));
    } catch (e) {
      // listen 注册失败（罕见）：拆除已注册的订阅再抛，不留泄漏
      for (const off of unlisteners) off();
      throw e;
    }
    // 三个事件订阅完成 → 后端回放缓存事件并切换直发（否则订阅前的事件永远丢失）
    await replayOutput(runId);
    return run;
  }

  async function answerConfirm(yes: boolean) {
    if (!confirmPending.value) return;
    const { runId } = confirmPending.value;
    try {
      await respondConfirm(runId, yes);
    } finally {
      // respondConfirm 抛错（如 run 已被后端清理）也要收尾，否则弹窗永远关不掉；
      // 仅当挂起确认属于当前 run 时才改其状态，防止陈旧 confirm 误伤新 run
      if (current.value?.id === runId) {
        current.value.state = yes ? "running" : "failed";
      }
      confirmPending.value = null;
    }
  }

  /** 弹窗式执行（操作类命令）：CmdDialog 全屏遮罩显示流式输出，
   * 命令结束后须手动点关闭（closeDialog）才能继续操作。
   * confirmTimeoutMs 默认 30000：慢命令（sync 的静默 git 阶段、repo add 的 clone）
   * 静默远超 1500ms 默认阈值会被误弹"等待确认"；真读 stdin 的命令（gws drop）
   * 由调用方显式传 1500。 */
  async function execDialog(label: string, args: string[], cwd: string, opts?: ExecOpts): Promise<CmdRun> {
    try {
      const run = await exec(label, args, cwd, { confirmTimeoutMs: opts?.confirmTimeoutMs ?? 30000 });
      dialogRun.value = run;
      return run;
    } catch (e) {
      // invoke reject（如 gws 未安装、IPC 失败）：命令弹窗展示合成 failed run，
      // 让用户看到失败原因并手动关闭（不额外订阅事件，id 取 -1 与真实 runId 区分）；
      // 随后原样 rethrow——调用方既有 catch（内联 err 提示）逻辑保持不变
      dialogRun.value = { id: -1, label, output: String(e), state: "failed", code: null };
      throw e;
    }
  }

  /** 用户手动关闭命令弹窗（运行中/hold 期间按钮禁用，此处不再重复拦截） */
  function closeDialog() {
    dialogRun.value = null;
  }

  /** 多命令序列（如 AddModuleDialog 逐模块 add）期间持有弹窗：计数>0 时关闭按钮禁用，
   * 防止序列中途弹窗被关掉后后续命令无输出可见。成对调用，release 须放 finally。 */
  function holdDialog() {
    holdCount.value++;
  }

  function releaseDialog() {
    if (holdCount.value > 0) holdCount.value--;
  }

  function isRunning(): boolean {
    return current.value?.state === "running" || current.value?.state === "confirm";
  }

  /** 等待 run 到终态（done/failed）。已终态时前置短路同步 resolve。 */
  function waitDone(run: CmdRun): Promise<CmdRun> {
    // WHY 前置短路：tauri exit 事件可能先于 exec promise 决议，调用方拿到 run 时
    // 它可能已是终态——直接 resolve，不进 watcher。
    if (run.state === "done" || run.state === "failed") return Promise.resolve(run);
    // 走到这里 run 必为非终态，state 只会在事件回调里变更；无 immediate 的 watch
    // 回调总是异步触发（pre-flush 队列），执行时 const stop 已完成赋值，无 TDZ 风险
    return new Promise((resolve) => {
      const stop = watch(() => run.state, (s) => {
        if (s === "done" || s === "failed") { stop(); resolve(run); }
      });
    });
  }

  return {
    current, history, confirmPending, dialogRun, holdCount,
    exec, execDialog, closeDialog, holdDialog, releaseDialog,
    answerConfirm, isRunning, waitDone,
  };
});
