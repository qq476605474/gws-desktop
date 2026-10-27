//! 环境：**环境标识与分支名解耦**（设计方案 §2.2）。
//!
//! ## 要解决的问题
//!
//! 旧版里 `envs/<环境>/` 的目录名**就是**分支名，两者硬绑定。真实数据里
//! `envs/` 下同时存在 `pre` 和 `pre-202609`——同一个"预发环境"，不同仓库用了
//! 不同的分支名，而且下个月还要换成 `pre-202610`。硬绑定导致换分支名就要重命名
//! 目录、所有引用跟着失效。
//!
//! ## 解耦方式
//!
//! 目录名恒为**稳定的环境标识**（`envs/pre/`），实际 checkout 哪个分支由本模块的
//! 覆盖表解析。换月份只改配置一处，目录名不动。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::name;

/// 通配键：`branches["*"]` 表示所有仓库的默认分支。
pub const WILDCARD: &str = "*";

/// 一个环境的定义。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvDef {
    /// 稳定标识，同时是 `envs/<id>/` 的目录名。**永不随分支名变化。**
    pub id: String,
    /// 中文显示名，如「预发」。仅用于界面展示。
    #[serde(default)]
    pub label: String,
    /// 仓库 → 分支名 的覆盖表。
    ///
    /// 空表 = 所有仓库都用同名分支（即 `id` 本身）。只登记不一致的仓库，
    /// 不必把 22 个仓库全列一遍。
    #[serde(default)]
    pub branches: BTreeMap<String, String>,
}

impl EnvDef {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self { id: id.into(), label: label.into(), branches: BTreeMap::new() }
    }

    /// 某仓库在本环境下实际使用的分支名。
    ///
    /// 优先级：**精确匹配 → `"*"` 通配 → 兜底用 `id` 本身**。
    pub fn branch_for(&self, repo: &str) -> &str {
        self.branches
            .get(repo)
            .or_else(|| self.branches.get(WILDCARD))
            .map(String::as_str)
            .unwrap_or(&self.id)
    }

    /// 校验：id 必须是合法的目录名，分支名必须能通过 git 的 ref 格式检查。
    pub fn validate(&self) -> crate::Result<()> {
        name::validate_component(&self.id, "环境标识")?;
        for (repo, branch) in &self.branches {
            if repo != WILDCARD {
                name::validate_component(repo, "仓库名")?;
            }
            // 分支名可以含 `/`（如 `feature/x`），所以用 git 自己的规则而非目录名规则。
            // 这里只做粗筛，精确校验交给 git（`gws env add` 会调 check-ref-format）。
            if branch.trim().is_empty() || branch.contains(char::is_whitespace) {
                return Err(crate::Error::InvalidName {
                    what: format!("环境 {} 中仓库 {repo} 的分支名", self.id),
                    name: branch.clone(),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_falls_back_to_id_when_no_override() {
        let env = EnvDef::new("dev", "开发");
        assert_eq!(env.branch_for("demo-web"), "dev");
        assert_eq!(env.branch_for("任何仓库"), "dev");
    }

    #[test]
    fn exact_match_beats_wildcard_and_id() {
        let mut env = EnvDef::new("pre", "预发");
        env.branches.insert(WILDCARD.to_string(), "pre-latest".into());
        env.branches.insert("demo-api".into(), "pre-202609".into());

        assert_eq!(env.branch_for("demo-api"), "pre-202609"); // 精确匹配
        assert_eq!(env.branch_for("demo-payment"), "pre-latest"); // 通配
    }

    #[test]
    fn wildcard_beats_id_fallback() {
        let mut env = EnvDef::new("prod", "生产");
        env.branches.insert(WILDCARD.to_string(), "main".into());
        assert_eq!(env.branch_for("任意"), "main");
    }

    #[test]
    fn month_switch_only_touches_config() {
        // 真实场景：pre 环境这个月是 pre-202609，下个月换 pre-202610。
        // 环境标识（目录名）始终是 "pre"，只有覆盖表变。
        let mut env = EnvDef::new("pre", "预发");
        env.branches.insert("demo-api".into(), "pre-202609".into());
        assert_eq!(env.branch_for("demo-api"), "pre-202609");

        env.branches.insert("demo-api".into(), "pre-202610".into());
        assert_eq!(env.branch_for("demo-api"), "pre-202610");
        assert_eq!(env.id, "pre"); // 标识不变
    }

    #[test]
    fn validate_rejects_bad_id_and_branch() {
        assert!(EnvDef::new("pre", "预发").validate().is_ok());
        assert!(EnvDef::new("pre 2026", "预发").validate().is_err()); // 含空格
        assert!(EnvDef::new("pre/2026", "预发").validate().is_err()); // 含斜杠

        let mut bad = EnvDef::new("pre", "预发");
        bad.branches.insert("demo-api".into(), "pre 202609".into());
        assert!(bad.validate().is_err()); // 分支名含空格
    }
}