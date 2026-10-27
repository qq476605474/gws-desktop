//! GitLab API 客户端。
//!
//! ## 为什么自研而不是用现成的 `gitlab` crate
//!
//! 只用到四组接口（projects / users / branches / merge_requests），自己包一层
//! 薄封装约两百行；换来的是对**自建 GitLab 的版本**的完全可控——第三方 crate
//! 按官方 SaaS 的最新 API 演进，遇到自建实例的旧版本字段缺失就会在反序列化时炸掉，
//! 而那种错误现场极难排查。
//!
//! ## 所有可选字段都必须给默认值
//!
//! 自建实例的不同版本返回的字段集不一致（`default_branch` 可能为 null、
//! `ssh_url_to_repo` 在某些端点不返回）。响应结构体里的字段一律 `#[serde(default)]`，
//! 缺字段不能让整个请求失败。

pub mod token;

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// 整体超时。GitLab 偶发慢查询不至于把命令挂死。
const TIMEOUT: Duration = Duration::from_secs(30);

/// **连接**超时，单独设短一些。
///
/// 连不上时（网络不通、DNS 挂了、防火墙丢包）应该在几秒内失败并报错，
/// 而不是让人对着转圈等满 30 秒——30 秒和「卡死」在使用者眼里没区别。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

// ══ 响应类型 ═══════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Project {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path_with_namespace: String,
    /// 主干分支。空仓库时 GitLab 返回 null。
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub web_url: String,
    #[serde(default)]
    pub ssh_url_to_repo: String,
    #[serde(default)]
    pub http_url_to_repo: String,
}

impl Project {
    /// 用于 git clone 的地址。优先 HTTP（HTTP 免配 SSH key 的场景更多）。
    pub fn clone_url(&self) -> &str {
        if !self.http_url_to_repo.is_empty() {
            &self.http_url_to_repo
        } else {
            &self.ssh_url_to_repo
        }
    }

    /// 所属 group（`demo-api-team/demo-api` → `demo-api-team`）。
    pub fn group(&self) -> &str {
        self.path_with_namespace.split('/').next().unwrap_or("")
    }

