//! 需求列表的**分组**规则：按需求分类（需求分类）把各期次归到一起。
//!
//! 实测反馈：同时在开发的「二期」「三期」在列表里各占一行、互不相邻 ——
//! 而它们的文档本来就归在同一个族目录（`doc-hub/workspaces/<族>/`）下，
//! 列表里理应也在一起。
//!
//! 抽成纯函数是为了能直接测：「没填分类的怎么归组」这条规则
//! （用户明确定的）值得钉住，而不是埋在一个 computed 里靠肉眼验。

/** 连创建日期都没有时的兜底分组名（正常数据不会走到这儿）。 */
export const UNCLASSIFIED = "未分类";

export interface Groupable {
  doc_family: string | null;
  title: string;
  /** 创建日期 `YYYY-MM-DD`。没填分类时按它的**年月**归组。 */
  created?: string;
}

/**
 * 没填需求分类时的分组名：**按创建年月**（`2026-10`）。
 *
 * 实测反馈：原来一律归进「未分类」—— 那个桶会越攒越大，几十个不相干的
 * 需求挤在一个组里，反而失去了分组的意义。按年月分至少能看出
 * 「这是哪段时间做的」，找起来有线索。
 */
export function monthGroupName(created: string | undefined): string {
  const m = /^(\d{4})-(\d{2})/.exec((created ?? "").trim());
  return m ? `${m[1]}-${m[2]}` : UNCLASSIFIED;
}

export interface WsGroup<T> {
  family: string;
  items: T[];
}

/**
 * 按需求分类分组。
 *
 * **没填分类的按创建年月归组**（见 [`monthGroupName`]）：既不留散行，
 * 也不会攒成一个巨大的「未分类」桶。
 *
 * 分组顺序 = 各组第一项在原列表里的位置，组内保持原顺序 ——
 * 不额外排序，免得「分组」顺手把顺序也改了。
 */
export function groupByFamily<T extends Groupable>(rows: T[]): WsGroup<T>[] {
  const out = new Map<string, T[]>();
  for (const r of rows) {
    // 只有空白也算「没填」
    const key = r.doc_family?.trim() || monthGroupName(r.created);
    const list = out.get(key);
    if (list) list.push(r);
    else out.set(key, [r]);
  }
  return [...out].map(([family, items]) => ({ family, items }));
}
