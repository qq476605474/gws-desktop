import { stripAnsi } from "./ansi";

export interface WsEntry { name: string; title: string; stage: string; modules: number; branch: string; }
export interface StModule {
  name: string;
  dirty?: number;
  /**
   * ahead/behind 三态：缺省=missing 模块（目录缺失行）、null=未推送（远程状态未知）、数字=已知差值。
   * aheadOfMain 缺省=无法计算（vs主干列显示 +?），数字=已知。
   */
  ahead?: number | null;
  behind?: number | null;
  pushed?: boolean;
  aheadOfMain?: number;
  missing?: boolean;
}
export interface StResult { title: string; branch: string; modules: StModule[]; }
export interface RepoEntry { name: string; mainBranch: string; }
export interface DocEntry { file: string; synced: boolean; pageId: string | null; }

export function parseLs(out: string): WsEntry[] {
  const lines = stripAnsi(out).split("\n").filter((l) => l.trim());
  const res: WsEntry[] = [];
  for (const l of lines) {
    if (l.startsWith("名称") || l.includes("暂无工作区")) continue;
    const m = l.match(/^(\S+)\s+(.*?)\s+(\S+)\s+(\d+)\s+(\S+)$/);
    if (m) res.push({ name: m[1], title: m[2], stage: m[3], modules: Number(m[4]), branch: m[5] });
  }
  return res;
}

export function parseSt(out: string): StResult {
  const lines = stripAnsi(out).split("\n");
  const res: StResult = { title: "", branch: "", modules: [] };
  const header1 = lines[0] ?? "";
  const m0 = header1.match(/^(.*?)\s{2}(\S+)$/);
  if (m0) { res.title = m0[1]; res.branch = m0[2]; }
  for (const l of lines) {
    const t = l.trim();
    if (!t || t.startsWith("模块") || t === res.title + "  " + res.branch) continue;
    if (t.includes("目录缺失")) {
      const mm = t.match(/^(\S+)/);
      if (mm) res.modules.push({ name: mm[1], missing: true });
      continue;
    }
    // 改动列恒为数字（gws 源码 wc -l）；vs远程 列为 ↑N ↓N 或 未推送；vs主干 列为 +N 或 +?
    // 注意：可选组必须捕获两个分支——若写成 (?:未推送|(↑\d+ ↓\d+))?，「未推送」命中时 m[3] 恒为 undefined
    const m = t.match(/^(\S+)\s+(未推送|\d+)\s+(未推送|↑\d+ ↓\d+)?\s*\+?(\d+|\?)$/);
    if (!m) continue;
    const mod: StModule = { name: m[1] };
    if (m[2] === "未推送") {
      // 改动列出现「未推送」形态（防御，实际 gws 不会输出）：改动未知按 0
      mod.dirty = 0; mod.pushed = false; mod.ahead = null; mod.behind = null;
    } else {
      mod.dirty = Number(m[2]);
      if (m[3] === "未推送") {
        mod.pushed = false; mod.ahead = null; mod.behind = null;
      } else if (m[3]) {
        mod.pushed = true;
        const ab = m[3].match(/↑(\d+) ↓(\d+)/);
        if (ab) { mod.ahead = Number(ab[1]); mod.behind = Number(ab[2]); }
      }
    }
    mod.aheadOfMain = m[4] === "?" ? undefined : Number(m[4]);
    res.modules.push(mod);
  }
  return res;
}

export function parseRepoLs(out: string): RepoEntry[] {
  const lines = stripAnsi(out).split("\n").filter((l) => l.trim());
  const res: RepoEntry[] = [];
  for (const l of lines) {
    if (l.startsWith("仓库") || l.startsWith("共 ") || l.includes("暂无仓库")) continue;
    const m = l.match(/^(\S+)\s+(\S+)$/);
    if (m) res.push({ name: m[1], mainBranch: m[2] });
  }
  return res;
}

export function parseEnvLs(out: string): string[] {
  const res: string[] = [];
  for (const l of stripAnsi(out).split("\n")) {
    // 跳过「环境分支」表头与空提示行，仅提取 ○/● 环境名
    if (l.includes("环境分支") || l.includes("(暂无环境")) continue;
    const m = l.match(/^\s*[○●]\s+(\S+)\s+\(/);
    if (m) res.push(m[1]);
  }
  return res;
}

export function parseDocLs(out: string): DocEntry[] {
  const lines = stripAnsi(out).split("\n");
  const res: DocEntry[] = [];
  for (const l of lines) {
    // gws doc ls 行有 2 个前导空格（echo "  ● ..."），故锚点须容忍行首空白；
    // 文件名可含空格（gws doc new 不禁止），而状态列前恒为 2+ 空格分隔——
    // 用非贪婪 (.+?\.md) 跨空格取文件名，配 \s{2,} 保证状态列不被吞进文件名
    const m = l.match(/^\s*[●○]\s+(.+?\.md)\s{2,}(?:wiki:(\d+)|\(未上传\))?/);
    if (m) res.push({ file: m[1], synced: l.includes("●"), pageId: m[2] ?? null });
  }
  return res;
}

export function parseVersion(out: string): string {
  const m = stripAnsi(out).match(/^gws\s+(\S+)/);
  return m ? m[1] : "";
}

// gws doc ls 首行是 docdir 名（任务 12 文档 Tab 用）
export function parseDocDir(out: string): string {
  return stripAnsi(out.split("\n")[0]).trim();
}
