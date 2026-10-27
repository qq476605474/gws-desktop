//! 发布地址矩阵 `deployments.json`（设计方案 §2.3，需求 #7）。
//!
//! # 解决什么
//!
//! 一个仓库在 dev / pre / prod 各有自己的访问地址和域名。这些信息散在
//! 各人的笔记里，要用的时候得翻半天，还得手工复制域名。
//!
//! 这里集中配置，桌面端做成**可点击的链接 + 一键复制域名**：
//! - 点地址 → 浏览器打开该环境的站点
//! - 点域名 → 复制到剪贴板（贴给测试/前端用）
//!
//! # 为什么不放在 `gws.json` 里
//!
//! `gws.json` 是结构性配置（环境定义、默认值），改错会坏掉整个 hub；
//! 这张矩阵是**经常批量编辑的数据**，混在一起容易误改。
//! 分开还有一个好处：`deployments.json` 可以直接发给别人复制粘贴。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::hub::Hub;
use crate::{Error, Result};

/// 文件名（位于 hub 根目录）。
pub const FILE: &str = "deployments.json";
/// 结构版本。
pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deployments {
    pub schema: u32,
    #[serde(default)]
    pub repos: BTreeMap<String, RepoDeploy>,
}

impl Default for Deployments {
    fn default() -> Self {
        Self { schema: SCHEMA, repos: BTreeMap::new() }
    }
}

/// 一个仓库的发布信息。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoDeploy {
    /// GitLab 仓库页。留空时由 `repos/<名字>` 的 remote 推导。
    #[serde(default)]
    pub gitlab: String,
    /// 环境标识 → 地址。键是**环境标识**（与 `gws.json` 的 envs 对应），
    /// 不是分支名——分支名会随月份变，标识不会（§2.2）。
    #[serde(default)]
    pub envs: BTreeMap<String, EnvAddress>,
    /// 自定义备注（实测反馈 #5）：仓库的项目域名、说明、注意事项等。
    /// 富文本放纯文本字段里 —— 界面按纯文本渲染，不引入 Markdown 引擎，
    /// 免得为了几个链接把渲染链拉长。
    #[serde(default)]
    pub notes: String,
    /// 自定义链接：tapd / 飞书 / 监控面板 / 域名 …… 名称 + 地址。
    ///
    /// 与 `envs` 的区别：`envs` 是**结构化的发布地址矩阵**（有固定语义、按环境
    /// 成行成列）；`links` 是**没有固定结构的信息**，想放什么放什么。
    #[serde(default)]
    pub links: Vec<crate::workspace::Link>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvAddress {
    /// 站点地址（点开就是浏览器）。
    #[serde(default)]
    pub url: String,
    /// 域名（复制用。有时只配域名不配完整 url）。
    #[serde(default)]
    pub domain: String,
}

impl EnvAddress {
    pub fn is_empty(&self) -> bool {
        self.url.trim().is_empty() && self.domain.trim().is_empty()
    }
}

impl Deployments {
    pub fn path(hub: &Hub) -> PathBuf {
        hub.root().join(FILE)
    }

