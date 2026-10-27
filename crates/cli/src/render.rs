//! 把 core 返回的结构化结果渲染成人类可读文本。
//!
//! **只在 CLI 里做渲染。** core 不打印任何东西（设计方案 §1 的硬约束），
//! 桌面端拿到的是同一批结构体，换成图形组件渲染。这也是本版取代旧版
//! 「前端正则解析 ANSI 彩色文本」的关键。
//!
//! 全部使用 `✓ / ✗ / ⚠ / ~ / -` 标记而非颜色：彩色文本在重定向到文件、
//! CI 日志、`less` 里都会变成乱码，而标记在哪里都能读。

use std::path::Path;

use gws_core::repo::{Outcome, RepoOutcome};
use gws_core::workspace::lifecycle::{
    CleanOutcome, DropMissingOutcome, NewOutcome, PrecheckFailure, RemoveOutcome,
};
use gws_core::workspace::mr::{CloseOutcome, MrPlan, Side};
use gws_core::workspace::sync::{
    DoneOutcome, MergeOutcome, PullOutcome, PushOutcome, StatusReport, SyncMainOutcome, SyncOutcome,
};
use gws_core::workspace::{Workspace, WorkspaceMeta};

pub fn ok(msg: impl AsRef<str>) {
    println!("✓ {}", msg.as_ref());
}

pub fn warn(msg: impl AsRef<str>) {
    println!("⚠ {}", msg.as_ref());
}

pub fn info(msg: impl AsRef<str>) {
    println!("▸ {}", msg.as_ref());
}

/// 字符串的终端显示宽度：CJK 字符占 2 列。
///
/// `{:<w$}` 只数字符个数，"仓库"（2 字符 4 列）会被当成宽度 2，表格随后就歪了。
fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| {
            let cp = c as u32;
            // 常见宽字符区间：CJK、全角标点、谚文
            let wide = (0x1100..=0x115F).contains(&cp)
                || (0x2E80..=0xA4CF).contains(&cp)
                || (0xAC00..=0xD7A3).contains(&cp)
                || (0xF900..=0xFAFF).contains(&cp)
                || (0xFE30..=0xFE6F).contains(&cp)
                || (0xFF00..=0xFF60).contains(&cp)
                || (0xFFE0..=0xFFE6).contains(&cp);
            if wide {
                2
            } else {
                1
            }
        })
        .sum()
}

/// 按显示宽度右侧补空格。
fn pad(s: &str, width: usize) -> String {
    let w = display_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - w))
    }
}

/// 打印一张对齐的表。
///
/// **所有列一律按显示宽度（CJK 算 2 列）算**，表头与数据行走同一套逻辑。
/// 用 `{:<w$}` / `{:>w$}` 会按**字符个数**补齐——中文列的宽度立刻算错，
/// 而且表头与数据行的错位量还不一样（右对齐列尤其明显）。所以自己算。
fn table(headers: &[&str], rows: &[Vec<String>]) {
    let cols = headers.len();
    let mut widths: Vec<usize> = headers.iter().map(|h| display_width(h)).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < cols {
                widths[i] = widths[i].max(display_width(cell));
            }
        }
    }
    let line = |cells: &[String]| -> String {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| {
                if i + 1 == cols {
                    c.clone()
                } else {
                    pad(c, widths[i])
                }
            })
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };
    println!(
        "{}",
        line(&headers.iter().map(|h| (*h).to_string()).collect::<Vec<_>>())
    );
    for row in rows {
        println!("{}", line(row));
    }
}

// ── 仓库 ──────────────────────────────────────────────────────────────

pub fn repo_outcomes(results: &[RepoOutcome]) {
    for r in results {
        match &r.outcome {
            Outcome::Ok => ok(&r.name),
            Outcome::Skipped(why) => println!("  - {}  {}", r.name, why),
            Outcome::Failed(why) => println!("  ✗ {}  {}", r.name, why),
        }
    }
}

