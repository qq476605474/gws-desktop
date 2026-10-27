//! 「新建需求」表单的规则。
//!
//! # 为什么单独抽出来
//!
//! 实测反馈：表单第一栏叫「名称」，填的人（包括提这个反馈的人）都以为那是
//! **中文名** —— 像「收银台改版」这样写进去，结果它其实是**分支名的后缀**；
//! 真正给人看的中文名，藏在下面一栏叫「中文备注」里，像个可填可不填的附注。
//! 更别扭的是选了「完整分支名」之后，那一栏根本用不上，却仍然必填。
//!
//! 这几条是**规则**（哪一栏该出现、留空怎么兜底、什么时候能提交），
//! 埋在组件的 `computed` 里只能靠肉眼验。抽出来就能逐条钉住。

/** 分支名怎么来：按前缀+日期拼（后缀），还是整段自己写。 */
export type BranchMode = "compose" | "full";

/** 前缀组合模式下，拼不出来时预览里显示的占位。 */
export const SUFFIX_PLACEHOLDER = "<后缀>";

export interface NewWsForm {
  mode: BranchMode;
  /** 分支后缀（只在前缀组合模式下有意义）。 */
  suffix: string;
  prefix: string;
  /** `YYYYMMDD`。 */
  date: string;
  /** 完整分支名（只在完整分支名模式下有意义）。 */
  fullBranch: string;
  /** 中文名。留空时按 [`resolveTitle`] 兜底。 */
  title: string;
  /** 选中的模块数（一个都不选就没什么可建的）。 */
  moduleCount: number;
}

/**
 * 「分支后缀」那一栏要不要出现。
 *
 * 选了完整分支名就**整栏不显示** —— 分支名已经逐字给全了，再要一个后缀
 * 既没处用，又会被当成必填项把人卡住（实测反馈就是这个）。
 */
export function needsSuffix(mode: BranchMode): boolean {
  return mode === "compose";
}

/** 去掉日期前缀的结果。`stripped` 用于界面提示「你写的日期已自动去掉」。 */
export interface StrippedDates {
  normalized: string;
  stripped: boolean;
}

function realDate(y: number, m: number, d: number): boolean {
  return y >= 2000 && y <= 2099 && m >= 1 && m <= 12 && d >= 1 && d <= 31;
}

function realMonth(y: number, m: number): boolean {
  return y >= 2000 && y <= 2099 && m >= 1 && m <= 12;
}

/** 整段就是一个日期（没有后续后缀）。 */
function isBareDate(s: string): boolean {
  let m = /^(\d{4})(\d{2})(\d{2})$/.exec(s);
  if (m) return realDate(+m[1], +m[2], +m[3]);
  m = /^(\d{2})(\d{2})(\d{2})$/.exec(s);
  if (m) return realDate(2000 + +m[1], +m[2], +m[3]);
  m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(s);
  if (m) return realDate(+m[1], +m[2], +m[3]);
  m = /^(\d{4})-(\d{2})$/.exec(s);
  if (m) return realMonth(+m[1], +m[2]);
  return false;
}

/**
 * 去掉后缀开头已经写了的日期（可能连续写了好几个）。
 *
 * 实测真实分支 `bugfix-20261020-20261020-ca-mobile-vault-2.3-bugfix`：
 * 人在后缀里已经写了 `20261020-...`，工具又自动加了 `bugfix-20261020-`。
 * 规则与 Rust 侧 `name::strip_leading_dates` 一致。
 */
export function stripLeadingDates(suffix: string): StrippedDates {
  let s = suffix.trim();
  let stripped = false;
  for (;;) {
    if (isBareDate(s)) {
      return { normalized: "", stripped: true };
    }
    let m = /^(\d{4})-(\d{2})-(\d{2})-(.+)$/.exec(s);
    if (m && realDate(+m[1], +m[2], +m[3])) {
      s = m[4].trim();
      stripped = true;
      continue;
    }
    m = /^(\d{4})-(\d{2})-(.+)$/.exec(s);
    if (m && realMonth(+m[1], +m[2])) {
      s = m[3].trim();
      stripped = true;
      continue;
    }
    m = /^(\d{4})(\d{2})(\d{2})-(.+)$/.exec(s);
    if (m && realDate(+m[1], +m[2], +m[3])) {
      s = m[4].trim();
      stripped = true;
      continue;
    }
    m = /^(\d{2})(\d{2})(\d{2})-(.+)$/.exec(s);
    if (m && realDate(2000 + +m[1], +m[2], +m[3])) {
      s = m[4].trim();
      stripped = true;
      continue;
    }
    return { normalized: s, stripped };
  }
}

/** 前缀组合模式的分支名（也是提交时用的那个）。 */
export function composeBranch(prefix: string, date: string, suffix: string): string {
  const raw = suffix.trim();
  if (!raw) return `${prefix}-${date}-${SUFFIX_PLACEHOLDER}`;
  const { normalized } = stripLeadingDates(raw);
  return [prefix, date, normalized].filter(Boolean).join("-");
}

/** 最终提交的分支名。 */
export function resolveBranch(f: Pick<NewWsForm, "mode" | "prefix" | "date" | "suffix" | "fullBranch">): string {
  if (f.mode === "full") return f.fullBranch.trim();
  return composeBranch(f.prefix, f.date, f.suffix);
}

/**
 * 最终提交的中文名。
 *
 * 留空时的兜底一律是**完整分支名**，不做「取分支后缀」那种短化：
 * 后缀是两个需求各起各的，很容易撞（比如两个需求都叫 `revamp`），撞了之后
 * 需求列表里两行同名，反而分不清哪个是哪个（实测反馈）。
 * 分支名整段唯一，拿它当兜底才靠得住。
 */
export function resolveTitle(title: string, branch: string): string {
  return title.trim() || branch.trim();
}

/** 「需求名称」那一栏的兜底说明（只有这一种，不再跟着分支模式变）。 */
export const TITLE_PLACEHOLDER = "留空就用分支名";

/**
 * 「创建」能不能点。
 *
 * 关键是**只校验当前模式下真正用得上的那一栏**：整段模式不校验后缀
 * （那一栏压根不显示），组合模式不校验完整分支名。
 */
export function canCreate(f: NewWsForm): boolean {
  if (f.moduleCount <= 0) return false;
  return f.mode === "full" ? !!f.fullBranch.trim() : !!f.suffix.trim();
}