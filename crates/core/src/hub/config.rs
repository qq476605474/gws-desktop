//! hub 级配置：`<hub>/gws.json`。
//!
//! 替代旧版「环境即目录、无配置文件」的做法。环境标识解耦、分支命名默认值、
//! 快捷入口都需要一个落盘位置，统一收在这里。
//!
//! 部署地址矩阵 `deployments.json` 单独成文件（阶段 8），因为它常被批量编辑，
//! 跟这里的结构性配置混在一起容易误改。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::env::EnvDef;
use crate::{Error, Result};

/// 配置文件名（位于 hub 根目录）。
pub const CONFIG_FILE: &str = "gws.json";

/// 当前配置结构版本。读到别的值直接报错，不做静默兼容。
pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubConfig {
    pub schema: u32,
    pub name: String,
    /// 本 hub 绑定的代码托管（GitLab）。
    ///
    /// **建 hub 时填，之后不可改** —— 仓库来源校验、文档库、MR 都以此为准。
    /// 空 = 还没绑定（老配置或离线创建的 hub），此时用到 GitLab 的操作会
    /// 报一条带下一步命令的错误，而不是拿一个想当然的地址去连。
    #[serde(default)]
    pub gitlab: GitlabBinding,
    /// 在这个 hub 里提交时用什么身份。
    #[serde(default)]
    pub git: GitIdentity,
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub envs: Vec<EnvDef>,
    #[serde(default)]
    pub dochub: DocHubConfig,
    /// 「快捷入口」——用户自己配的发布页快捷入口（可选，可以有多条）。
    ///
    /// 开源版**不预设任何发布系统**：列表为空时界面整块隐藏快捷入口，
    /// 而不是摆一个点过去必然 404 的按钮。
    #[serde(default)]
    pub quick_links: Vec<QuickLink>,
}

/// 一个快捷入口：显示名 + 地址。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickLink {
    /// 按钮上显示的名字，如「打开发布页」。
    pub label: String,
    /// 完整地址（http / https）。
    pub url: String,
}

/// 本 hub 绑定的 GitLab 实例。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitlabBinding {
    /// 实例根地址（`https://gitlab.example.com`）。
    ///
    /// **落盘后不可改**：它同时是「哪些仓库能被纳管」的判据（见
    /// `crate::gitlab::check_remote`）和文档库、自更新之外的 GitLab 操作基址。
    /// 空字符串 = 未绑定。
    #[serde(default)]
    pub base_url: String,
}

impl GitlabBinding {
    pub fn is_bound(&self) -> bool {
        !self.base_url.trim().is_empty()
    }
}

/// 「在这个 hub 里提交时用什么身份」。
///
/// # 为什么要跟 git 全局配置解耦
///
/// 同一台机器上常有多套身份：工作用的 GitLab 一个、个人的 GitHub 一个、也许还有
/// 客户项目的。它们都去读全局 `user.name` / `user.email`，**必然串**——
/// 提交记录里出现的会是另一个身份，而且事后很难发现。
///
/// 所以每个 hub 可以绑一份自己的身份，工具会把它写进 hub 内每个仓库的
/// **local config**（worktree 与主 clone 共享同一份，见 `git::apply_identity`），
/// 于是用户在 `ws/` 里手工 `git commit` 用的也是这个身份。
///
/// 两个字段都留空 = 不干预，仍然走 git 全局配置（这是默认值）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitIdentity {
    #[serde(default)]
    pub author_name: String,
    #[serde(default)]
    pub author_email: String,
}

impl GitIdentity {
    /// 两个字段都填了才算「配了身份」。只填一个是半成品，不能拿去写 config。
    pub fn is_set(&self) -> bool {
        !self.author_name.trim().is_empty() && !self.author_email.trim().is_empty()
    }
}