pub fn repo_table(repos: &[(String, String)]) {
    if repos.is_empty() {
        println!("(暂无仓库，用 gws repo add <git地址> 添加)");
        return;
    }
    let rows: Vec<Vec<String>> = repos
        .iter()
        .map(|(n, m)| vec![n.clone(), m.clone()])
        .collect();
    table(&["仓库", "主干"], &rows);
    println!();
    println!("共 {} 个", repos.len());
}

// ── 工作区 ────────────────────────────────────────────────────────────

pub fn workspace_table(list: &[Workspace]) {
    if list.is_empty() {
        println!("(暂无需求，用 gws new <名称> 创建)");
        return;
    }
    let rows: Vec<Vec<String>> = list
        .iter()
        .map(|ws| {
            let n = ws.present_modules().len();
            let missing = ws.meta().modules.len() - n;
            let modules = if missing > 0 {
                format!("{n}（另有 {missing} 个目录缺失）")
            } else {
                n.to_string()
            };
            vec![ws.branch().to_string(), ws.title().to_string(), modules]
        })
        .collect();
    table(&["分支", "需求名称", "模块数"], &rows);
    println!();
    println!("共 {} 个需求", list.len());
}

pub fn new_outcome(out: &NewOutcome) {
    ok(format!("需求已创建: {}", out.branch));
    println!("   分支: {}", out.branch);
    for m in &out.created {
        match out.base_notes.get(m) {
            Some(note) => println!("   ✓ {m}  ({note})"),
            None => println!(
                "   ✓ {m}  ← {}",
                out.base_resolved.get(m).map(String::as_str).unwrap_or("?")
            ),
        }
    }
    println!();
    println!("   下一步: cd ws/{}", out.branch);
}

pub fn remove_outcome(out: &RemoveOutcome) {
    if !out.risks.is_empty() && !out.forced {
        warn("存在未保存的工作，未删除：");
        for r in &out.risks {
            println!("     ✗ {}: {}", r.module, r.reason);
        }
        println!();
        println!("  确认丢弃请加 --force，或先 gws push");
        return;
    }
    if out.forced {
        // --force 下有风险项仍然删了：必须说清"已经删掉、这些东西没了"，
        // 而不是复用上面那段"未删除"的文案
        warn(format!(
            "已强制删除 {}（丢弃了 {} 项未保存的工作）",
            out.branch,
            out.risks.len()
        ));
        for r in &out.risks {
            println!("     - {}: {}", r.module, r.reason);
        }
    } else {
        ok(format!("需求 {} 已删除", out.branch));
    }
    if !out.removed_modules.is_empty() {
        println!("   已移除模块: {}", out.removed_modules.join(", "));
    }
}

pub fn clean_outcome(out: &CleanOutcome, apply: bool) {
    if out.leftovers.is_empty() {
        ok("ws/ 下没有残留目录");
        return;
    }
    if !apply {
        warn(format!(
            "发现 {} 个残留目录（没有 .workspace.json）：",
            out.leftovers.len()
        ));
        for p in &out.leftovers {
            println!("     {}", p.display());
        }
        println!();
        println!("  确认清理请加 --apply");
        return;
    }
    for p in &out.removed {
        ok(format!("已清理 {}", p.display()));
    }
    for (p, why) in &out.failed {
        println!("  ✗ {} 清理失败: {}", p.display(), why);
    }
}

pub fn drop_missing_outcome(out: &DropMissingOutcome) {
    if out.cleaned.is_empty() && out.kept.is_empty() {
        info("没有需要清理的缺失模块");
        return;
    }
    for m in &out.cleaned {
        ok(format!("{m}: 目录已删除，已清理 worktree 与分支"));
    }
    for (m, why) in &out.kept {
        warn(format!(
            "{m}: 有未推送提交（{why}），已保留分支，可用 gws add {m} 找回"
        ));
    }
}

pub fn precheck_failure(f: &PrecheckFailure) {
    eprint!("{f}");
}

// ── 状态 ──────────────────────────────────────────────────────────────

