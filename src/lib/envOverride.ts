//! 环境「分支覆盖」那张表里，**"仓库"那一栏的候选**。
//!
//! # 为什么不让选「所有仓库」
//!
//! 通配（`*`）盖上去之后，那些**实际没有这条分支**的仓库会被静默跳过 ——
//! 人设了一条 `* → pre-202610`，以为整个环境都换了，其实只有一部分动了
//! （实测反馈：要求**必须逐个仓库指定**）。
//!
//! 保留一个例外：这一行**本来就是 `*`** 时仍然把它列出来，否则老数据
//! 打开就变成一个"选不中任何项"的空下拉，改都没法改。

export const WILDCARD = "*";

export interface RepoOption {
  label: string;
  value: string;
  /**
   * naive-ui 的 `SelectMixedOption` 带索引签名，普通 interface 传不进去 ——
   * 这里放宽一格（只是给下拉用的选项包，没有别的字段会被读）。
   */
  [key: string]: unknown;
}

/**
 * 候选列表：**按原顺序**给仓库，末尾不追加通配。
 *
 * `current` 是这一行当前的值 —— 只有它已经是 `*` 时才补上那一项。
 */
export function overrideRepoOptions(
  repos: { name: string }[],
  current: string,
): RepoOption[] {
  const out: RepoOption[] = repos.map((r) => ({ label: r.name, value: r.name }));
  if (current === WILDCARD) {
    out.unshift({ label: "*（所有仓库）", value: WILDCARD });
  }
  return out;
}
