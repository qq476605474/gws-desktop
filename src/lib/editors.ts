//! 「用编辑器打开」的常用编辑器列表（有序）。
//!
//! 一条配置只描述**用哪个 + 怎么用**，顺序就是菜单里的顺序：
//! - 预设条目：`id` 是后端 `EDITORS` 里的 id，`command` 为空；
//! - 自定义条目：`id === "custom"`，`command` 是命令模板（`{path}` 占位）。
//!
//! 这里只放纯函数，方便单测；存到本地文件的是 `{key,id,label,command}[]`。

/** 后端的自定义命令 id。 */
export const CUSTOM_ID = "custom";

export interface EditorEntry {
  /** 列表内唯一，用来渲染与排序（重排时不能靠 index 当 key）。 */
  key: string;
  id: string;
  /** 显示名（预设从后端目录带过来；自定义条目用命令本身）。 */
  label: string;
  /** 自定义命令模板；预设为空串。 */
  command: string;
}

export interface EditorCatalogItem {
  id: string;
  label: string;
}

let seq = 0;

/** 生成列表内唯一的 key。加时间戳，避免重启后序号从头来与旧配置撞车。 */
export function newEditorKey(): string {
  seq += 1;
  return `ed${Date.now().toString(36)}-${seq}`;
}

/** 能否真的用来打开：自定义条目必须填了命令。 */
export function isUsable(entry: EditorEntry): boolean {
  return entry.id !== CUSTOM_ID || entry.command.trim() !== "";
}

/** 菜单里真正要列出来的条目，保持配置顺序。 */
export function usableEditors(list: EditorEntry[]): EditorEntry[] {
  return list.filter(isUsable);
}

/**
 * 菜单 / 设置里显示的名字。
 *
 * 选程序时会把程序名写进 `label`；旧配置里自定义条目的 label 是占位的
 * 「自定义命令」，这时退回显示命令本身，至少能看出是哪一条。
 */
export function displayLabel(entry: EditorEntry): string {
  const label = entry.label.trim();
  if (label && label !== "自定义命令") return label;
  if (entry.id === CUSTOM_ID) return entry.command.trim() || "未命名程序";
  return label || entry.id;
}

/** 从程序路径里取一个能看懂的短名字（`IntelliJ IDEA.app` → `IntelliJ IDEA`）。 */
export function programLabel(path: string): string {
  const base = path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? "";
  const isExe = /\.exe$/i.test(base);
  let name = base
    .replace(/\.app$/i, "")
    .replace(/\.(exe|cmd|bat|sh|appimage)$/i, "");
  // JetBrains 的 launcher 常带位数后缀（idea64 / webstorm64）
  if (isExe) name = name.replace(/(32|64)$/, "");
  return name || "未命名程序";
}

/**
 * 把选中的程序变成后端能执行的命令。
 *
 * macOS 选到的是 `.app` 包，直接执行包目录会失败，必须 `open -a`；
 * 其它情况用带引号的绝对路径。后端会自动把目录追加在末尾。
 */
export function commandForProgram(path: string): string {
  const p = path.trim();
  return /\.app$/i.test(p) ? `open -a "${p}"` : `"${p}"`;
}

/**
 * 把本地文件里读出来的东西收成合法条目。
 *
 * 配置文件是用户能改的，也可能是旧版本写的：宁可丢掉坏条目，
 * 也不要让界面在渲染时抛错。缺 key / key 重复的补一个新的。
 */
export function normalizeEditors(raw: unknown): EditorEntry[] {
  if (!Array.isArray(raw)) return [];
  const out: EditorEntry[] = [];
  const seen = new Set<string>();
  for (const item of raw) {
    if (!item || typeof item !== "object") continue;
    const o = item as Record<string, unknown>;
    const id = typeof o.id === "string" ? o.id : "";
    if (!id) continue;
    const command = typeof o.command === "string" ? o.command : "";
    const label = typeof o.label === "string" ? o.label : "";
    let key = typeof o.key === "string" ? o.key : "";
    if (!key || seen.has(key)) key = newEditorKey();
    seen.add(key);
    out.push({
      key,
      id,
      label: label || (id === CUSTOM_ID ? "自定义命令" : id),
      command,
    });
  }
  return out;
}

/** 用后端目录里的正式名字覆盖预设条目的显示名（旧配置里存的是 id）。 */
export function withCatalogLabels(
  list: EditorEntry[],
  catalog: EditorCatalogItem[],
): EditorEntry[] {
  const names = new Map(catalog.map((o) => [o.id, o.label]));
  let changed = false;
  const out = list.map((e) => {
    if (e.id === CUSTOM_ID) return e;
    const label = names.get(e.id);
    if (!label || label === e.label) return e;
    changed = true;
    return { ...e, label };
  });
  // 没变就返回原数组：设置页每次打开都会跑一遍，别白白触发落盘
  return changed ? out : list;
}