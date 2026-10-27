//! 环境目录清单：**哪些模块在那个环境里真的检出了**。
//!
//! # 为什么需要
//!
//! 环境页列的是**配置**（这个环境该检出哪些仓库、各自检到哪条分支），
//! 而目录是**同步时**才建出来的。一个还没同步过的环境（比如刚建好的 hub）
//! 一个子目录都没有，界面上却照样给每一行配「文件管理」——
//! 点了当然没反应（实测反馈）。
//!
//! 这份清单让界面能当场把「还没检出」标出来，而不是让人点了才发现。

/**
 * `环境标识 → 该环境目录下的模块名`。
 *
 * `null` = **没读出来**（列目录失败 / 还没读），跟「读到了，是空的」是两回事。
 */
export type EnvDirs = Record<string, string[] | null>;

/**
 * 该模块在这个环境里检出了没有。
 *
 * **读不出来时返回 `true`** —— 把「未知」当成「没有」会把本来能用的按钮
 * 全禁掉，那比让用户点一下再看到报错更糟。真打不开时还有后端的存在性预检
 * 和提示兜着，不会静默。
 */
export function checkedOut(dirs: EnvDirs, envId: string, mod: string): boolean {
  const list = dirs[envId];
  if (!list) return true;
  return list.includes(mod);
}

/**
 * 这个环境是不是**确认**一个都没检出。
 *
 * 读不出来（`null`）不算 —— 否则一次列目录失败就会弹一句
 * 「这个环境还没有检出任何仓库」，而它可能只是没读到。
 */
export function confirmedEmpty(dirs: EnvDirs, envId: string): boolean {
  const list = dirs[envId];
  return Array.isArray(list) && list.length === 0;
}