    pub fn name_tail(&self) -> &str {
        self.path_with_namespace.rsplit('/').next().unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct User {
    pub id: u64,
    #[serde(default)]
    pub username: String,
    /// 显示名（通常是中文名，用于 @ 提醒）。
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct MergeRequest {
    pub iid: u64,
    #[serde(default)]
    pub web_url: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub source_branch: String,
    #[serde(default)]
    pub target_branch: String,
    #[serde(default)]
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateMrRequest {
    pub source_branch: String,
    pub target_branch: String,
    pub title: String,
    pub description: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reviewer_ids: Vec<u64>,
    pub squash: bool,
    pub remove_source_branch: bool,
}

// ══ 客户端 ═════════════════════════════════════════════════════════════

pub struct Client {
    base: String,
    token: String,
    http: reqwest::blocking::Client,
}

impl Client {
    pub fn new(base: impl Into<String>, token: impl Into<String>) -> Result<Self> {
        let http = reqwest::blocking::Client::builder()
            .timeout(TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent(concat!("gws/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::GitLab(format!("创建 HTTP 客户端失败: {e}")))?;
        Ok(Self { base: normalize_base(&base.into()), token: token.into(), http })
    }

    /// **唯一**的正常入口：地址与 token 都取自当前 hub。
    ///
    /// 地址只认 hub 里绑定的那个（**不受环境变量影响** —— 否则「地址不可改」
    /// 就形同虚设）；token 允许被 `GWS_GITLAB_TOKEN` / `GITLAB_TOKEN` 临时覆盖，
    /// CI 与脚本用得上。
    pub fn for_hub(hub: &crate::hub::Hub) -> Result<Self> {
        Self::new(hub.gitlab_base_url()?, token::resolve(hub.root())?)
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    // ── 接口 ──────────────────────────────────────────────────────────

    /// 当前 token 对应的用户。用于「测试连接」。
    pub fn current_user(&self) -> Result<User> {
        self.get("/user", &[])
    }

    /// 按路径取项目（`group/name`）。
    pub fn project(&self, path: &str) -> Result<Project> {
        self.get(&format!("/projects/{}", encode_path(path)), &[])
    }

    /// 检索**当前用户有权限**的项目（需求 #16：不再手工粘贴地址）。
    pub fn search_projects(&self, query: &str, limit: usize) -> Result<Vec<Project>> {
        let mut q: Vec<(&str, String)> = vec![
            ("membership", "true".into()),
            ("per_page", limit.to_string()),
            ("order_by", "last_activity_at".into()),
            ("sort", "desc".into()),
        ];
        if !query.trim().is_empty() {
            q.push(("search", query.trim().to_string()));
        }
        self.get("/projects", &q)
    }

    /// 按关键词搜用户（评审人候选，需求 #4）。
    pub fn search_users(&self, query: &str, limit: usize) -> Result<Vec<User>> {
        let q: Vec<(&str, String)> = vec![
            ("search", query.trim().to_string()),
            ("per_page", limit.to_string()),
        ];
        self.get("/users", &q)
    }

    /// 按 username 精确解析用户（评审人配置里写的是 username）。
    pub fn user_by_username(&self, username: &str) -> Result<User> {
        let users: Vec<User> = self.get("/users", &[("username", username.to_string())])?;
        users
            .into_iter()
            .find(|u| u.username == username)
            .ok_or_else(|| Error::GitLab(format!("GitLab 上不存在用户: {username}")))
    }

    /// 项目上该源分支是否已有**打开的** MR。用于幂等：重跑不重复建。
    pub fn open_mr(&self, project: &str, source_branch: &str) -> Result<Option<MergeRequest>> {
        let list: Vec<MergeRequest> = self.get(
            &format!("/projects/{}/merge_requests", encode_path(project)),
            &[
                ("source_branch", source_branch.to_string()),
                ("state", "opened".to_string()),
            ],
        )?;
        Ok(list.into_iter().next())
    }

    pub fn create_mr(&self, project: &str, req: &CreateMrRequest) -> Result<MergeRequest> {
        let body = serde_json::to_value(req)
            .map_err(|e| Error::GitLab(format!("序列化 MR 请求失败: {e}")))?;
        self.post(&format!("/projects/{}/merge_requests", encode_path(project)), &body)
    }

    /// 关闭 MR（需求 #1 的「撤销」）。
    ///
    /// GitLab 的 MR **不能删除，只能关闭**——所以「撤销」的语义就是 close，
    /// 分支不动（分支清理是独立功能）。
    pub fn close_mr(&self, project: &str, iid: u64) -> Result<MergeRequest> {
        let body = serde_json::json!({ "state_event": "close" });
        self.put(
            &format!("/projects/{}/merge_requests/{}", encode_path(project), iid),
            &body,
        )
    }

    /// 远程分支是否存在（需求 #11 / #12 都用它做预检）。
    ///
    /// 单独一个方法而不是拉全量列表再查：分支多的大仓库全量列表可能几百条，
    /// 而我们只关心一条。
    pub fn branch_exists(&self, project: &str, branch: &str) -> Result<bool> {
        #[derive(Deserialize)]
        struct Branch {
            name: String,
        }
        let url = format!(
            "/projects/{}/repository/branches/{}",
            encode_path(project),
            encode_path(branch)
        );
        match self.get::<Branch>(&url, &[]) {
            Ok(b) => Ok(b.name == branch),
            // 404 = 不存在。这不是错误，是「没有」这个答案。
            Err(Error::GitLab(msg)) if msg.contains("404") => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// 删除远程分支（需求 #11）。
    pub fn delete_branch(&self, project: &str, branch: &str) -> Result<()> {
        self.delete(&format!(
            "/projects/{}/repository/branches/{}",
            encode_path(project),
            encode_path(branch)
        ))
    }

    /// 项目的远程分支名列表。
    pub fn branches(&self, project: &str) -> Result<Vec<String>> {
        #[derive(Deserialize)]
        struct Branch {
            name: String,
        }
        let list: Vec<Branch> =
            self.get(&format!("/projects/{}/repository/branches", encode_path(project)), &[])?;
        Ok(list.into_iter().map(|b| b.name).collect())
    }

    // ── HTTP 底层 ─────────────────────────────────────────────────────

    /// 不需要 token 的客户端（公开项目读取）。
    pub fn anonymous(base: impl Into<String>) -> Result<Self> {
        Self::new(base, "")
    }

    fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T> {
        let url = format!("{}{}", self.base, path);
        let req = self.get_req(&url).query(query);
        send(req, path)
    }

    /// 构造 GET 请求：**token 为空时不发鉴权头**——
    /// 发一个空的 PRIVATE-TOKEN 会被 GitLab 当成无效凭据而 401。
    fn get_req(&self, url: &str) -> reqwest::blocking::RequestBuilder {
        let req = self.http.get(url);
        if self.token.is_empty() {
            req
        } else {
            req.header("PRIVATE-TOKEN", &self.token)
        }
    }

    fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<T> {
        let url = format!("{}{}", self.base, path);
        let req = self.http.post(&url).header("PRIVATE-TOKEN", &self.token).json(body);
        send(req, path)
    }

    fn delete(&self, path: &str) -> Result<()> {
        let url = format!("{}{}", self.base, path);
        let resp = self
            .http
            .delete(&url)
            .header("PRIVATE-TOKEN", &self.token)
            .send()
            .map_err(|e| Error::GitLab(format!("请求 {path} 失败: {e}")))?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let text = resp.text().unwrap_or_default();
        Err(Error::GitLab(format!(
            "GitLab 返回 {status}（{path}）: {}",
            crate::http::snippet(&text)
        )))
    }

    fn put<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<T> {
        let url = format!("{}{}", self.base, path);
        let req = self.http.put(&url).header("PRIVATE-TOKEN", &self.token).json(body);
        send(req, path)
    }
}

fn send<T: serde::de::DeserializeOwned>(
    req: reqwest::blocking::RequestBuilder,
    path: &str,
) -> Result<T> {
    let resp = req
        .send()
        .map_err(|e| Error::GitLab(format!("请求 {path} 失败: {e}")))?;
    let status = resp.status();
    let text = resp.text().unwrap_or_default();

    if !status.is_success() {
        return Err(Error::GitLab(format!(
            "GitLab 返回 {status}（{path}）: {}",
            crate::http::snippet(&text)
        )));
    }

    serde_json::from_str(&text).map_err(|e| {
        Error::GitLab(format!("解析 GitLab 响应失败（{path}）: {e}\n  原文: {}", crate::http::snippet(&text)))
    })
}

/// 把实例地址规范成 API 根地址。
///
/// **必须带上 `/api/v4`**：`GITLAB_URL` 环境变量里存的是实例根地址
/// （`https://gitlab.example.com`），拼上 `/projects` 会打到 GitLab 的**网页**
/// 路由，返回一个 HTML 页面而不是 JSON。那种失败在客户端看来只是「解析响应
/// 失败」，很容易误判成 token 或网络问题。
///
/// 也容忍用户直接把完整 API 地址填进来（`.../api/v4`）。
fn normalize_base(base: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    if b.ends_with("/api/v4") {
        b.to_string()
    } else {
        format!("{b}/api/v4")
    }
}

/// GitLab API 里的项目路径必须**整段**百分号编码（`group/name` → `group%2Fname`）。
///
/// 不编码的话 `/projects/group/name` 会被路由成别的端点，返回 404 而不是报错，
/// 排查起来很费劲。
fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 8);
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_path_escapes_slashes() {
        assert_eq!(encode_path("demo-api-team/demo-api"), "demo-api-team%2Fdemo-api");
        assert_eq!(encode_path("a/b/c"), "a%2Fb%2Fc");
        assert_eq!(encode_path("plain"), "plain");
        // 中文字符按 UTF-8 逐字节编码
        assert_eq!(encode_path("组/仓库"), "%E7%BB%84%2F%E4%BB%93%E5%BA%93");
    }

    #[test]
    fn project_helpers() {
        let p = Project {
            id: 1,
            name: "demo-api".into(),
            path_with_namespace: "demo-api-team/demo-api".into(),
            default_branch: Some("main".into()),
            web_url: "https://git/x/demo-api".into(),
            ssh_url_to_repo: "git@git:x/demo-api.git".into(),
            http_url_to_repo: "https://git/x/demo-api.git".into(),
        };
        assert_eq!(p.group(), "demo-api-team");
        assert_eq!(p.name_tail(), "demo-api");
        assert_eq!(p.clone_url(), "https://git/x/demo-api.git");
    }

    #[test]
    fn project_clone_url_falls_back_to_ssh() {
        let mut p = Project {
            id: 1,
            name: "n".into(),
            path_with_namespace: "g/n".into(),
            default_branch: None,
            web_url: String::new(),
            ssh_url_to_repo: "git@git:g/n.git".into(),
            http_url_to_repo: String::new(),
        };
        assert_eq!(p.clone_url(), "git@git:g/n.git");
        p.ssh_url_to_repo.clear();
        assert_eq!(p.clone_url(), "", "两者都没有时返回空串而不是 panic");
    }

    #[test]
    fn project_tolerates_missing_and_null_fields() {
        // 自建实例的不同版本返回的字段集不一致，缺字段不能让整个请求失败
        let p: Project = serde_json::from_str(r#"{"id":7,"path_with_namespace":"g/n"}"#).unwrap();
        assert_eq!(p.id, 7);
        assert_eq!(p.default_branch, None);
        assert_eq!(p.name, "");
    }

    #[test]
    fn merge_request_tolerates_missing_fields() {
        let mr: MergeRequest = serde_json::from_str(r#"{"iid":42}"#).unwrap();
        assert_eq!(mr.iid, 42);
        assert_eq!(mr.state, "");
    }

    #[test]
    fn base_url_always_gets_the_api_prefix() {
        assert_eq!(normalize_base("https://gitlab.example.com"), "https://gitlab.example.com/api/v4");
        assert_eq!(normalize_base("https://gitlab.example.com/"), "https://gitlab.example.com/api/v4");
        // 用户已经填了完整 API 地址时不重复拼
        assert_eq!(
            normalize_base("https://gitlab.example.com/api/v4"),
            "https://gitlab.example.com/api/v4"
        );
        assert_eq!(
            normalize_base("https://gitlab.example.com/api/v4/"),
            "https://gitlab.example.com/api/v4"
        );
    }

    #[test]
    fn create_mr_request_omits_empty_reviewers() {
        let req = CreateMrRequest {
            source_branch: "feature-x".into(),
            target_branch: "main".into(),
            title: "t".into(),
            description: "d".into(),
            reviewer_ids: vec![],
            squash: false,
            remove_source_branch: false,
        };
        let v = serde_json::to_value(&req).unwrap();
        assert!(v.get("reviewer_ids").is_none(), "空评审人列表不该出现在请求体里: {v}");
    }
}
/// 从任意 git 远端地址里取主机名。
///
/// 认 `https://host/...`、`http://host/...`、`git@host:...`、`ssh://git@host/...`
/// 四种写法；看不懂返回空串。**只看主机**，不比路径 —— 同一个实例下换 group
/// 是正常的。
///
/// 「仓库来源校验」与「文档库必须同域名」都用它，所以主机名怎么解析只有这一处。
pub fn remote_host(remote: &str) -> &str {
    let r = remote.trim();
    if let Some(rest) = r.strip_prefix("git@") {
        rest.split([':', '/']).next().unwrap_or("")
    } else if let Some(rest) = r.strip_prefix("ssh://") {
        let rest = rest.split('@').next_back().unwrap_or(rest);
        rest.split([':', '/']).next().unwrap_or("")
    } else {
        r.split_once("://")
            .map(|(_, rest)| rest.split(['/', ':']).next().unwrap_or(""))
            .unwrap_or("")
    }
}

/// 校验一个 git 远端地址是不是**给定主机**上的仓库。
///
/// 认 `https://host/...`、`http://host/...`、`git@host:...`、`ssh://git@host/...`
/// 四种写法。**只看主机**，不比路径 —— 同一个 GitLab 下换 group 是正常的。
///
/// `allowed_host` 由调用方传入（通常是当前 hub 绑定的 GitLab 主机，
/// 见 `Hub::gitlab_host`）。加这条校验是为了防止「混进来别的内容」——
/// 比如手滑粘了一个 GitHub 地址，或者从别处拷来的配置带着别家的 remote。
/// 混进来之后工具所有假设都不成立（分支约定、MR、文档库都只对那个实例有效）。
pub fn check_remote(remote: &str, allowed_host: &str) -> Result<()> {
    let r = remote.trim();

    // **本地路径放行**（绝对路径 / `file://` / 相对路径）。
    //
    // 要拦的是「从**别的远端**拉代码进来」，本地目录不是那一类：离线镜像、
    // 自建的临时仓库、以及测试里造的裸仓库都走这种形式。拦掉它们只会逼人
    // 绕过校验，而不会真的挡住什么。
    if r.starts_with("file://") || r.starts_with('/') || r.starts_with("./") || r.starts_with("../")
    {
        return Ok(());
    }
    if cfg!(windows) && r.chars().nth(1) == Some(':') {
        return Ok(()); // C:\... 这种
    }

    let host = remote_host(r);

    if host.is_empty() {
        return Err(Error::other(format!("看不懂的 git 地址: {remote}")));
    }
    if host != allowed_host {
        return Err(Error::other(format!(
            "只允许 {} 上的仓库（这个 hub 绑定的是这个 GitLab，不接受其它来源）\n  你给的是: {remote}",
            allowed_host
        )));
    }
    Ok(())
}

#[cfg(test)]
mod remote_host_tests {
    use super::{check_remote, remote_host};

    const HOST: &str = "gitlab.example.com";

    #[test]
    fn remote_host_handles_all_four_forms() {
        // 「仓库来源校验」与「文档库同域名校验」都靠它，四种写法都要认
        assert_eq!(remote_host("https://gitlab.example.com/team/repo.git"), "gitlab.example.com");
        assert_eq!(remote_host("http://gitlab.example.com"), "gitlab.example.com");
        assert_eq!(remote_host("git@gitlab.example.com:team/repo.git"), "gitlab.example.com");
        assert_eq!(
            remote_host("ssh://git@gitlab.example.com/team/repo.git"),
            "gitlab.example.com"
        );
        assert_eq!(remote_host("  https://a.b/c  "), "a.b");
        // 空串 / 看不懂的给空串，调用方据此报「看不懂的地址」
        assert_eq!(remote_host(""), "");
        assert_eq!(remote_host("不是地址"), "");
    }

    #[test]
    fn remotes_on_the_hub_gitlab_are_accepted() {
        for ok in [
            "https://gitlab.example.com/team/repo.git",
            "http://gitlab.example.com/group/repo",
            "git@gitlab.example.com:group/repo.git",
            "ssh://git@gitlab.example.com/group/repo.git",
        ] {
            assert!(check_remote(ok, HOST).is_ok(), "应接受：{ok}");
        }
    }

    #[test]
    fn local_paths_are_always_accepted() {
        // 离线镜像 / 自建临时仓库 / 测试里的裸仓库都走这种形式，
        // 拦掉只会逼人绕过校验，而不会真的挡住什么
        for ok in ["/tmp/repo.git", "file:///tmp/repo", "./rel", "../rel"] {
            assert!(check_remote(ok, HOST).is_ok(), "应接受本地路径：{ok}");
        }
    }

    #[test]
    fn the_allowed_host_is_configurable_not_baked_in() {
        // 「不写死某个 GitLab」的核心回归：同一个地址，主机换了结论就跟着变
        let url = "https://gitlab.example.com/team/repo.git";
        assert!(check_remote(url, "gitlab.example.com").is_ok());
        assert!(check_remote(url, "another.example.com").is_err());

        // 反过来同样成立：换一个 hub 绑定的主机，能接受的仓库也跟着换
        let other = "https://another.example.com/team/repo.git";
        assert!(check_remote(other, "another.example.com").is_ok());
        assert!(check_remote(other, "gitlab.example.com").is_err());
    }

    #[test]
    fn other_hosts_are_rejected_with_a_clear_reason() {
        for bad in [
            "https://github.com/foo/bar.git",
            "git@gitlab.com:foo/bar.git",
        ] {
            let err = check_remote(bad, HOST).unwrap_err().to_string();
            // 报错要**说清只允许哪个** —— 否则用户只看到「被拒绝」，不知道该改成什么
            assert!(err.contains(HOST), "报错要说清只允许哪个：{err}");
        }
        // 压根解析不出主机名的，给的是「看不懂」而不是「不允许」，两者要分开
        let err = check_remote("不是地址", HOST).unwrap_err().to_string();
        assert!(err.contains("看不懂"), "{err}");
    }
}