impl HubConfig {
    /// 新建 hub 的默认配置。
    ///
    /// 默认只建 `dev` 一个环境，其余（`pre` / `dev1` / `dev_tmp`…）按需
    /// `gws env add` 增加——旧版同款行为。
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            schema: SCHEMA,
            name: name.into(),
            gitlab: GitlabBinding::default(),
            git: GitIdentity::default(),
            defaults: Defaults::default(),
            envs: vec![EnvDef::new("dev", "开发")],
            dochub: DocHubConfig::default(),
            quick_links: Vec::new(),
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).map_err(|e| Error::io(path.to_path_buf(), e))?;
        // 先单独校验结构版本，否则旧格式文件只会报「缺字段」，用户看不懂
        crate::schema::check_version(&text, path, SCHEMA)?;
        let cfg: HubConfig = serde_json::from_str(&text)
            .map_err(|source| Error::Config { path: path.to_path_buf(), source })?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        // **地址不可改**：写盘前与磁盘上的既有值比一次。
        //
        // 放在 `save` 里是因为**所有**保存路径都经过它，没有别的口子能改地址。
        // `Hub::bind_gitlab` 那里已经拦了「二次绑定」，这里兜住将来任何新增的、
        // 或者手滑的配置修改路径（比如通用的 update_hub 闭包）。
        if path.is_file() {
            if let Ok(existing) = HubConfig::load(path) {
                if existing.gitlab.is_bound()
                    && existing.gitlab.base_url != self.gitlab.base_url
                {
                    return Err(Error::other(
                        "这个 hub 绑定的 GitLab 地址不可修改。\n  \
                         地址是「哪些仓库能纳管」和文档库的依据，建 hub 时就固定了；\n  \
                         确实要换实例，请新建一个 hub。",
                    ));
                }
                if !existing.dochub.url.is_empty() && existing.dochub.url != self.dochub.url {
                    return Err(Error::other(
                        "文档库地址不可修改（建 hub 时已固定）。\n  \
                         确实要换，请新建一个 hub。",
                    ));
                }
            }
        }
        let mut text = serde_json::to_string_pretty(self)
            .map_err(|e| Error::other(format!("序列化配置失败: {e}")))?;
        text.push('\n');
        // 先写临时文件再 rename：中途失败不会留下半截配置把 hub 弄坏
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| Error::io(tmp.clone(), e))?;
        std::fs::rename(&tmp, path).map_err(|e| Error::io(path.to_path_buf(), e))?;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != SCHEMA {
            return Err(Error::other(format!(
                "配置结构版本不支持: 文件是 {}，本程序支持 {}",
                self.schema, SCHEMA
            )));
        }
        if self.name.trim().is_empty() {
            return Err(Error::other("hub 名称不能为空"));
        }
        // GitLab 地址**允许为空**（未绑定），但给了就必须是像样的 http(s) 地址。
        // 不在这里强制非空：那会让「还没绑定」的 hub 连打开都失败，
        // 补绑命令也就无从跑起。要 GitLab 的操作自己会报可操作的错。
        if self.gitlab.is_bound() {
            let u = self.gitlab.base_url.trim();
            if !u.starts_with("http://") && !u.starts_with("https://") {
                return Err(Error::other(format!(
                    "GitLab 地址必须是 http/https 开头，当前为 '{u}'"
                )));
            }
        }
        self.defaults.validate()?;

        // 提交身份要么两个都填、要么都不填：只填一个会写出半截身份
        // （有邮箱没名字之类），提交记录看着就不对劲，而且很难联想到是配置问题
        let has_name = !self.git.author_name.trim().is_empty();
        let has_email = !self.git.author_email.trim().is_empty();
        if has_name != has_email {
            return Err(Error::other(
                "提交身份要么「名字 + 邮箱」都填，要么都留空（留空则用 git 全局配置）",
            ));
        }

        let mut seen = std::collections::BTreeSet::new();
        for env in &self.envs {
            env.validate()?;
            if !seen.insert(env.id.as_str()) {
                return Err(Error::other(format!("环境标识重复: {}", env.id)));
            }
        }

        for l in &self.quick_links {
            if l.label.trim().is_empty() {
                return Err(Error::other("快捷入口的名称不能为空"));
            }
            if !l.url.starts_with("http://") && !l.url.starts_with("https://") {
                return Err(Error::other(format!(
                    "快捷入口「{}」的地址必须是 http/https 开头，当前为 '{}'",
                    l.label, l.url
                )));
            }
        }
        Ok(())
    }

    pub fn env(&self, id: &str) -> Option<&EnvDef> {
        self.envs.iter().find(|e| e.id == id)
    }

    pub fn env_mut(&mut self, id: &str) -> Option<&mut EnvDef> {
        self.envs.iter_mut().find(|e| e.id == id)
    }
}

/// doc-hub（文档仓库）配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocHubConfig {
    #[serde(default = "default_dochub_url")]
    pub url: String,
    /// 关掉后建工作区不会碰 doc-hub（离线开发、或暂时不想接文档时用）。
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for DocHubConfig {
    fn default() -> Self {
        Self { url: default_dochub_url(), enabled: true }
    }
}

fn default_dochub_url() -> String {
    crate::dochub::DEFAULT_URL.to_string()
}

fn default_true() -> bool {
    true
}

