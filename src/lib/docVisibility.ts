//! 文档页的可见性规则。
//!
//! 抽出来是因为它**不只是排序**：一条期次该不该出现，决定了用户会不会
//! 看到一份"点开是空的"文档（实测反馈：「巨量单元状态同步」只在远程分支 /
//! 环境 worktree 里，本地文档页打不开，却一直占着一行）。

import type { Family, PeriodWithState } from "../types";

/**
 * 一个族里要显示的期次。
 *
 * - 只有脚手架（`meta.json` / `CLAUDE.md` / `AGENTS.md`）的空期次不显示；
 * - 本地打不开的期次（`available=false`）不显示 —— 点开是空的；
 * - 搜索词可按族名 / 分支名 / 需求名过滤（族名命中时整族都算）；
 * - 排序：**开发中在前、已归档在后**，各自**时间倒序**。
 */
export function visiblePeriods(f: Family, query: string): PeriodWithState[] {
  const s = query.trim().toLowerCase();
  const hit = (p: PeriodWithState) =>
    !s ||
    f.name.toLowerCase().includes(s) ||
    p.branch.toLowerCase().includes(s) ||
    p.name.toLowerCase().includes(s);
  return f.periods
    .filter((p) => p.has_content && p.available && hit(p))
    .slice()
    .sort((a, b) => {
      if (a.merged !== b.merged) return a.merged ? 1 : -1; // 开发中在前
      return b.date.localeCompare(a.date); // 时间倒序
    });
}
