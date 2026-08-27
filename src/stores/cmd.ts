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
    await listen<{ chunk: string }>(`gws-output:${runId}`, (e) => {
      run.output += e.payload.chunk;
    });
    await listen<{ code: number | null }>(`gws-exit:${runId}`, (e) => {
      run.code = e.payload.code;
      run.state = e.payload.code === 0 ? "done" : "failed";
      // confirm 态下进程自行退出：清掉指向已死 runId 的挂起确认，避免僵尸弹窗
      if (confirmPending.value?.runId === runId) confirmPending.value = null;
      history.value.unshift(run);
    });
    await listen<{ question: string }>(`gws-confirm:${runId}`, (e) => {
      run.state = "confirm";
      confirmPending.value = { runId, question: e.payload.question };
    });
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
    const run = await exec(label, args, cwd, { confirmTimeoutMs: opts?.confirmTimeoutMs ?? 30000 });
    dialogRun.value = run;
    return run;
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
