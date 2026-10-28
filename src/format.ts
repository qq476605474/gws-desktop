//! 把 core 返回的结构化结果转成**给人看的文本行**。
//!
//! 与 CLI 的渲染是同一套语义（`✓ / ✗ / ⚠ / -` 标记），只是输出到界面里。
//! 桌面端不复用 CLI 的渲染代码——那边是 println，这边要返回字符串数组。

import type {
  CloseOutcome,
  DoneOutcome,
  MergeOutcome,
  MrPlan,
  PullOutcome,
  PushOutcome,
  RepoOutcome,
  SyncMainOutcome,
  SyncOutcome,
} from "./types";
import { outcomeKind as kind, outcomeReason as reason } from "./types";

export function repoLines(rs: RepoOutcome[]): string[] {
  return rs.map((r) => {
    switch (kind(r.outcome)) {
      case "ok":
        return `✓ ${r.name}`;
      case "skipped":
        return `- ${r.name}  ${reason(r.outcome)}`;
      default:
        return `✗ ${r.name}  ${reason(r.outcome)}`;
    }
  });
}

export function pushLines(o: PushOutcome): string[] {
  const out: string[] = [];
  if (o.behind.length) {
    out.push("✗ 本地落后于远程，无法推送：");
    for (const [m, n] of o.behind) out.push(`     ${m}: 落后 ${n} 个提交`);
    out.push("");
    out.push("  先同步他人的提交：拉取（Pull）");
    return out;
  }
  for (const m of o.pushed) out.push(`✓ ${m}`);
  for (const [m, n] of o.dirty) out.push(`⚠ ${m}: 有 ${n} 个未提交改动，先提交`);
  for (const [m, why] of o.failed) out.push(`✗ ${m} 推送失败: ${why}`);
  out.push(...credentialHint(o.failed.map(([, why]) => why)));
  if (!out.length) out.push("没有需要推送的模块");
  else if (o.pushed.length) out.push(`\n✓ 已推送到远程（${o.pushed.length} 个模块）`);
  return out;
}

export function pullLines(o: PullOutcome): string[] {
  const out: string[] = [];
  for (const [m, n] of o.updated) out.push(`✓ ${m}  (${n} 个提交)`);
  for (const [m, why] of o.skipped) out.push(`- ${m}  ${why}`);
  out.push(...credentialHint(o.skipped.map(([, why]) => why)));
  for (const [m, n] of o.dirty) out.push(`⚠ ${m}: 有 ${n} 个未提交改动，未拉取`);
  if (o.conflicts.length) {
    out.push("");
    out.push(`✗ 冲突待处理 (${o.conflicts.length})：`);
    for (const m of o.conflicts) out.push(`     ${m}`);
    out.push("");
    out.push("  到「模块」页用「终端 / IDE」打开该模块目录解决，然后提交");
    return out;
  }
  if (!out.length) out.push("✓ 已是最新，无需更新");
  return out;
}

export function mergeLines(o: MergeOutcome): string[] {
  const out: string[] = [];
  for (const m of o.stale) out.push(`⚠ ${m}: 远程环境分支曾被重建，本地副本已重置并重新合并`);
  for (const [m, n] of o.pending) out.push(`~ ${m}  (${n} 个提交待推)`);
  for (const m of o.clean) out.push(`✓ ${m}`);
  for (const [m, why] of o.skipped) out.push(`- ${m}  ${why}`);
  for (const [m, why] of o.push_failed) out.push(`✗ ${m} 推送失败: ${why}`);

  if (o.conflicts.length) {
    out.push("");
    out.push(`✗ 冲突待处理 (${o.conflicts.length})：`);
    for (const m of o.conflicts) {
      out.push(`     ${m}`);
      const dir = o.conflict_dirs[m];
      if (dir) out.push(`      ${dir}`);
    }
    out.push("");
    out.push(
      `  下面可直接用编辑器打开对应目录解决冲突，然后重新点一次「合并到 ${o.env}」收尾`,
    );
    return out;
  }

  if (o.did_push) {
    out.push("");
    out.push(
      o.pushed.length
        ? `✓ 已合入并推送 ${pushedTargets(o)}（${o.pushed.length} 个模块）`
        : "⚠ 没有任何模块可推送（见上方跳过项）",
    );
  } else if (o.clean.length || o.pending.length) {
    out.push("");
    out.push(`  已合并到本地 envs/${o.env}，确认无误后再点一次并勾选「推送到远程」`);
  } else {
    out.push("");
    out.push(`⚠ 没有任何模块合入 ${o.env}（${o.skipped.length} 个被跳过）`);
    out.push("     跳过多为「该仓库没有该环境的分支」，属正常：不是每个仓库都参与每个环境");
  }
  return out;
}

