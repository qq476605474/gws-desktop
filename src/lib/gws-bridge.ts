import { invoke } from "@tauri-apps/api/core";

export interface RunResult { code: number | null; output: string; }

/** 在指定 cwd 执行 gws（一次性收集全部输出）。args 不含 "gws" 本身。 */
export function runGws(args: string[], cwd: string): Promise<RunResult> {
  return invoke<RunResult>("run_gws", { args, cwd });
}

/** 流式执行：返回 runId，输出经 Tauri 事件 gws-output:<runId> 推送。 */
export function runGwsStream(args: string[], cwd: string): Promise<number> {
  return invoke<number>("run_gws_stream", { args, cwd });
}

/** 交互确认场景：向 stdin 写入 "y\n"（yes=true）或杀进程（yes=false）。 */
export function respondConfirm(runId: number, yes: boolean): Promise<void> {
  return invoke("respond_confirm", { runId, yes });
}

/** 前端订阅事件完成后调用：后端回放订阅前缓存的事件并切换直发。 */
export function replayOutput(runId: number): Promise<void> {
  return invoke("replay_output", { runId });
}
