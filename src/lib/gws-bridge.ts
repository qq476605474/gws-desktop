import { invoke } from "@tauri-apps/api/core";
import { busyCount } from "./busy";

export interface RunResult { code: number | null; output: string; }

/** 在指定 cwd 执行 gws（一次性收集全部输出）。args 不含 "gws" 本身。
 * 数据刷新类调用：在途期间 busyCount +1（App.vue 全屏加载遮罩挡住其他操作），
 * resolve/reject 均归零（finally）。 */
export function runGws(args: string[], cwd: string): Promise<RunResult> {
  busyCount.value++;
  return invoke<RunResult>("run_gws", { args, cwd }).finally(() => {
    busyCount.value--;
  });
}

/** 流式执行：返回 runId，输出经 Tauri 事件 gws-output:<runId> 推送。
 * confirmTimeoutMs：stdout 静默多久后发 gws-confirm（缺省走 Rust 侧默认 1500ms）。
 * 慢命令（sync 的静默 git 阶段、repo add 的 clone）须传大值（如 30000）防假确认；
 * 真读 stdin 的命令（gws drop）保持默认。参数名 camelCase 由 Tauri 自动映射
 * Rust 侧 snake_case 的 confirm_timeout_ms（与 runId → run_id 同一机制）。 */
export function runGwsStream(args: string[], cwd: string, confirmTimeoutMs?: number): Promise<number> {
  return invoke<number>("run_gws_stream", { args, cwd, confirmTimeoutMs });
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

/** 查询 gws 远端最新版本（默认 GitHub raw 源，GWS_UPDATE_URL 环境变量可覆盖）。
 * Rust 侧返回 Result<String,String>：网络失败/内容非 gws 脚本时 invoke reject（中文错误文案），不会 resolve null。 */
export function latestGwsVersion(): Promise<string> {
  return invoke<string>("latest_gws_version");
}
