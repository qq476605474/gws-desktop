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
    const runId = await runGwsStream(args, cwd);
    // reactive 包装：事件回调闭包持有 run 本身，若为普通对象，
    // 回调里 run.output += ... 会绕过 ref 内部的代理而不触发 UI 更新。
    const run: CmdRun = reactive({ id: runId, label, output: "", state: "running", code: null });
    current.value = run;
    const { listen } = await import("@tauri-apps/api/event");
    await listen<{ chunk: string }>(`gws-output:${runId}`, (e) => {
      run.output += e.payload.chunk;
    });
    await listen<{ code: number }>(`gws-exit:${runId}`, (e) => {
      run.code = e.payload.code;
      run.state = e.payload.code === 0 ? "done" : "failed";
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
    await respondConfirm(confirmPending.value.runId, yes);
    if (!yes && current.value) current.value.state = "failed";
    confirmPending.value = null;
  }

  function isRunning(): boolean {
    return current.value?.state === "running" || current.value?.state === "confirm";
  }

  return { current, history, confirmPending, exec, answerConfirm, isRunning };
});