/**
 * 「推到了哪条分支」。
 *
 * **不能用环境标识**（`o.env`）：环境标识是 `pre`，而某仓库在这个环境里实际用的
 * 分支可能是 `pre-202610`（覆盖表里写的）—— 拿标识去报，人看到的和实际发生的
 * 对不上（实测反馈：明明设的是 `pre-202610`，提示却说 `origin/pre`）。
 *
 * 各模块分支不同时**逐个列出来**，不挑一个代表 —— 那会再次让人误以为"全都合到这一条"。
 */
function pushedTargets(o: MergeOutcome): string {
  const bs = [...new Set(o.pushed.map((m) => o.branches?.[m] ?? o.env))];
  if (bs.length <= 2) return bs.map((b) => `origin/${b}`).join(" 和 ");
  return `${bs.length} 条分支：${bs.map((b) => `origin/${b}`).join("、")}`;
}

/**
 * 跳过原因里带"要凭据"的迹象时，补一句**下一步**。
 *
 * 实测反馈：没配 GitLab token 时拉取失败，界面上只有一句 git 的英文报错
 * （`could not read Username for 'https://…'`），人不知道该去哪儿配。
 */
function credentialHint(reasons: string[]): string[] {
  const hit = reasons.some(
    (r) => /could not read Username|Authentication failed|401|403/i.test(r),
  );
  return hit
    ? ["  → 这多半是没配 GitLab 凭据：到「设置」页填一下 token（需要 api 权限），", "     或先在你自己的终端里 clone 一次这个仓库，让 git 记住凭据"]
    : [];
}

export function syncMainLines(o: SyncMainOutcome): string[] {
  const out: string[] = [];
  for (const [m, note] of o.downgraded) out.push(`⚠ ${m}: ${note}`);
  for (const [m, note] of o.updated) out.push(`✓ ${m}  (${note})`);
  for (const [m, why] of o.blocked) {
    out.push(`✗ ${m}  未开始合并`);
    for (const l of why.split("\n")) out.push(`      ${l}`);
  }
  if (o.conflicts.length) {
    out.push("");
    out.push(`⚠ 合并冲突: ${o.conflicts.join("、")}`);
    out.push("  到模块目录解决后提交。你原先未提交的改动被 git 暂存了，commit 完会自动回来");
  }
  if (o.restash_conflicts.length) {
    out.push("");
    out.push(`⚠ 改动回填冲突: ${o.restash_conflicts.join("、")}`);
    out.push("  来源已经合进来了，是你未提交的改动和它撞在同一处");
    out.push("  解决后 git add 即可（不用 commit），再 git stash drop 清掉残留条目");
  }
  if (!o.conflicts.length && !o.restash_conflicts.length && !o.blocked.length) {
    out.push(`✓ 全部已同步 ${o.from.join("、")}`);
  }
  return out;
}

export function syncLines(o: SyncOutcome): string[] {
  const out: string[] = [];
  for (const n of o.repos_updated) out.push(`✓ repos/${n}`);
  for (const [n, why] of o.repo_failures) out.push(`✗ repos/${n}  ${why}`);
  if (o.env_created.length) {
    out.push("");
    out.push(`补建环境 worktree（${o.env_created.length} 个）`);
    for (const [e, m, b] of o.env_created) out.push(`     ${e}/${m}  ← origin/${b}`);
  }
  if (o.env_moved.length) {
    out.push("");
    out.push("环境更新");
    for (const [e, m, d] of o.env_moved) out.push(`     ${e}/${m}  ${d}`);
  }
  for (const [n, why] of o.env_failures) out.push(`✗ envs/${n}  ${why}`);
  if (!o.repo_failures.length && !o.env_failures.length) {
    out.push("");
    out.push("✓ 全部同步完成");
  }
  return out;
}

