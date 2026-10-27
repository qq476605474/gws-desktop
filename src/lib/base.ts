//! 需求**基线来源**的显示。
//!
//! 基线是一条**有序的链**，顺序即优先级：逐模块按顺序找第一个存在的分支，
//! **都不命中就兜底主干**。而链本身**不写主干**（写进去等于多维护一项，
//! 而且「主干」是每个仓库都有的，写了是废话）。
//!
//! 所以显示时要把主干**显式补在末尾** —— 否则看不出「链上一个都不命中时落到哪」。

/**
 * 基线链的显示文本，末尾补「主干」作兜底。
 *
 * `ws:` 前缀去掉：那是命令行 `--from ws:<上游分支>` 的写法，列表里读着累赘。
 */
export function baseChainLabel(base: string[]): string {
  const chain = base
    .map((b) => b.trim())
    .filter(Boolean)
    .map((b) => (b.startsWith("ws:") ? b.slice(3) : b));
  return [...chain, "主干"].join(" → ");
}

/** 有没有真的填过基线（全空 / 只有空白 = 没填）。 */
export function hasBase(base: string[]): boolean {
  return base.some((b) => b.trim() !== "");
}
