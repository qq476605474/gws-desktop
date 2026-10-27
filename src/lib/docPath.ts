//! 一期文档**到底读哪个目录**。
//!
//! # 两份拷贝
//!
//! 同一期文档在本地可能有两份：
//!
//! - **生产环境那份**：`<hub>/repos/doc-hub/workspaces/<族>/<分支>/`
//!   —— 期次已并入 `main`（= **已上线**）时才有
//! - **工作区那份**：`<hub>/ws/<分支>/doc-hub/workspaces/<族>/<分支>/`
//!   —— 需求还在开发（**还没上线**）时文档在这里
//!
//! # 规则（实测反馈定的）
//!
//! **优先取生产环境那份**：它在 `main` 上，是已经上线的那一份；取不到才退到
//! 工作区那份。已归档的期次同理，界面上、「文件管理」按钮上都要用**同一个**答案 ——
//! 否则会出现「显示的是一份、点开文件管理跳到另一份」，那种不一致比看不到更难查。
//!
//! 抽成纯函数是为了能直接钉住这个优先级，而不是埋在两个 `if` 里靠肉眼验。

/** 这一份目录是从哪儿来的。 */
export type DocWhere = "main" | "ws";

export interface DocDir {
  /** 绝对路径。 */
  dir: string;
  where: DocWhere;
}

/** 两份拷贝各自的路径。 */
export function docDirs(
  root: string,
  docName: string,
  family: string,
  branch: string,
): Record<DocWhere, string> {
  const rel = `workspaces/${family}/${branch}`;
  return {
    main: `${root}/repos/${docName}/${rel}`,
    ws: `${root}/ws/${branch}/${docName}/${rel}`,
  };
}

/**
 * 候选目录，**按优先级排**。
 *
 * 已归档（已上线）的：先生产环境，再工作区 —— 工作区那份是开发期的中间产物，
 * 线上那份才是这一期最终的样子。
 * 开发中的：反过来（线上还没有）。
 *
 * 两份都给出来，是为了调用方能「取不到就退下一个」，而不是猜一个路径然后空着。
 */
export function docDirCandidates(
  root: string,
  docName: string,
  family: string,
  branch: string,
  merged: boolean,
): DocDir[] {
  const d = docDirs(root, docName, family, branch);
  return merged
    ? [{ dir: d.main, where: "main" }, { dir: d.ws, where: "ws" }]
    : [{ dir: d.ws, where: "ws" }, { dir: d.main, where: "main" }];
}

/** 这一份目录的说明文字（界面上要标出来，免得两份傻傻分不清）。 */
export function docWhereLabel(where: DocWhere): string {
  return where === "main" ? "生产环境（已在 main 上）" : "工作区（还没并主干）";
}
