//! 「对比代码」用哪个基准 ref。
//!
//! # 为什么单独抽出来
//!
//! 实测反馈：**「与主干」永远显示 0 个改动**，而本地明明改了一堆。
//!
//! 根因是这里把**两种完全不同的意思**都用了 `null`：
//!
//! - 「与主干」要让**后端去探测**主干（各仓库 `main` / `master` 混用，
//!   前端写死必错）→ 语义是「不用前端给基准」
//! - 「与环境」「与远程分支」还没选环境 / 没填分支名 → 语义是「**还没选全，
//!   先别请求**」
//!
//! 调用方只看 `null` 判断"要不要发请求"，于是「与主干」被当成"没选全"，
//! **请求根本没发出去**，界面上就是「差异 (0)」。
//!
//! 所以三个返回值各有各的意思，**不能混**：
//!
//! - `string`    → 具体基准
//! - `undefined` → 交给后端探测主干（**可以请求**）
//! - `null`      → 目标还没选全（**先别请求**）

/** 对比目标。 */
export type CompareTarget =
  | "worktree"
  | "remote"
  | "main"
  | "env"
  | "branch"
  | "local"
  | "baseline";

/** 一个可对比的目录（单模块模式就一个）。 */
export interface CompareSource {
  repo: string;
  /** 该目录检出的分支（「与远程」要用）。 */
  branch: string;
}

/** 基准的选择结果。三种返回值含义见模块头注释。 */
export type BaseChoice = string | null | undefined;

export interface BaseInput {
  target: CompareTarget;
  source: CompareSource;
  /** 「与远程分支 / 与本地分支」手输的分支名。 */
  branchInput: string;
  /** 「与环境」选中的环境标识。 */
  env: string;
  /** 该环境在这个仓库上实际用的分支；查不到就是 undefined。 */
  envBranch?: string;
  /**
   * 「与基线」时后端解析出的 ref（`origin/...`）。
   *
   * 基线要靠 git 探测（还要 fetch），**只能异步拿**；没解析出来时
   * 必须是 `null`（先别请求），不能退化成 `undefined` —— 那会被当成
   * 「与主干」，把主干的结果挂在「与基线」标题下。
   */
  baselineRef?: string;
  /** 外部指定的基准（比如合并检查传的 `origin/<环境分支>`）—— 有它就一切以它为准。 */
  override?: string | null;
}

/**
 * 这个基准**够不够发请求**。
 *
 * `undefined`（交给后端探测主干）是够的 —— 这正是那个 bug 的分界线。
 */
export function isReady(choice: BaseChoice): boolean {
  return choice !== null;
}

/** 选基准。返回值三种含义见模块头注释。 */
export function compareBase(input: BaseInput): BaseChoice {
  if (input.override) return input.override;

  switch (input.target) {
    case "worktree":
      // 与本地：工作区 ↔ 本地 HEAD（未提交的改动）。
      // 这是默认目标 —— 需求分支常常**还没推送**，那时 origin/<分支> 不存在，
      // 若默认「与远程」就只剩一句报错，本地明明改了东西却什么都看不到（实测反馈）。
      return "HEAD";
    case "remote":
      // 与远程：本地分支 ↔ 远程**同名**分支
      return input.source.branch ? `origin/${input.source.branch}` : null;
    case "branch":
      // 与远程分支：填了才算
      return input.branchInput.trim() ? `origin/${input.branchInput.trim()}` : null;
    case "local":
      // 与本地分支：**不带 `origin/`**（跟同机上的另一条本地分支比）
      return input.branchInput.trim() || null;
    case "env":
      // 与环境：必须选一个环境 —— 不选就悄悄退化成"与主干比"的话，
      // 右侧标题还写着"与环境"，等于骗人（实测反馈）
      if (!input.env) return null;
      return input.envBranch ? `origin/${input.envBranch}` : null;
    case "baseline":
      // 与基线：后端解析完才有 ref；没解析出来 = 先别请求
      return input.baselineRef ?? null;
    default:
      // 与主干：**交给后端探测**（`undefined` ≠ `null`，见模块头注释）
      return undefined;
  }
}