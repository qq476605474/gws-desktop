//! 「把需求配置复制给别人」—— 导出成一段文本，对方粘进新建需求就建出同一个需求。
//!
//! # 为什么要这个东西
//!
//! 实测反馈里的两个场景：
//!
//! - **拉别人一起做**：我建好需求（分支名、需求分类、涉及哪些仓库、基线、常用链接），
//!   现在只能口头告诉他「分支是 `feature-xxxx`」，他自己还得再摸一遍配置
//! - **提测**：以前要把分支名告诉测试，现在直接把这段发过去，对方粘贴一下就
//!   在自己机器上建出**跟我这边一样**的需求
//!
//! 所以这段文本必须**自包含**：光有分支名不够 ——
//! 对方没有那个仓库时得能自己拉下来，所以模块要**连远程地址一起带**。
//!
//! # 格式
//!
//! 两行标记夹一段 JSON：
//!
//! ```text
//! ----- gws 需求配置（复制整段发给别人）-----
//! { "schema": 1, "branch": "...", ... }
//! ----- gws 需求配置 结束 -----
//! ```
//!
//! 为什么是 JSON 而不是「分支: xxx / 名称: yyy」那种更好看的行文本：
//! 解析不能靠猜。里面有中文名、链接、可能带换行的备注（`notes`），
//! 行文本格式遇到这些就得自己发明转义规则，一处没对齐就是静默解析错。
//! JSON 有现成的转义与校验，人也能直接看懂（缩进过的）。
//!
//! 标记行只是为了让聊天窗口里**一眼看出这是一整段配置**，
//! 解析时并不依赖它（见 [`parse`]：从第一个 `{` 配到最后一个 `}`）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::hub::Hub;
use crate::repo::Repo;
use crate::workspace::{Link, Workspace};
use crate::{Error, Result};

/// 载荷格式版本。以后改了结构靠它认。
pub const SHARE_SCHEMA: u32 = 1;

const BEGIN: &str = "----- gws 需求配置（复制整段发给别人）-----";
const END: &str = "----- gws 需求配置 结束 -----";

/// 载荷里的一个模块：**名字 + 远程地址**。
///
/// 地址是必须的 —— 对方没这个仓库时要靠它克隆。取不到地址（仓库没配 origin）
/// 时留空串，导入时会**明确报出这个模块补不了**，而不是悄悄少建一个。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedModule {
    pub name: String,
    #[serde(default)]
    pub url: String,
}

/// 一段可转发的需求配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedRequirement {
    pub schema: u32,
    /// 分支名（也是对方那边的目录名）。
    pub branch: String,
    /// 需求名称（给人看的中文名）。
    pub title: String,
    /// 基线来源链，顺序即优先级。空 = 主干兜底。
    #[serde(default)]
    pub base: Vec<String>,
    /// 需求分类。
    #[serde(default)]
    pub doc_family: Option<String>,
    /// 挂了文档库时，这里写它的仓库名（`doc-hub`）。
    ///
    /// **纯信息性字段**：文档库不能塞进 `modules` —— 它有自己的挂载流程
    /// （建期次目录、写 meta.json、认需求分类），当普通模块建 worktree 会跟那套
    /// 打架，反而挂不上。但配置文本里不写它，人一数就以为"少了一个仓库"
    /// （实测反馈：我有 4 个仓库，怎么只复制出来 3 个）。
    #[serde(default)]
    pub doc_hub: Option<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub links: Vec<Link>,
    pub modules: Vec<SharedModule>,
}

impl SharedRequirement {
    /// 渲染成可以贴在聊天窗口里的文本。
    pub fn render(&self) -> String {
        let json = serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string());
        format!("{BEGIN}\n{json}\n{END}")
    }

    /// 模块名列表（按载荷里的顺序）。
    pub fn module_names(&self) -> Vec<String> {
        self.modules.iter().map(|m| m.name.clone()).collect()
    }
}