pub fn status_report(rep: &StatusReport) {
    let rows: Vec<Vec<String>> = rep
        .modules
        .iter()
        .map(|m| {
            let (dirty, vs_remote, vs_main) = if let Some(err) = &m.error {
                ("读取失败".to_string(), err.clone(), "-".to_string())
            } else if !m.present {
                (
                    "目录缺失".to_string(),
                    "gws drop 清理".to_string(),
                    "-".to_string(),
                )
            } else {
                let vs_remote = if m.pushed {
                    format!("↑{} ↓{}", m.ahead, m.behind)
                } else {
                    "未推送".to_string()
                };
                let vs_main = match m.ahead_of_main {
                    Some(n) => format!("+{n}"),
                    None => "+?".to_string(),
                };
                (m.dirty.to_string(), vs_remote, vs_main)
            };
            vec![m.module.clone(), dirty, vs_remote, vs_main]
        })
        .collect();
    table(&["模块", "改动", "vs远程", "vs主干"], &rows);
}

/// 环境列表：标识 / 显示名 / 分支名覆盖。
pub fn env_table(envs: &[gws_core::env::EnvDef]) {
    if envs.is_empty() {
        println!("(暂无环境，用 gws env add <标识> 创建)");
        return;
    }
    let rows: Vec<Vec<String>> = envs
        .iter()
        .map(|e| {
            let overrides = if e.branches.is_empty() {
                "(全部同名)".to_string()
            } else {
                e.branches
                    .iter()
                    .map(|(r, b)| format!("{r}={b}"))
                    .collect::<Vec<_>>()
                    .join("  ")
            };
            vec![e.id.clone(), e.label.clone(), overrides]
        })
        .collect();
    table(&["环境标识", "显示名", "分支名覆盖"], &rows);
}

// ── 同步与合并 ────────────────────────────────────────────────────────

pub fn push_outcome(out: &PushOutcome) {
    if !out.behind.is_empty() {
        eprintln!("✗ 本地落后于远程，无法推送：");
        for (m, n) in &out.behind {
            eprintln!("     {m}: 落后 {n} 个提交");
        }
        eprintln!();
        eprintln!("  先同步他人的提交: gws pull");
        return;
    }
    for m in &out.pushed {
        ok(m);
    }
    for (m, n) in &out.dirty {
        warn(format!("{m}: 有 {n} 个未提交改动，先提交"));
    }
    for (m, why) in &out.failed {
        println!("  ✗ {m} 推送失败: {why}");
    }
    if out.pushed.is_empty() && out.dirty.is_empty() && out.failed.is_empty() {
        info("没有需要推送的模块");
    } else if !out.pushed.is_empty() {
        println!();
        ok("已推送到远程");
    }
}

pub fn pull_outcome(out: &PullOutcome) {
    for (m, n) in &out.updated {
        ok(format!("{m}  ({n} 个提交)"));
    }
    for (m, why) in &out.skipped {
        println!("  - {m}  {why}");
    }
    for (m, n) in &out.dirty {
        warn(format!(
            "{m}: 有 {n} 个未提交改动，未拉取（先提交或 stash）"
        ));
    }
    if !out.conflicts.is_empty() {
        println!();
        eprintln!("✗ 冲突待处理 ({}):", out.conflicts.len());
        for m in &out.conflicts {
            eprintln!("     {m}");
        }
        eprintln!();
        eprintln!(
            "  在 IDE 中打开 ws/<需求>/{}/ 解决后 Commit 即可",
            out.conflicts[0]
        );
        return;
    }
    if out.updated.is_empty() && out.dirty.is_empty() {
        ok("已是最新，无需更新");
    }
}