    /// 读取。文件不存在时返回空矩阵——**不是错误**：
    /// 没配过发布地址是完全正常的状态，不该让界面报错。
    pub fn load(hub: &Hub) -> Result<Self> {
        let path = Self::path(hub);
        if !path.is_file() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path).map_err(|e| Error::io(path.clone(), e))?;
        crate::schema::check_version(&text, &path, SCHEMA)?;
        serde_json::from_str(&text)
            .map_err(|source| Error::Config { path: path.clone(), source })
    }

    pub fn save(&self, hub: &Hub) -> Result<()> {
        let path = Self::path(hub);
        let mut text = serde_json::to_string_pretty(self)
            .map_err(|e| Error::other(format!("序列化发布地址失败: {e}")))?;
        text.push('\n');
        // 原子写：中途失败不会留下半截文件
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| Error::io(tmp.clone(), e))?;
        std::fs::rename(&tmp, &path).map_err(|e| Error::io(path, e))?;
        Ok(())
    }

    pub fn repo(&self, name: &str) -> Option<&RepoDeploy> {
        self.repos.get(name)
    }

    /// 取某仓库在某环境的地址。没配就返回 `None`。
    pub fn address(&self, repo: &str, env: &str) -> Option<&EnvAddress> {
        self.repos.get(repo)?.envs.get(env).filter(|a| !a.is_empty())
    }

    /// 设置某仓库在某环境的地址。空值会**删掉该条**而不是留一条空记录——
    /// 空记录在界面上会显示成一行空白，看着像数据坏了。
    pub fn set_address(&mut self, repo: &str, env: &str, addr: EnvAddress) {
        let entry = self.repos.entry(repo.to_string()).or_default();
        if addr.is_empty() {
            entry.envs.remove(env);
        } else {
            entry.envs.insert(env.to_string(), addr);
        }
        self.prune_if_empty(repo);
    }

    /// 设置自定义备注（空串 = 清掉）。
    pub fn set_notes(&mut self, repo: &str, notes: String) {
        let entry = self.repos.entry(repo.to_string()).or_default();
        entry.notes = notes;
        self.prune_if_empty(repo);
    }

    /// 设置自定义链接（空列表 = 清掉）。
    pub fn set_links(&mut self, repo: &str, links: Vec<crate::workspace::Link>) {
        let entry = self.repos.entry(repo.to_string()).or_default();
        // 名称为空又没有地址的条目是手误留下的，丢掉
        entry.links = links
            .into_iter()
            .filter(|l| !l.label.trim().is_empty() || l.is_link())
            .collect();
        self.prune_if_empty(repo);
    }

    /// 条目全空就不必留在文件里。
    fn prune_if_empty(&mut self, repo: &str) {
        let empty = self
            .repos
            .get(repo)
            .map(|e| {
                e.envs.is_empty()
                    && e.gitlab.trim().is_empty()
                    && e.notes.trim().is_empty()
                    && e.links.is_empty()
            })
            .unwrap_or(false);
        if empty {
            self.repos.remove(repo);
        }
    }

    pub fn set_gitlab(&mut self, repo: &str, url: String) {
        let entry = self.repos.entry(repo.to_string()).or_default();
        entry.gitlab = url;
        self.prune_if_empty(repo);
    }

    /// 环境标识改名时，把各仓库地址表里对应的键也迁过去。
    ///
    /// 地址是按**环境标识**存的（分支名会随月份变、标识不会，§2.2）。
    /// 改名不迁键 = 那些地址全丢，而且界面上看不出丢在哪。
    pub fn rename_env(&mut self, old_id: &str, new_id: &str) {
        for entry in self.repos.values_mut() {
            if let Some(v) = entry.envs.remove(old_id) {
                entry.envs.insert(new_id.to_string(), v);
            }
        }
    }

    /// 该仓库的 GitLab 页：优先用配置里的，否则从 remote URL 推导。
    pub fn gitlab_url(&self, hub: &Hub, repo: &str) -> Option<String> {
        if let Some(u) = self.repos.get(repo).map(|r| r.gitlab.trim()).filter(|u| !u.is_empty()) {
            return Some(u.to_string());
        }
        let r = crate::repo::Repo::open(hub, repo).ok()?;
        gitlab_from_remote(&r.remote_url()?)
    }
}

/// 从 git remote URL 推 GitLab 页面地址。
///
/// `https://host/group/name.git` 与 `git@host:group/name.git` 都要认。
pub fn gitlab_from_remote(remote: &str) -> Option<String> {
    let trimmed = remote.trim().trim_end_matches('/');
    let (host, path) = if let Some(rest) = trimmed.strip_prefix("git@") {
        let (h, p) = rest.split_once(':')?;
        (h.to_string(), p.to_string())
    } else {
        let rest = trimmed.split_once("://").map(|(_, r)| r)?;
        let (h, p) = rest.split_once('/')?;
        (h.to_string(), p.to_string())
    };
    let path = path.trim_end_matches(".git").trim_matches('/');
    if path.is_empty() {
        return None;
    }
    Some(format!("https://{host}/{path}"))
}