/// 各项默认值。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Defaults {
    /// 新建分支的日期位数：8（默认，符合真实仓库现状）或 6。
    ///
    /// 真实仓库里绝大多数是 `feature-20250926-xxx`（8 位），
    /// 也有仓库用 6 位（`feature-260926-xxx`）。这个值只影响
    /// **本工具生成**的分支名；读取别人的分支名时两种都能识别。
    #[serde(default = "default_date_digits")]
    pub branch_date_digits: u8,

    /// 新建分支的默认前缀。
    #[serde(default = "default_prefix")]
    pub branch_prefix: String,

    /// 哪些 GitLab group 下的仓库算**前端**（提 MR 时按前后端分配评审人）。
    ///
    /// 原来的实现把 group 名写死成一个具体值，那对别人的实例永远不成立 ——
    /// 而判错的后果是**该侧的评审人永远挂不上**（那一侧没被用到就直接跳过）。
    /// 现在改成可配置，默认空 = 只按「模块名以 `-front` 结尾」判断。
    ///
    /// 例：`"frontend_groups": ["web", "h5"]`。
    #[serde(default)]
    pub frontend_groups: Vec<String>,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            branch_date_digits: default_date_digits(),
            branch_prefix: default_prefix(),
            frontend_groups: Vec::new(),
        }
    }
}

impl Defaults {
    pub fn validate(&self) -> Result<()> {
        if !matches!(self.branch_date_digits, 6 | 8) {
            return Err(Error::other(format!(
                "branch_date_digits 只能是 6 或 8，当前为 {}",
                self.branch_date_digits
            )));
        }
        if self.branch_prefix.is_empty()
            || !self.branch_prefix.chars().all(|c| c.is_ascii_alphabetic())
        {
            return Err(Error::other(format!(
                "branch_prefix 只能是英文字母，当前为 '{}'",
                self.branch_prefix
            )));
        }
        Ok(())
    }
}

fn default_date_digits() -> u8 {
    8
}

fn default_prefix() -> String {
    "feature".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn new_config_has_dev_env_and_sane_defaults() {
        let cfg = HubConfig::new("my-hub");
        assert_eq!(cfg.schema, SCHEMA);
        assert_eq!(cfg.envs.len(), 1);
        assert_eq!(cfg.envs[0].id, "dev");
        assert_eq!(cfg.defaults.branch_date_digits, 8);
        assert_eq!(cfg.defaults.branch_prefix, "feature");
        cfg.validate().unwrap();
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("gws.json");

        let mut cfg = HubConfig::new("测试 hub");
        cfg.envs.push(EnvDef::new("pre", "预发"));
        cfg.envs[1].branches.insert("demo-api".into(), "pre-202609".into());
        cfg.save(&path).unwrap();

        let back = HubConfig::load(&path).unwrap();
        assert_eq!(cfg, back);
        assert_eq!(back.env("pre").unwrap().branch_for("demo-api"), "pre-202609");
        assert_eq!(back.env("pre").unwrap().branch_for("别的仓库"), "pre");
    }

    #[test]
    fn missing_optional_fields_get_defaults() {
        // 手写的精简配置不应因为缺字段而加载失败
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("gws.json");
        std::fs::write(&path, r#"{"schema":1,"name":"h"}"#).unwrap();

        let cfg = HubConfig::load(&path).unwrap();
        assert_eq!(cfg.defaults.branch_date_digits, 8);
        assert!(cfg.envs.is_empty());
    }

    #[test]
    fn rejects_unknown_schema() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("gws.json");
        std::fs::write(&path, r#"{"schema":99,"name":"h"}"#).unwrap();
        assert!(HubConfig::load(&path).is_err());
    }

    #[test]
    fn rejects_duplicate_env_ids() {
        let mut cfg = HubConfig::new("h");
        cfg.envs.push(EnvDef::new("dev", "开发2"));
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("重复"), "{err}");
    }

    #[test]
    fn rejects_bad_date_digits_and_prefix() {
        let mut cfg = HubConfig::new("h");
        cfg.defaults.branch_date_digits = 7;
        assert!(cfg.validate().is_err());

        let mut cfg = HubConfig::new("h");
        cfg.defaults.branch_prefix = "feat1".into();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn malformed_json_reports_path() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("gws.json");
        std::fs::write(&path, "{ not json").unwrap();
        let err = HubConfig::load(&path).unwrap_err();
        assert!(err.to_string().contains("gws.json"), "{err}");
    }

    #[test]
    fn save_is_atomic_leaves_no_tmp_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("gws.json");
        HubConfig::new("h").save(&path).unwrap();
        assert!(path.exists());
        assert!(!path.with_extension("json.tmp").exists());
    }
}