pub fn sync_main_outcome(out: &SyncMainOutcome, ws: &Workspace) {
    for (m, note) in &out.downgraded {
        warn(format!("{m}: {note}"));
    }
    for (m, note) in &out.updated {
        ok(format!("{m}  ({note})"));
    }
    for (m, why) in &out.blocked {
        println!("  ✗ {m}  未开始合并");
        for line in why.lines() {
            println!("      {line}");
        }
    }
    if !out.conflicts.is_empty() {
        println!();
        eprintln!("⚠ 合并冲突: {}", out.conflicts.join(", "));
        eprintln!("  打开 ws/{}/<模块> 解决后 git commit", ws.branch());
        eprintln!("  你原先未提交的改动被 git 暂存起来了，commit 完会自动回到工作树");
    }
    if !out.restash_conflicts.is_empty() {
        println!();
        eprintln!("⚠ 改动回填冲突: {}", out.restash_conflicts.join(", "));
        eprintln!("  来源已经合进来了，是你未提交的改动和它撞在同一处");
        eprintln!("  在 IDE 里解决后 git add 即可（不用 commit），再 git stash drop 清掉残留条目");
    }
    if out.is_clean() {
        ok(format!("全部已同步 {}", out.from.join("、")));
    }
}

pub fn merge_outcome(out: &MergeOutcome) {
    for m in &out.stale {
        warn(format!(
            "{m}: 远程环境分支曾被重建，本地副本作废，已重置到最新并重新合并"
        ));
    }
    for (m, n) in &out.pending {
        println!("  ~ {m}  ({n} 个提交待推)");
    }
    for m in &out.clean {
        ok(m);
    }
    for (m, why) in &out.skipped {
        println!("  - {m}  {why}");
    }
    for (m, why) in &out.push_failed {
        println!("  ✗ {m} 推送失败: {why}");
    }

    if out.has_conflicts() {
        println!();
        eprintln!("✗ 冲突待处理 ({}):", out.conflicts.len());
        for m in &out.conflicts {
            eprintln!("     {m}");
            if let Some(dir) = out.conflict_dirs.get(m) {
                eprintln!("      {dir}");
            }
        }
        eprintln!();
        eprintln!("  用 IDE 打开上述目录解决冲突（解决后 Commit 或不 Commit 都行），然后:");
        eprintln!(
            "    gws merge {} --push    再跑一次：补提交 + 推送，一步收尾",
            out.env
        );
        eprintln!(
            "    gws merge {}           或先合到本地 review，稍后 --push 推",
            out.env
        );
        return;
    }

    println!();
    if out.did_push {
        if out.pushed.is_empty() {
            warn("没有任何模块可推送（见上方跳过项）");
        } else {
            ok(format!(
                "已合入并推送 origin/{}（{} 个模块）",
                out.env,
                out.pushed.len()
            ));
        }
    } else if !out.clean.is_empty() || !out.pending.is_empty() {
        println!(
            "  已合并到本地 envs/{}，review 确认后再推送: gws merge {} --push",
            out.env, out.env
        );
        println!(
            "  想丢弃本地合并成果重新合并: gws merge {} --force",
            out.env
        );
    } else {
        // 一个模块都没合：要么全被跳过（该环境没这些仓库的分支），要么全在 pending。
        // 必须给一句结论——只留几行 "-" 让人不知道到底发生了什么。
        warn(format!(
            "没有任何模块合入 {}（{} 个被跳过，见上）",
            out.env,
            out.skipped.len()
        ));
        println!(
            "     跳过多为「该仓库没有 {} 分支」，属正常：不是每个仓库都参与每个环境",
            out.env
        );
        println!("     若预期所有仓库都该有，检查环境配置: gws env ls");
    }
}

pub fn sync_outcome(out: &SyncOutcome, hub: &gws_core::Hub) {
    for name in &out.repos_updated {
        ok(format!("repos/{name}"));
    }
    for (name, why) in &out.repo_failures {
        println!("  ✗ repos/{name}  {why}");
    }
    if !out.env_created.is_empty() {
        println!();
        info(format!("补建环境 worktree（{} 个）", out.env_created.len()));
        for (env, module, branch) in &out.env_created {
            println!("     {env}/{module}  ← origin/{branch}");
        }
    }
    if !out.env_moved.is_empty() {
        println!();
        info("envs 更新");
        for (env, module, desc) in &out.env_moved {
            println!("     {env}/{module}  {desc}");
        }
    }
    for (name, why) in &out.env_failures {
        println!("  ✗ envs/{name}  {why}");
    }
    if out.repo_failures.is_empty() && out.env_failures.is_empty() {
        println!();
        ok("全部同步完成");
        println!("   hub: {}", hub.root().display());
    }
}