/// 一张「该配什么」的清单：每个仓库 × 每个环境。
///
/// 界面拿它画表格，人只填空格子。**不用自己去想有哪些仓库哪些环境**。
pub fn matrix(hub: &Hub) -> Result<Vec<MatrixRow>> {
    let d = Deployments::load(hub)?;
    let repos = crate::repo::list(hub)?;
    let envs: Vec<String> = hub.config().envs.iter().map(|e| e.id.clone()).collect();

    Ok(repos
        .into_iter()
        .map(|r| MatrixRow {
            gitlab: d.gitlab_url(hub, &r.name).unwrap_or_default(),
            cells: envs
                .iter()
                .map(|e| {
                    let a = d.address(&r.name, e).cloned().unwrap_or_default();
                    (e.clone(), a)
                })
                .collect(),
            notes: d.repo(&r.name).map(|e| e.notes.clone()).unwrap_or_default(),
            links: d.repo(&r.name).map(|e| e.links.clone()).unwrap_or_default(),
            name: r.name,
        })
        .collect())
}

// ══ 仓库配置的复制 / 粘贴（实测反馈：仓库也一键复制给别人）══════════════
//
// 为什么放在这个文件里：**一个仓库能分享的配置，除了 clone 地址，就是这张矩阵**
// （GitLab 页、各环境地址、自定义备注与链接）。它们本来就在一起。
//
// 场景跟「需求配置复制给别人」一样：我配好了 jenkins/域名/常用链接，
// 发一段给别人，他一键导入就有了 —— 而不是听我念地址。

/// 一段可转发的仓库配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedRepo {
    pub schema: u32,
    /// 仓库名（= `repos/<名字>` 的目录名）。
    pub name: String,
    /// clone 地址。对方没有这个仓库时靠它克隆；取不到时留空串，
    /// 导入会**明确报出这个仓库补不了**，而不是悄悄少一个。
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub gitlab: String,
    /// 环境标识 → 地址。
    #[serde(default)]
    pub envs: BTreeMap<String, EnvAddress>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub links: Vec<crate::workspace::Link>,
}

/// 仓库配置的载荷版本。
pub const SHARE_SCHEMA: u32 = 1;

impl SharedRepo {
    /// 渲染成可以贴在聊天窗口里的文本（标记行 + JSON，跟需求配置同一套形状）。
    pub fn render(&self) -> String {
        let json = serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string());
        format!("{BEGIN}\n{json}\n{END}")
    }

    /// 这份配置里有没有「地址之外」的东西（备注/链接/环境地址）。
    /// 界面据此说清「导入之后除了加仓库，还会补上什么」。
    pub fn info_count(&self) -> usize {
        self.envs.values().filter(|a| !a.is_empty()).count()
            + self.links.len()
            + usize::from(!self.notes.trim().is_empty())
    }
}

const BEGIN: &str = "----- gws 仓库配置（复制整段发给别人）-----";
const END: &str = "----- gws 仓库配置 结束 -----";

/// 导出某仓库的配置。
pub fn share_repo(hub: &Hub, name: &str) -> Result<SharedRepo> {
    let repo = crate::repo::Repo::open(hub, name)?;
    let d = Deployments::load(hub)?;
    let entry = d.repo(name).cloned().unwrap_or_default();
    Ok(SharedRepo {
        schema: SHARE_SCHEMA,
        name: name.to_string(),
        // 读不到 origin 就留空：导入时会明确报「这个仓库补不了」
        url: repo.remote_url().unwrap_or_default(),
        gitlab: entry.gitlab,
        envs: entry.envs,
        notes: entry.notes,
        links: entry.links,
    })
}

/// 解析别人发来的仓库配置。容错方式与需求配置一致（标记行被吃掉也能认）。
pub fn parse_repo(text: &str) -> Result<SharedRepo> {
    let start = text.find('{').ok_or_else(|| {
        Error::other("这段内容里没有找到仓库配置（应以 `{` 开头的一段 JSON）。\n\n确认复制的是完整的一段：从「----- gws 仓库配置」到「结束」都要")
    })?;
    let end = text
        .rfind('}')
        .ok_or_else(|| Error::other("这段内容不完整：JSON 没有闭合的 `}`"))?;
    if end < start {
        return Err(Error::other("这段内容不完整：`}` 出现在 `{` 之前"));
    }
    let shared: SharedRepo = serde_json::from_str(&text[start..=end])
        .map_err(|e| Error::other(format!("这段配置读不出来（可能复制少了或改了内容）: {e}")))?;
    validate_repo(&shared)?;
    Ok(shared)
}