export function doneLines(o: DoneOutcome): string[] {
  const out: string[] = [];
  const total = o.merged.length + o.unmerged.length;

  if (o.merged.length) {
    out.push(`已并入主干（${o.merged.length}/${total}）：`);
    for (const m of o.merged) out.push(`     ✓ ${m}`);
  }
  if (!o.unmerged.length) {
    out.push("");
    out.push("✓ 全部模块都已并入主干，这个需求可以删掉了");
    return out;
  }

  if (out.length) out.push("");
  out.push(`尚未并入主干（${o.unmerged.length}/${total}）：`);
  for (const [m, n] of o.unmerged) {
    // n=0 是「连领先多少都读不到」，别写成「0 个提交」骗人
    out.push(n === 0 ? `     ${m}: 读不到与主干的差异` : `     ${m}: 领先主干 ${n} 个提交`);
  }
  out.push("");
  out.push("  主干合并走「提 MR」，评审通过后才算并入");
  return out;
}

export function closeLines(o: CloseOutcome): string[] {
  const out: string[] = [];
  for (const [m, url] of o.closed) out.push(`✓ ${m} 已关闭  ${url}`);
  for (const [m, why] of o.skipped) out.push(`- ${m}  ${why}`);
  for (const [m, why] of o.failed) out.push(`✗ ${m}  ${why}`);
  if (o.closed.length) {
    out.push("");
    out.push("  注意：只关闭了 MR，源分支仍在（删分支是另一件事）");
  }
  return out;
}

/** MR 的固定格式清单（可直接复制发给评审人）。 */
export function mrManifest(plan: MrPlan): string {
  const lines: string[] = [];
  lines.push(plan.title, "");

  for (const side of ["Backend", "Frontend"] as const) {
    const rows = plan.rows.filter((r) => r.side === side && urlOf(r));
    if (!rows.length) continue;
    const label = side === "Backend" ? "后端" : "前端";
    lines.push(`【${label} · ${rows[0].group}】`);
    for (const r of rows) {
      lines.push(`\`${r.module}\` ${r.files} 文件 →${r.target_branch}`);
      lines.push(`${urlOf(r)}`, "");
    }
  }

  lines.push(`源分支统一：\`${plan.branch}\``, "");

  const names: string[] = [];
  for (const side of ["Backend", "Frontend"] as const) {
    for (const r of plan.rows.filter((x) => x.side === side && urlOf(x))) {
      if (r.reviewer_name && !names.includes(r.reviewer_name)) names.push(r.reviewer_name);
    }
  }
  if (names.length) lines.push(names.map((n) => `@${n}`).join(" "));

  return lines.join("\n");
}

function urlOf(r: { created: { web_url: string } | null; existing: { web_url: string } | null }): string {
  return r.created?.web_url || r.existing?.web_url || "";
}
// ── 危险操作的预览与结果 ───────────────────────────────────────────────
//
// 预览的职责是**让人判断该不该做**，所以要说清「会丢什么」而不是「确认吗」。

import type {
  PruneOutcome,
  PrunePlan,
  RenameOutcome,
  RenamePlan,
  ResetOutcome,
  ResetPlan,
  RevertOutcome,
  RevertPlan,
} from "./types";

export function revertPlanLines(p: RevertPlan): string[] {
  if (!p.modules.length) return ["没有未提交的改动"];
  const out = [`将撤销 ${p.modules.length} 个模块的改动：`];
  for (const m of p.modules) {
    out.push(`  ${m.module}  （${m.modified.length} 处改动，${m.untracked.length} 个未跟踪文件）`);
    if (m.untracked.length) {
      // 未跟踪文件没进过 git，撤销就是永久删除——必须显眼
      out.push("    ⚠ 未跟踪文件会被永久删除（git 里没有记录）：");
      m.untracked.slice(0, 20).forEach((f) => out.push(`       ${f}`));
      if (m.untracked.length > 20) out.push(`       …另有 ${m.untracked.length - 20} 个`);
    }
  }
  out.push("", "node_modules / target / .idea 会保留，不会被清掉。");
  return out;
}