/// 从当前工作区导出。
///
/// `urls` 是「模块名 → 远程地址」，由调用方给（core 里从 repos/ 读）；
/// 抽出来是为了能单测，不必为此造一整个 hub。
pub fn export_with(
    hub: &Hub,
    ws: &Workspace,
    urls: &BTreeMap<String, String>,
) -> SharedRequirement {
    let meta = ws.meta();
    SharedRequirement {
        schema: SHARE_SCHEMA,
        branch: meta.branch.clone(),
        title: meta.title.clone(),
        base: meta.base.clone(),
        doc_family: meta.doc.as_ref().map(|d| d.family.clone()),
        doc_hub: meta.doc.as_ref().map(|_| hub.dochub_name()),
        notes: meta.notes.clone(),
        links: meta.links.clone(),
        modules: meta
            .modules
            .iter()
            .map(|name| SharedModule {
                name: name.clone(),
                url: urls.get(name).cloned().unwrap_or_default(),
            })
            .collect(),
    }
}

/// 导出某工作区的配置。
pub fn export(hub: &Hub, ws: &Workspace) -> Result<SharedRequirement> {
    let mut urls = BTreeMap::new();
    for m in &ws.meta().modules {
        // 读不到就留空（下一步会明确报「这个模块补不了」），不该让导出整个失败
        let url = Repo::open(hub, m).ok().and_then(|r| r.remote_url()).unwrap_or_default();
        urls.insert(m.clone(), url);
    }
    Ok(export_with(hub, ws, &urls))
}

/// 从粘贴进来的文本里解析出配置。
///
/// **容错**：聊天窗口可能把标记行吃掉、或者前后多出「好的，配置如下：」这类话，
/// 所以定位方式是**从第一个 `{` 配到最后一个 `}`**，标记行只当参考。
/// 但只要内容本身不对（缺 `branch`、版本不认识），就明确报错 ——
/// 一份解析歪掉的配置会建出一个错的需求，那比报错难查得多。
pub fn parse(text: &str) -> Result<SharedRequirement> {
    let start = text.find('{').ok_or_else(|| {
        Error::other("这段内容里没有找到需求配置（应以 `{` 开头的一段 JSON）。\n\n确认复制的是完整的一段：从「----- gws 需求配置」到「结束」都要")
    })?;
    let end = text.rfind('}').ok_or_else(|| Error::other("这段内容不完整：JSON 没有闭合的 `}`"))?;
    if end < start {
        return Err(Error::other("这段内容不完整：`}` 出现在 `{` 之前"));
    }
    let json = &text[start..=end];

    let shared: SharedRequirement = serde_json::from_str(json)
        .map_err(|e| Error::other(format!("这段配置读不出来（可能复制少了或改了内容）: {e}")))?;
    validate(&shared)?;
    Ok(shared)
}

/// 一段配置是不是能用来建需求。
///
/// 单独抽出来是因为**两条路都要过它**：从文本解析出来的（别人发来的），
/// 和界面递进来的（人在预览里可能改过中文名/备注）。校验只有一处，
/// 界面那份改动不会绕过它。
pub fn validate(s: &SharedRequirement) -> Result<()> {
    if s.schema != SHARE_SCHEMA {
        return Err(Error::other(format!(
            "这段配置的格式版本是 {}，本程序支持 {}。让对方用当前版本的程序重新复制一次",
            s.schema, SHARE_SCHEMA
        )));
    }
    if s.branch.trim().is_empty() {
        return Err(Error::other("这段配置里没有分支名"));
    }
    crate::name::validate_component(&s.branch, "分支名")?;
    if s.modules.is_empty() {
        return Err(Error::other(format!(
            "这段配置里一个模块都没有。让对方在需求详情里重新复制一次（{}）",
            s.branch
        )));
    }
    for m in &s.modules {
        crate::name::validate_component(&m.name, "模块名")?;
    }
    Ok(())
}