/// 一份仓库配置能不能用来导入。两条路（解析出来的、界面递进来的）都过它。
pub fn validate_repo(s: &SharedRepo) -> Result<()> {
    if s.schema != SHARE_SCHEMA {
        return Err(Error::other(format!(
            "这段配置的格式版本是 {}，本程序支持 {}。让对方用当前版本的程序重新复制一次",
            s.schema, SHARE_SCHEMA
        )));
    }
    crate::name::validate_component(&s.name, "仓库名")?;
    Ok(())
}

/// 导入结果。
#[derive(Debug, Clone, Serialize)]
pub struct RepoImportOutcome {
    pub name: String,
    /// 这次是不是新克隆的（false = 本地本来就有）。
    pub added: bool,
    /// 补上的自定义信息项数（备注 / 链接 / 环境地址）。
    pub info_applied: usize,
    /// 本地已有、**没有覆盖**的项。绝不冲掉人家自己配过的东西 ——
    /// 这是「一键导入别人的配置」，不是「一键把本地的覆盖掉」。
    pub kept_local: Vec<String>,
    /// 本地没有的环境（地址跳过了）。
    pub skipped_envs: Vec<String>,
}

/// 把别人发来的仓库配置在这台机器上落地。
#[allow(clippy::too_many_lines)]
pub fn import_repo(
    hub: &Hub,
    shared: &SharedRepo,
    cb: crate::progress::Progress<'_>,
) -> Result<RepoImportOutcome> {
    let existing = hub.repos_dir().join(&shared.name).join(".git").exists();
    let mut added = false;
    if !existing {
        if shared.url.trim().is_empty() {
            return Err(Error::other(format!(
                "本地没有仓库 {}，而这份配置里又没带它的地址，补不了。\n\n                 让对方在仓库页**重新复制一次**（复制时会带上地址）",
                shared.name
            )));
        }
        let results = crate::repo::add(hub, &[shared.url.trim().to_string()], cb)?;
        for r in &results {
            if let crate::repo::Outcome::Failed(why) = &r.outcome {
                let first = why.lines().next().unwrap_or("").trim();
                return Err(Error::other(format!(
                    "克隆 {} 失败，什么都没改：{first}",
                    shared.name
                )));
            }
        }
        added = true;
    }

    let mut d = Deployments::load(hub)?;
    let mut applied = 0usize;
    let mut kept = Vec::new();
    let mut skipped_envs = Vec::new();

    let local = d.repo(&shared.name).cloned().unwrap_or_default();

    // 已有内容的一律**不覆盖**：这是导入别人的配置，不该把你自己调过的冲掉。
    // 要改的话在「仓库」页自己改 —— 那边本来就能编辑。
    if !shared.gitlab.trim().is_empty() {
        if local.gitlab.trim().is_empty() {
            d.set_gitlab(&shared.name, shared.gitlab.clone());
            applied += 1;
        } else {
            kept.push("GitLab 地址".to_string());
        }
    }
    if !shared.notes.trim().is_empty() {
        if local.notes.trim().is_empty() {
            d.set_notes(&shared.name, shared.notes.clone());
            applied += 1;
        } else {
            kept.push("备注".to_string());
        }
    }
    if !shared.links.is_empty() {
        if local.links.is_empty() {
            d.set_links(&shared.name, shared.links.clone());
            applied += shared.links.len();
        } else {
            kept.push(format!("{} 个自定义链接", local.links.len()));
        }
    }

    // 环境地址：**只认本地已有的环境**。对方 hub 多出来的环境标识在这边
    // 没有对应物，硬塞进去会在配置里留一条永远用不上的记录。
    let known_envs: Vec<String> = hub.config().envs.iter().map(|e| e.id.clone()).collect();
    for (env, addr) in &shared.envs {
        if addr.is_empty() {
            continue;
        }
        if !known_envs.contains(env) {
            skipped_envs.push(env.clone());
            continue;
        }
        if local.envs.get(env).is_some_and(|a| !a.is_empty()) {
            kept.push(format!("{env} 的地址"));
            continue;
        }
        d.set_address(&shared.name, env, addr.clone());
        applied += 1;
    }
    d.save(hub)?;

    skipped_envs.sort();
    Ok(RepoImportOutcome {
        name: shared.name.clone(),
        added,
        info_applied: applied,
        kept_local: kept,
        skipped_envs,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MatrixRow {
    pub name: String,
    pub gitlab: String,
    /// `环境标识 → 地址`，顺序与 hub 配置一致。
    pub cells: Vec<(String, EnvAddress)>,
    /// 自定义备注（实测反馈 #5）。
    #[serde(default)]
    pub notes: String,
    /// 自定义链接（tapd / 飞书 / 域名 / 监控 ……）。
    #[serde(default)]
    pub links: Vec<crate::workspace::Link>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::InitOptions;
    use tempfile::TempDir;

    use crate::workspace::Link;

    /// 造一个裸仓库（真 git，不 mock）。
    fn git_init_bare(cwd: &std::path::Path, dest: &std::path::Path, branch: &str) {
        let out = std::process::Command::new("git")
            .current_dir(cwd)
            .args(["init", "--bare", "-b", branch, dest.to_str().unwrap()])
            .output()
            .expect("git 可执行");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }

    /// 造一个临时 hub。
    ///
    /// **直接写 GitLab 绑定，不走 `Hub::init` 的联网校验**：单测不能联网，
    /// 而 `repo::add` 这类操作现在要求 hub 已绑定（地址用来校验仓库来源）。
    /// 注意测试里的裸仓库都是**本地路径**，`check_remote` 对本地路径一律放行，
    /// 所以这个假地址不会被真的去连。
    fn hub() -> (TempDir, Hub) {
        let tmp = TempDir::new().unwrap();
        let mut h = Hub::init(&tmp.path().join("hub"), InitOptions::default()).unwrap();
        h.config_mut().gitlab.base_url = "https://gitlab.example.com".into();
        h.save_config().unwrap();
        (tmp, h)
    }

    #[test]
    fn missing_file_is_not_an_error() {
        // 没配过发布地址是完全正常的状态，不该让界面报错
        let (_t, h) = hub();
        let d = Deployments::load(&h).unwrap();
        assert!(d.repos.is_empty());
    }

    #[test]
    fn round_trips_through_disk() {
        let (_t, h) = hub();
        let mut d = Deployments::load(&h).unwrap();
        d.set_address(
            "demo-web",
            "dev",
            EnvAddress {
                url: "https://ad-dev.demo.com".into(),
                domain: "ad-dev.demo.com".into(),
            },
        );
        d.set_address(
            "demo-web",
            "prod",
            EnvAddress { url: "https://ad.demo.com".into(), domain: "ad.demo.com".into() },
        );
        d.save(&h).unwrap();

        let back = Deployments::load(&h).unwrap();
        assert_eq!(back, d);
        assert_eq!(back.address("demo-web", "dev").unwrap().domain, "ad-dev.demo.com");
    }

    #[test]
    fn setting_an_empty_address_removes_the_entry() {
        // 留一条空记录在界面上会显示成一行空白，看着像数据坏了
        let (_t, h) = hub();
        let mut d = Deployments::load(&h).unwrap();
        d.set_address("r", "dev", EnvAddress { url: "x".into(), domain: "y".into() });
        assert!(d.address("r", "dev").is_some());

        d.set_address("r", "dev", EnvAddress::default());
        assert!(d.address("r", "dev").is_none());
        // 仓库条目也一并清掉（没有任何内容了）
        assert!(d.repo("r").is_none());
    }

    #[test]
    fn gitlab_url_falls_back_to_remote() {
        let remote = "https://gitlab.example.com/demo-team/demo-web.git";
        assert_eq!(
            gitlab_from_remote(remote).unwrap(),
            "https://gitlab.example.com/demo-team/demo-web"
        );
        assert_eq!(
            gitlab_from_remote("git@gitlab.example.com:demo-api-team/demo-api.git").unwrap(),
            "https://gitlab.example.com/demo-api-team/demo-api"
        );
        assert!(gitlab_from_remote("not a url").is_none());
        assert!(gitlab_from_remote("https://host/").is_none());
    }

    #[test]
    fn rejects_unknown_schema() {
        let (_t, h) = hub();
        std::fs::write(Deployments::path(&h), r#"{"schema":99}"#).unwrap();
        let err = Deployments::load(&h).unwrap_err();
        assert!(err.to_string().contains("版本不支持"), "{err}");
    }

    // ── 仓库配置的复制 / 粘贴 ────────────────────────────────────────

    fn bare_repo(root: &std::path::Path, name: &str) -> PathBuf {
        let bare = root.join(format!("{name}.git"));
        git_init_bare(root, &bare, "main");
        bare
    }

    #[test]
    fn shared_repo_round_trips_through_text() {
        let s = SharedRepo {
            schema: SHARE_SCHEMA,
            name: "demo-payment".into(),
            url: "https://gitlab.example.com/demo-internal/demo-payment.git".into(),
            gitlab: "https://gitlab.example.com/demo-internal/demo-payment".into(),
            envs: [("dev".to_string(), EnvAddress { url: "https://d.example".into(), domain: "d.example".into() })]
                .into_iter()
                .collect(),
            notes: "发布要看这里\n第二行".into(),
            links: vec![Link::new("Jenkins", "https://j.example/job/x")],
        };
        let back = parse_repo(&s.render()).unwrap();
        assert_eq!(back, s, "导出再导入必须一模一样");

        // 聊天窗口把标记行吃掉也认
        let json = serde_json::to_string_pretty(&s).unwrap();
        assert_eq!(parse_repo(&json).unwrap(), s);
        assert_eq!(parse_repo(&format!("好的，配置如下：\n{json}\n谢谢")).unwrap(), s);
    }

    #[test]
    fn parse_repo_rejects_garbage_readably() {
        assert!(parse_repo("你好").unwrap_err().to_string().contains("没有找到仓库配置"));
        assert!(parse_repo("{\"name\":\"a\"").unwrap_err().to_string().contains("不完整"));
        let bad = "{\"schema\":99,\"name\":\"a\"}".to_string();
        assert!(parse_repo(&bad).unwrap_err().to_string().contains("格式版本"));
        // 仓库名不合法（会被当成目录名）
        let bad2 = "{\"schema\":1,\"name\":\"a/b\"}".to_string();
        assert!(parse_repo(&bad2).is_err());
    }

    #[test]
    fn import_repo_clones_the_missing_repo_and_applies_the_info() {
        let (_t, h) = hub(); // 这个 hub 的 envs 默认是 dev/pre
        let work = _t.path().to_path_buf();
        let bare = bare_repo(&work, "demo-payment");

        let shared = SharedRepo {
            schema: SHARE_SCHEMA,
            name: "demo-payment".into(),
            url: bare.to_string_lossy().to_string(),
            gitlab: "https://gitlab.example.com/demo-internal/demo-payment".into(),
            envs: [("dev".to_string(), EnvAddress { url: "https://d.example".into(), domain: "d.example".into() })]
                .into_iter()
                .collect(),
            notes: "域名是 xx".into(),
            links: vec![Link::new("Jenkins", "https://j.example/x")],
        };

        let out = import_repo(&h, &shared, None).unwrap();

        assert!(out.added, "本地没有 → 应当克隆");
        assert!(h.repos_dir().join("demo-payment").join(".git").exists());
        assert!(out.info_applied >= 3, "备注 + 链接 + dev 地址：{:?}", out);
        assert!(out.kept_local.is_empty());

        let d = Deployments::load(&h).unwrap();
        assert_eq!(d.address("demo-payment", "dev").unwrap().domain, "d.example");
        assert_eq!(d.repo("demo-payment").unwrap().notes, "域名是 xx");
        assert_eq!(d.repo("demo-payment").unwrap().links[0].label, "Jenkins");
    }

    #[test]
    fn import_repo_does_not_clobber_local_config() {
        // **这是这个功能最容易伤人的地方**：导入别人的配置不该把你调过的冲掉
        let (_t, h) = hub();
        let work = _t.path().to_path_buf();
        let bare = bare_repo(&work, "demo-payment");
        crate::repo::add(&h, &[bare.to_string_lossy().to_string()], None).unwrap();

        // 我自己先配好了
        let mut d = Deployments::default();
        d.set_notes("demo-payment", "我自己写的备注".into());
        d.set_address(
            "demo-payment",
            "dev",
            EnvAddress { url: "https://mine.example".into(), domain: "mine.example".into() },
        );
        d.save(&h).unwrap();

        let shared = SharedRepo {
            schema: SHARE_SCHEMA,
            name: "demo-payment".into(),
            url: bare.to_string_lossy().to_string(),
            gitlab: String::new(),
            envs: [("dev".to_string(), EnvAddress { url: "https://his.example".into(), domain: "his.example".into() })]
                .into_iter()
                .collect(),
            notes: "其他人写的备注".into(),
            links: vec![Link::new("Jenkins", "https://j.example/x")],
        };
        let out = import_repo(&h, &shared, None).unwrap();

        assert!(!out.added, "本地已有，不该再克隆");
        let d = Deployments::load(&h).unwrap();
        assert_eq!(d.repo("demo-payment").unwrap().notes, "我自己写的备注", "本地的不该被覆盖");
        assert_eq!(d.address("demo-payment", "dev").unwrap().domain, "mine.example");
        // 但本地没有的（那个链接）要补上
        assert_eq!(d.repo("demo-payment").unwrap().links[0].label, "Jenkins");
        // 而且**如实报出哪些没覆盖**，不能悄悄跳过
        assert!(out.kept_local.iter().any(|k| k.contains("备注")), "{:?}", out.kept_local);
        assert!(out.kept_local.iter().any(|k| k.contains("dev")), "{:?}", out.kept_local);
    }

    #[test]
    fn import_repo_skips_envs_this_hub_does_not_have() {
        let (_t, h) = hub();
        let work = _t.path().to_path_buf();
        let bare = bare_repo(&work, "demo-payment");
        let shared = SharedRepo {
            schema: SHARE_SCHEMA,
            name: "demo-payment".into(),
            url: bare.to_string_lossy().to_string(),
            gitlab: String::new(),
            envs: [("staging".to_string(), EnvAddress { url: "https://s.example".into(), domain: "s.example".into() })]
                .into_iter()
                .collect(),
            notes: String::new(),
            links: Vec::new(),
        };

        let out = import_repo(&h, &shared, None).unwrap();

        assert_eq!(out.skipped_envs, vec!["staging"], "本地没有的环境要跳过并报出来");
        let d = Deployments::load(&h).unwrap();
        assert!(d.address("demo-payment", "staging").is_none(), "不该塞一条用不上的记录");
    }

    #[test]
    fn import_repo_refuses_when_url_is_missing_and_repo_is_absent() {
        let (_t, h) = hub();
        let shared = SharedRepo {
            schema: SHARE_SCHEMA,
            name: "ghost-repo".into(),
            url: String::new(),
            gitlab: String::new(),
            envs: BTreeMap::new(),
            notes: "没有地址".into(),
            links: Vec::new(),
        };

        let err = import_repo(&h, &shared, None).unwrap_err().to_string();
        assert!(err.contains("ghost-repo"), "要点名：{err}");
        assert!(err.contains("重新复制"), "要给下一步：{err}");
        assert!(!Deployments::path(&h).exists(), "失败时不该写下任何配置");
    }

    #[test]
    fn save_is_atomic() {
        let (_t, h) = hub();
        Deployments::default().save(&h).unwrap();
        assert!(Deployments::path(&h).is_file());
        assert!(!Deployments::path(&h).with_extension("json.tmp").exists());
    }
}