export function revertResultLines(o: RevertOutcome): string[] {
  const out: string[] = [];
  for (const [m, n] of o.reverted) out.push(`✓ ${m}  已撤销 ${n} 处改动`);
  for (const [m, why] of o.failed) out.push(`✗ ${m}  ${why}`);
  if (!out.length) out.push("没有需要撤销的改动");
  return out;
}

/** Rust 侧 `total_local_commits()` 是方法不是字段，前端自己算。 */
function totalCommits(p: ResetPlan): number {
  return p.modules.reduce((s, m) => s + m.local_commits, 0);
}

export function resetPlanLines(p: ResetPlan): string[] {
  const out: string[] = [];
  if (p.modules.length) {
    out.push(
      `将把 ${p.modules.length} 个模块回退到远程分支，丢弃 ${totalCommits(p)} 个本地提交：`,
    );
    for (const m of p.modules) {
      const bits = [`→ ${m.target}`];
      if (m.local_commits) bits.push(`丢弃 ${m.local_commits} 个本地提交`);
      if (m.dirty_files) bits.push(`还原 ${m.dirty_files} 处未提交改动`);
      if (!m.local_commits && !m.dirty_files) bits.push("本来就和远程一致");
      out.push(`  ${m.module}  ${bits.join("，")}`);
    }
  }
  for (const [m, why] of p.skipped) out.push(`  - ${m}  ${why}`);
  if (!out.length) out.push("没有可回退的模块");
  return out;
}

export function resetResultLines(o: ResetOutcome): string[] {
  const out: string[] = [];
  for (const [m, t] of o.reset) out.push(`✓ ${m}  → ${t}`);
  for (const [m, why] of o.skipped) out.push(`- ${m}  ${why}`);
  for (const [m, why] of o.failed) out.push(`✗ ${m}  ${why}`);
  return out;
}

export function prunePlanLines(p: PrunePlan): string[] {
  const out: string[] = [];
  if (p.safe.length) {
    out.push(`可安全删除的远程分支（${p.safe.length} 个）：`);
    for (const s of p.safe) out.push(`  ${s.module}  ${s.project}/${s.remote_branch}`);
  }
  if (p.risky.length) {
    out.push("", `⚠ 不建议删（${p.risky.length} 个）：`);
    for (const [s, why] of p.risky) out.push(`  ${s.module}  ${why}`);
  }
  for (const [m, why] of p.skipped) out.push(`  - ${m}  ${why}`);
  if (!out.length) out.push("没有需要清理的远程分支");
  return out;
}

export function pruneResultLines(o: PruneOutcome): string[] {
  const out: string[] = [];
  for (const [m, b] of o.deleted) out.push(`✓ ${m}  已删除远程分支 ${b}`);
  for (const [m, why] of o.skipped) out.push(`- ${m}  ${why}`);
  for (const [m, why] of o.failed) out.push(`✗ ${m}  ${why}`);
  return out;
}

export function renamePlanLines(p: RenamePlan): string[] {
  const out = [`分支名: ${p.old_branch} → ${p.new_branch}`];
  if (p.modules.length) {
    out.push("", `将处理 ${p.modules.length} 个模块：`);
    for (const m of p.modules) out.push(`  ${m.module}  ${m.project}/${m.remote_branch}`);
  }
  for (const x of p.problems) out.push("", `✗ ${x}`);
  out.push(...p.notice);
  return out;
}

export function renameResultLines(o: RenameOutcome): string[] {
  const out: string[] = [];
  for (const m of o.renamed) out.push(`✓ ${m}  本地分支已改名`);
  for (const m of o.pushed) out.push(`✓ ${m}  已推送新分支`);
  for (const m of o.deleted_remote) out.push(`✓ ${m}  已删除远程旧分支`);
  for (const [m, why] of o.failed) out.push(`✗ ${m}  ${why}`);
  out.push(...o.notice);
  return out;
}