pub fn done_outcome(out: &DoneOutcome) {
    if out.is_done() {
        ok(format!("所有模块已并入主干（共检查 {} 个）", out.total()));
        println!("   删除工作区: gws rm <需求分支名>");
        return;
    }
    if !out.merged.is_empty() {
        println!("已并入主干（{} 个）：", out.merged.len());
        for m in &out.merged {
            println!("     ✓ {m}");
        }
        println!();
    }
    eprintln!("⚠ 尚未并入主干（{} 个）：", out.unmerged.len());
    for (m, n) in &out.unmerged {
        // n=0 是「连领先多少都读不到」的情况，别写成「0 个提交」骗人
        if *n == 0 {
            eprintln!("     {m}: 读不到与主干的差异");
        } else {
            eprintln!("     {m}: 领先主干 {n} 个提交");
        }
    }
    eprintln!();
    eprintln!("  主干合并需在代码托管平台发起 Merge Request 并完成评审");
}

// ── 元数据展示 ────────────────────────────────────────────────────────

pub fn workspace_meta(ws: &Workspace, hub_root: &Path) {
    let meta: &WorkspaceMeta = ws.meta();
    println!("{}", meta.title);
    println!("  分支     {}", meta.branch);
    // 空 = 主干，但要说清是「没设」而不是「设成了主干」——
    // 后者会让人以为文件里真写了这么个值（实测反馈 #4）
    println!(
        "  基线     {}",
        if meta.base.is_empty() {
            "主干（未设置）".to_string()
        } else {
            meta.base.join(" → ")
        }
    );
    println!("  创建     {}", meta.created);
    if let Some(doc) = &meta.doc {
        println!("  文档     {}/{}", doc.family, doc.period);
    }
    println!(
        "  目录     {}",
        ws.dir()
            .strip_prefix(hub_root)
            .unwrap_or(ws.dir())
            .display()
    );
    if !meta.links.is_empty() {
        println!("  自定义信息");
        for l in &meta.links {
            if l.is_link() {
                println!("    {}  {}", l.label, l.url);
            } else {
                println!("    {}", l.label);
            }
        }
    }
    if !meta.notes.trim().is_empty() {
        println!("  备注     {}", meta.notes);
    }
}

// ── MR ────────────────────────────────────────────────────────────────

pub fn mr_plan(plan: &MrPlan) {
    info(format!(
        "{}   分支: {}",
        if plan.is_dry_run {
            "预览（加 --create 才真正创建）"
        } else {
            "创建 MR"
        },
        plan.branch
    ));
    println!(
        "   评审人: 前端={}  后端={}   [来源: {}]",
        plan.reviewers.frontend, plan.reviewers.backend, plan.reviewers.source
    );
    println!();

    let rows: Vec<Vec<String>> = plan
        .rows
        .iter()
        .map(|r| {
            let status = if let Some(e) = &r.error {
                format!("失败: {}", e.lines().next().unwrap_or(""))
            } else if r.created.is_some() {
                "已创建".to_string()
            } else if r.reused() {
                "已存在".to_string()
            } else {
                "待创建".to_string()
            };
            vec![
                r.module.clone(),
                r.target_branch.clone(),
                format!("{} 文件", r.files),
                format!("{}({})", r.side.label(), r.reviewer),
                status,
            ]
        })
        .collect();
    table(&["模块", "目标分支", "改动", "评审人", "状态"], &rows);

    for r in &plan.rows {
        if let Some(url) = r.url() {
            println!("   {}  {}", r.module, url);
        }
    }
    for (m, why) in &plan.skipped {
        println!("   - {m}  {why}");
    }

    if plan.rows.is_empty() && plan.skipped.is_empty() {
        warn("没有任何模块需要提 MR");
    } else if !plan.is_dry_run {
        println!();
        let created = plan.created_count();
        let reused = plan.reused_count();
        if created > 0 || reused > 0 {
            ok(format!("已创建 {created} 个，复用 {reused} 个"));
        }
    } else {
        println!();
        println!("   确认无误后创建: gws mr --create");
    }
}