/// 导入一份配置的结果。
#[derive(Debug, Clone, Serialize)]
pub struct ImportOutcome {
    pub branch: String,
    pub title: String,
    /// 这次补上的仓库（本地原本没有，按配置里的地址拉的）。
    pub added_repos: Vec<String>,
    /// 建出来的模块。
    pub modules: Vec<String>,
    /// 复用了本地已有同名分支的模块。
    pub reused: Vec<String>,
    /// 配置里写了、但这个仓库的远程没有该分支（正常——这个需求没改动它）。
    pub not_involved: Vec<String>,
}

/// 把别人发来的配置在这台机器上建出来。
///
/// 两步，顺序不能反：
///
/// 1. **先把缺的仓库补上**（按配置里的地址克隆）—— 工作区是「模块的 worktree
///    集合」，仓库不在，模块就无从建起。这一步失败就**整体停下**：宁可什么都不建，
///    也不能建出一个「少了两个模块」的需求，那跟其他人那边就不是一回事了
/// 2. 再按配置把工作区拉出来（等价于「导入需求」，但模块集合、基线、中文名、
///    需求分类**都以那份配置为准**，不再靠扫全部仓库反推）
///
/// 环境里的检出（`envs/`）由调用方在拿到 `added_repos` 后补 —— 那是 UI 层的
/// 编排（`ensure_envs_for`），core 不做。
pub fn import(
    hub: &Hub,
    shared: &SharedRequirement,
    cb: crate::progress::Progress<'_>,
) -> Result<ImportOutcome> {
    let known: std::collections::BTreeSet<String> =
        crate::repo::list(hub)?.into_iter().map(|r| r.name).collect();

    // ① 本地没有的仓库：按配置里的地址补上
    let mut urls: Vec<String> = Vec::new();
    let mut no_url: Vec<String> = Vec::new();
    for m in &shared.modules {
        if known.contains(&m.name) {
            continue;
        }
        if m.url.trim().is_empty() {
            no_url.push(m.name.clone());
            continue;
        }
        urls.push(m.url.trim().to_string());
    }
    if !no_url.is_empty() {
        return Err(Error::other(format!(
            "这些模块你本地没有，而这份配置里又没带它们的地址，补不了：{}\n\n\
             让对方在需求详情里**重新复制一次**配置（复制时会带上每个模块的地址）",
            no_url.join("、")
        )));
    }

    let mut added_repos = Vec::new();
    if !urls.is_empty() {
        let results = crate::repo::add(hub, &urls, cb)?;
        let failures: Vec<String> = results
            .iter()
            .filter_map(|r| match &r.outcome {
                crate::repo::Outcome::Failed(why) => Some(format!("  ✗ {}: {}", r.name, first_line(why))),
                _ => None,
            })
            .collect();
        if !failures.is_empty() {
            let joined = failures.join("\n");
            // 失败原因要**分开说**：认证失败和网络不通的下一步完全不同，
            // 一律说"修好网络"会让人白折腾（实测反馈里对误导性提示很敏感）。
            let hint = if joined.contains("could not read Username")
                || joined.contains("Authentication failed")
                || joined.contains("401")
            {
                "这些仓库不允许匿名克隆。\n  · 到「设置」页确认 GitLab token 已配置（需要 api 权限）\n  · 或者先在你自己的终端里 clone 一次，让 git 记住凭据"
            } else if joined.contains("not found") || joined.contains("404") || joined.contains("Repository not found") {
                "多半是地址写错、或那个仓库已改名/删除 —— 让对方重新复制一次配置。"
            } else {
                "确认网络 / VPN 通不通（能打开 GitLab 网页一般就没问题），然后重试。"
            };
            return Err(Error::other(format!(
                "补建仓库失败，需求**没有**创建（不会留下半成品）：\n{joined}\n\n{hint}"
            )));
        }
        added_repos = results.iter().map(|r| r.name.clone()).collect();
    }

    // ② 按配置建工作区
    let opts = crate::workspace::lifecycle::GetOptions {
        branch: shared.branch.clone(),
        title: shared.title.clone(),
        doc_family: shared.doc_family.clone(),
        notes: shared.notes.clone(),
        links: shared.links.clone(),
        base: shared.base.clone(),
        modules: Some(shared.module_names()),
    };
    let out = crate::workspace::lifecycle::get(hub, &opts, cb).map_err(|e| {
        let msg = e.to_string();
        // 实测反馈：这份配置是对方发来的，最常见的失败就是**对方还没推** ——
        // 分支只在对方本地，这边当然"没有任何仓库存在该分支"。
        // `get` 的原始提示是针对"自己敲分支名"的场景写的（先怀疑名字敲错），
        // 在这里会把人的注意力带偏，所以补上这个场景的下一步。
        if msg.contains("没有任何仓库存在远程分支") {
            Error::other(format!(
                "{msg}\n\n这份配置是对方发来的，所以最常见的原因是：**对方还没把这条分支推到远程**。\n\
                 让对方在自己的需求里点一次「推送」，再把配置重新复制一份发给你。"
            ))
        } else {
            e
        }
    })?;

    Ok(ImportOutcome {
        branch: out.branch,
        title: shared.title.clone(),
        added_repos,
        modules: out.modules,
        reused: out.reused,
        not_involved: out.not_involved,
    })
}

