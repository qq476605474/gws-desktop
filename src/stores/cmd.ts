import { listen } from "@tauri-apps/api/event";
import { defineStore } from "pinia";
import { reactive, ref } from "vue";
import { runGwsStream, respondConfirm } from "../lib/gws-bridge";

export interface CmdRun {
  id: number;
  label: string;
  output: string;
  state: "running" | "confirm" | "done" | "failed";
  code: number | null;
}

export const useCmdStore = defineStore("cmd", () => {
  const current = ref<CmdRun | null>(null);
  const history = ref<CmdRun[]>([]);
  const confirmPending = ref<{ runId: number; question: string } | null>(null);

  async function exec(label: string, args: string[], cwd: string): Promise<CmdRun> {
    // 契约（依赖后端 run_gws_stream 两条不变量）：
    // 1) invoke 必须在子进程退出前返回 runId，否则下方 exit 订阅会错过 gws-exit 事件；
    // 2) 订阅完成前已发出的 gws-output/gws-exit/gws-confirm 事件须由后端缓存回放（见计划任务 5）。
    const runId = await runGwsStream(args, cwd);
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
    return run;
  }

  async function answerConfirm(yes: boolean) {
    if (!confirmPending.value) return;
    const { runId } = confirmPending.value;
    await respondConfirm(runId, yes);
    // 仅当挂起确认属于当前 run 时才改其状态，防止陈旧 confirm 误伤新 run
    if (current.value?.id === runId) {
      current.value.state = yes ? "running" : "failed";
    }
    confirmPending.value = null;
  }

  function isRunning(): boolean {
    return current.value?.state === "running" || current.value?.state === "confirm";
  }

  return { current, history, confirmPending, exec, answerConfirm, isRunning };
});
