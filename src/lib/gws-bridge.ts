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

/** 目录是否为 gws hub（.gws-hub 标记存在）。 */
export function hubExists(path: string): Promise<boolean> {
  return invoke<boolean>("hub_exists", { path });
}

/** 在系统文件管理器（macOS Finder 等）中打开指定目录。 */
export function openInFinder(path: string): Promise<void> {
  return invoke("open_in_finder", { path });
}
/** terminal 传 null 表示跟随系统（Rust 侧自动检测 iTerm2/Terminal.app）。 */
export function openInTerminal(path: string, terminal: string | null): Promise<void> {
  return invoke("open_in_terminal", { path, terminal });
}
/** 检测 gws 是否已安装（PATH 中可找到可执行文件）。 */
export function checkGwsInstalled(): Promise<boolean> {
  return invoke<boolean>("check_gws_installed");
}