/// 固定格式的清单（可直接复制发给评审人）。
///
/// 顺序：需求名 → 按前后端分组（仓库 + 文件数 + 链接）→ 源分支统一 →
/// 文档链接 → @中文名。文档链接那一节由 doc-hub 阶段补上。
pub fn mr_manifest(plan: &MrPlan, doc_links: &[(String, String)]) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();

    let _ = writeln!(s, "{}\n", plan.title);

    for side in [Side::Backend, Side::Frontend] {
        let group: Vec<_> = plan
            .rows
            .iter()
            .filter(|r| r.side == side && r.url().is_some())
            .collect();
        if group.is_empty() {
            continue;
        }
        let group_name = group[0].group.clone();
        let _ = writeln!(s, "【{} · {}】", side.label(), group_name);
        for r in group {
            let _ = writeln!(s, "`{}` {} 文件 →{}", r.module, r.files, r.target_branch);
            let _ = writeln!(s, "{}\n", r.url().unwrap_or(""));
        }
    }

    let _ = writeln!(s, "源分支统一：`{}`\n", plan.branch);

    if !doc_links.is_empty() {
        for (label, url) in doc_links {
            let _ = writeln!(s, "{label}：{url}");
        }
        s.push('\n');
    }

    // @中文名放最后（企微提醒）
    let names = plan.touched_reviewer_names();
    if !names.is_empty() {
        let _ = writeln!(
            s,
            "{}",
            names
                .iter()
                .map(|n| format!("@{n}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }

    s
}

pub fn mr_close(out: &CloseOutcome) {
    for (m, url) in &out.closed {
        ok(format!("{m} 已关闭  {url}"));
    }
    for (m, why) in &out.skipped {
        println!("   - {m}  {why}");
    }
    for (m, why) in &out.failed {
        println!("   ✗ {m}  {why}");
    }
    if !out.closed.is_empty() {
        println!();
        println!("   注意：只关闭了 MR，源分支仍在。清理分支见 gws branch（阶段 9）");
    }
}

// ── GitLab 凭据 ───────────────────────────────────────────────────────

/// 凭据状态。`connected` 是**具体原因**而不是「是否成功」——
/// 原来只传 Option 时，失败分支会打印「见上方错误」，而上方根本没有错误，
/// 让人不知道该去查什么。
/// 当前 hub 的 GitLab 状态。
///
/// 地址来自 hub 配置（写死不可改），凭据来自该 hub 的凭据文件或环境变量 ——
/// 所以这里要把**这个 hub 的根目录**带上，不能再去读「全局那份」。
pub fn gitlab_status(
    hub: &gws_core::hub::Hub,
    connected: Result<(String, String), String>,
    stored: bool,
) {
    use gws_core::gitlab::token;
    let url = hub.config().gitlab.base_url.clone();
    println!(
        "   地址:   {}",
        if url.trim().is_empty() { "（未绑定）" } else { url.as_str() }
    );
    // 报出来源，是为了回答「我改了地址怎么还是旧的」——通常是环境变量压过了 token
    println!("   地址来自: hub 配置（建 hub 时固定，不可改）");
    println!("   凭据来源: {}", token::source(hub.root()));
    println!(
        "   凭据文件: {}",
        if stored { "已存有 token" } else { "无" }
    );
    println!("   文件位置: {}", hub.token_file().display());
    println!();
    match connected {
        Ok((name, username)) => ok(format!("连接成功: {name} ({username})")),
        // 具体原因和下一步都已在错误信息里，不再追加一段重复的「怎么配」
        Err(why) => eprintln!("✗ 无法连接 GitLab: {why}"),
    }
}

// ── 文档（doc-hub）────────────────────────────────────────────────────

pub fn doc_attach(a: &gws_core::dochub::AttachOutcome) {
    if !a.attached {
        // `ReuseOnly`（导入）时远程没有这一期 —— 这是"这次不挂"，不是失败
        info("远程还没有这一期的文档，先不挂上（想建文档用 gws doc attach）");
        return;
    }
    if a.created {
        ok(format!("文档目录已挂上：{}", a.relative_dir));
    } else {
        info(format!("文档目录已存在：{}", a.relative_dir));
    }
    println!("   需求分类  {}   期次  {}", a.family, a.label);
}

pub fn doc_files(period: &Path, files: &[String]) {
    if files.is_empty() {
        println!("(暂无文档，用 gws doc new <类型> 创建)");
        return;
    }
    println!("{}", period.display());
    for f in files {
        println!("   {f}");
    }
    println!();
    println!("共 {} 个", files.len());
}

pub fn doc_status(pending: &[String]) {
    if pending.is_empty() {
        ok("没有待提交的文档变更");
        return;
    }
    warn(format!("{} 处待提交的文档变更：", pending.len()));
    for p in pending {
        println!("   {p}");
    }
    println!();
    println!("   提交: gws doc sync -m \"说明\"        （再加 --push 推到远程）");
}

pub fn doc_sync(out: &gws_core::dochub::DocSyncOutcome) {
    if out.changed.is_empty() {
        ok("没有变更，未提交");
        return;
    }
    for f in &out.changed {
        println!("   {f}");
    }
    if out.committed {
        ok(format!("已提交 {} 个文件", out.changed.len()));
    }
    if out.pushed {
        ok("已推送到远程文档分支");
    } else if out.committed {
        println!();
        println!("   推送到远程: gws doc sync --push");
        println!("   上线时与代码 MR 一起提文档 MR 合入 main");
    }
}

pub fn doc_families(families: &[gws_core::dochub::Family]) {
    // 与界面同一套可见性：空期次（只有脚手架）和本地打不开的期次都不列，
    // 否则「巨量单元状态同步」这种只在远程/环境里存在的条目会看着像空文档。
    let visible: Vec<_> = families
        .iter()
        .map(|f| {
            let periods: Vec<_> = f
                .periods
                .iter()
                .filter(|p| p.has_content && p.available)
                .collect();
            (f, periods)
        })
        .filter(|(_, periods)| !periods.is_empty())
        .collect();
    if visible.is_empty() {
        println!("(暂无需求文档)");
        return;
    }
    for (f, periods) in &visible {
        println!("{}", f.name);
        for p in periods.iter().copied() {
            let label = if p.meta.label.is_empty() {
                "  "
            } else {
                &p.meta.label
            };
            // 已并入主干的标「已归档」——AI 不该再改它们
            let state = if p.merged { "已归档" } else { "开发中" };
            println!(
                "   {:<7} {}  {}  {}",
                label, p.meta.date, p.meta.branch, state
            );
            if !p.meta.summary.trim().is_empty() {
                println!("           {}", p.meta.summary);
            }
        }
    }
    println!();
    println!("共 {} 个需求分类", visible.len());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_width_counts_cjk_as_two() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("仓库"), 4, "CJK 应算 2 列");
        assert_eq!(display_width("收银台改版"), 10);
        assert_eq!(display_width("feature-20261010-收银台改版"), 27);
    }

    #[test]
    fn pad_fills_to_display_width() {
        assert_eq!(pad("ab", 5), "ab   ");
        assert_eq!(display_width(&pad("仓库", 11)), 11);
        assert_eq!(display_width(&pad("收银台改版", 10)), 10);
    }
}