/// 报错信息只留第一行 —— git 的 stderr 常常是多行，塞进一句总结里没人看。
fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SharedRequirement {
        SharedRequirement {
            schema: SHARE_SCHEMA,
            branch: "feature-20261013-private-agent-v3".into(),
            title: "示例需求v1.3".into(),
            base: vec!["ws:feature-20260901-smart-reach".into()],
            doc_family: Some("示例需求".into()),
            doc_hub: None,
            notes: "第一行\n第二行".into(),
            links: vec![Link::new("需求", "https://tapd.example/1")],
            modules: vec![
                SharedModule { name: "demo-api".into(), url: "https://git/x/a.git".into() },
                SharedModule { name: "demo-payment".into(), url: "https://git/y/p.git".into() },
            ],
        }
    }

    #[test]
    fn round_trips_through_text() {
        let s = sample();
        let text = s.render();
        let back = parse(&text).unwrap();
        assert_eq!(back, s, "导出再导入必须一模一样");
        // 备注里的换行、中文、链接都不能丢
        assert_eq!(back.notes, "第一行\n第二行");
        assert_eq!(back.links[0].url, "https://tapd.example/1");
    }

    #[test]
    fn parses_even_if_the_marker_lines_got_eaten() {
        // 聊天窗口里粘贴时常常只剩 JSON 本体
        let s = sample();
        let json = serde_json::to_string_pretty(&s).unwrap();
        assert_eq!(parse(&json).unwrap(), s);
        // 前后多出人话也要认
        assert_eq!(parse(&format!("好的，配置如下：\n{json}\n谢谢")).unwrap(), s);
    }

    #[test]
    fn rejects_garbage_with_a_readable_reason() {
        assert!(parse("你好").unwrap_err().to_string().contains("没有找到需求配置"));
        assert!(parse("{\"branch\":").unwrap_err().to_string().contains("不完整"));
        let e = parse("{\"schema\":99,\"branch\":\"b\",\"title\":\"t\",\"modules\":[{\"name\":\"m\"}]}")
            .unwrap_err()
            .to_string();
        assert!(e.contains("格式版本"), "{e}");
    }

    #[test]
    fn rejects_a_bad_branch_or_empty_modules() {
        let mut s = sample();
        s.branch = "a/b".into(); // 分支名不能有斜杠（目录名就是它）
        let e = parse(&s.render()).unwrap_err().to_string();
        assert!(e.contains("分支名"), "{e}");

        let mut s = sample();
        s.modules.clear();
        let e = parse(&s.render()).unwrap_err().to_string();
        assert!(e.contains("一个模块都没有"), "{e}");
    }
}