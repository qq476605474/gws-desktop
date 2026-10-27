//! 需求工作区元数据 `.workspace.json`（schema 2）。
//!
//! 放在 `ws/<完整分支名>/.workspace.json`，是这个工作区的唯一真源：
//! 涉及哪些模块、基线是什么、合并到各环境的日期、文档归属、自定义信息都在这里。
//!
//! ## 与旧版（schema 1）的差别
//!
//! - **目录名即分支名**，所以不再需要单独的 `name` 字段（旧版 `name` 是"短名"，
//!   与 `branch` 并存且要人工保持一致，是撞车与错位的来源）
//! - `base` 从字符串改成**数组**（基线链，顺序即优先级）
//! - 新增 `doc`（doc-hub 归属）、`links`（自定义信息）、`notes`、`mr`（MR 登记）

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::name;
use crate::{Error, Result};

/// 元数据文件名（位于工作区目录根部）。
pub const META_FILE: &str = ".workspace.json";

/// 当前元数据结构版本。
pub const SCHEMA: u32 = 2;

/// 工作区根 `CLAUDE.md` 的文件名。
pub const CLAUDE_FILE: &str = "CLAUDE.md";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceMeta {
    pub schema: u32,

    /// **完整分支名，同时也是目录名**（需求 #5：分支名即文件夹名）。
    pub branch: String,

    /// 中文备注，给人看。导入需求时从 doc-hub 的 `meta.json` 读取，保证双方一致。
    pub title: String,

    /// 分支前缀（`feature` / `hotfix` / `bugfix` …）。
    #[serde(default)]
    pub prefix: String,

    /// 创建日期 `YYYY-MM-DD`。
    pub created: String,

    /// **基线来源链**，顺序即优先级。可随时修改（需求 #15），改完后继同步走新值。
    ///
    /// 取值形态：`主干` / `ws:<上游需求分支名>` / `<ref>`。
    /// 全都不命中时兜底主干。
    #[serde(default)]
    pub base: Vec<String>,

    /// 各模块上次解析出的实际基线 ref，仅供展示与排查。
    #[serde(default)]
    pub base_resolved: BTreeMap<String, String>,

    /// 本需求涉及并被创建的模块（仓库名）。
    pub modules: Vec<String>,

    /// doc-hub 归属（需求 #2 / #17）。
    #[serde(default)]
    pub doc: Option<DocRef>,

    /// 自定义信息（需求 #20）：tapd、飞书文档，或任意备注链接。
    #[serde(default)]
    pub links: Vec<Link>,

    /// 自由文本备注。
    #[serde(default)]
    pub notes: String,

    /// 各环境的合并日期：`环境标识 → Option<YYYY-MM-DD>`。
    #[serde(default)]
    pub merged: BTreeMap<String, Option<String>>,

    /// MR 登记：`仓库名 → MR 记录`（需求 #1）。
    #[serde(default)]
    pub mr: BTreeMap<String, MrRecord>,
}

impl WorkspaceMeta {
    /// 组装一份新元数据。
    pub fn new(
        branch: impl Into<String>,
        title: impl Into<String>,
        prefix: impl Into<String>,
        base: Vec<String>,
        modules: Vec<String>,
    ) -> Self {
        let branch = branch.into();
        let title = title.into();
        Self {
            schema: SCHEMA,
            title: if title.trim().is_empty() {
                branch.clone()
            } else {
                title
            },
            branch,
            prefix: prefix.into(),
            created: crate::date::today(),
            base,
            base_resolved: BTreeMap::new(),
            modules,
            doc: None,
            links: Vec::new(),
            notes: String::new(),
            merged: BTreeMap::new(),
            mr: BTreeMap::new(),
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path.to_path_buf(), e))?;
        // 先单独校验结构版本，否则旧格式文件只会报「缺字段」，用户看不懂
        crate::schema::check_version(&text, path, SCHEMA)?;
        let mut meta: WorkspaceMeta =
            serde_json::from_str(&text).map_err(|source| Error::Config {
                path: path.to_path_buf(),
                source,
            })?;
        // **读宽松**：把历史遗留的「主干」这类等同于默认的写法就地清掉。
        // 不清的话界面会把它当成一个真填过的值显示出来（实测反馈 #4：明明没填，
        // 却显示成「主干」，改的时候也不知道该不该删）。
        // 这里**不报错**——一个字段不规范不该让整个需求打不开，
        // 严格校验留给写入路径（`normalise_base_chain`）。
        meta.base = crate::workspace::lifecycle::sanitise_base_chain(&meta.base);
        meta.validate()?;
        Ok(meta)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let mut text = serde_json::to_string_pretty(self)
            .map_err(|e| Error::other(format!("序列化工作区元数据失败: {e}")))?;
        text.push('\n');
        // 原子写：中途失败不会留下半截元数据让工作区变得不可读
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| Error::io(tmp.clone(), e))?;
        std::fs::rename(&tmp, path).map_err(|e| Error::io(path.to_path_buf(), e))?;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != SCHEMA {
            return Err(Error::other(format!(
                "工作区元数据版本不支持: 文件是 {}，本程序支持 {}",
                self.schema, SCHEMA
            )));
        }
        name::validate_component(&self.branch, "分支名")?;
        if self.title.trim().is_empty() {
            return Err(Error::other("需求名称（title）不能为空"));
        }
        // **允许 modules 为空**：模块可以被逐个 `gws drop` 掉，或目录被手工删除，
        // 此时工作区本身仍然存在（元数据、文档归属、自定义信息都还在）。
        // 「创建时至少一个模块」是 `lifecycle::new` 的前置条件，不是元数据的约束。
        for m in &self.modules {
            name::validate_component(m, "模块名")?;
        }
        // 重复模块会导致 merge/push 对同一仓库跑两次
        let mut seen = std::collections::BTreeSet::new();
        for m in &self.modules {
            if !seen.insert(m.as_str()) {
                return Err(Error::other(format!("模块重复: {m}")));
            }
        }
        Ok(())
    }

    pub fn has_module(&self, module: &str) -> bool {
        self.modules.iter().any(|m| m == module)
    }

    /// 记一次合并（`环境标识 → 今天`）。
    pub fn stamp_merged(&mut self, env_id: &str) {
        self.merged
            .insert(env_id.to_string(), Some(crate::date::today()));
    }
}

/// doc-hub 归属（需求 #2 / #17）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocRef {
    /// 需求分类中文名 —— doc-hub 里 `workspaces/<族>/` 的目录名。
    /// 同一需求的 V1/V2/bugfix 共用同一个族目录。
    pub family: String,
    /// 期次目录名 —— 等于本工作区的完整分支名。
    pub period: String,
}

impl DocRef {
    pub fn new(family: impl Into<String>, period: impl Into<String>) -> Self {
        Self {
            family: family.into(),
            period: period.into(),
        }
    }

    /// 相对 doc-hub 仓库根的路径：`workspaces/<族>/<期次>/`。
    pub fn relative_path(&self) -> String {
        format!("workspaces/{}/{}", self.family, self.period)
    }
}

/// 一条自定义信息（需求 #20）。`url` 为空时就是纯文本备注。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub label: String,
    #[serde(default)]
    pub url: String,
}

impl Link {
    pub fn new(label: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            url: url.into(),
        }
    }

    pub fn is_link(&self) -> bool {
        !self.url.trim().is_empty()
    }
}

/// 一条 MR 登记（需求 #1）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MrRecord {
    pub iid: u64,
    pub url: String,
    /// `opened` / `closed` / `merged`（取自 GitLab）。
    #[serde(default)]
    pub state: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample() -> WorkspaceMeta {
        WorkspaceMeta::new(
            "feature-20261010-smart-reach",
            "示例需求",
            "feature",
            vec!["ws:feature-upstream".to_string()],
            vec!["demo-web".to_string(), "demo-api".to_string()],
        )
    }

    #[test]
    fn new_fills_sensible_defaults() {
        let m = sample();
        assert_eq!(m.schema, SCHEMA);
        assert_eq!(m.branch, "feature-20261010-smart-reach");
        assert_eq!(m.title, "示例需求");
        assert_eq!(m.prefix, "feature");
        assert_eq!(m.base, vec!["ws:feature-upstream"]);
        assert_eq!(m.created, crate::date::today());
        assert!(m.doc.is_none());
        assert!(m.links.is_empty());
        assert!(m.merged.is_empty());
        m.validate().unwrap();
    }

    #[test]
    fn empty_title_falls_back_to_branch() {
        let m = WorkspaceMeta::new("feature-x", "  ", "feature", vec![], vec!["a".into()]);
        assert_eq!(m.title, "feature-x");
    }

    #[test]
    fn a_legacy_trunk_keyword_in_base_is_cleaned_on_load() {
        // 实测反馈 #4：曾经清空基线会把字面量「主干」写进文件（现在不会了），
        // 但已经写进去的文件还得能正常打开，而且**不能再显示成一个填过的值**。
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(META_FILE);

        let raw = r#"{
          "schema": 2,
          "branch": "feature-x",
          "title": "t",
          "prefix": "feature",
          "created": "2026-10-12",
          "base": ["主干"],
          "base_resolved": {},
          "modules": [],
          "doc": null,
          "links": [],
          "notes": "",
          "merged": {},
          "mr": {}
        }"#;
        std::fs::write(&path, raw).unwrap();

        let m = WorkspaceMeta::load(&path).expect("旧文件必须还能打开");
        assert!(
            m.base.is_empty(),
            "「主干」等价于默认，读出来应是空的：{:?}",
            m.base
        );
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(META_FILE);

        let mut m = sample();
        m.doc = Some(DocRef::new("示例需求", "feature-20261010-smart-reach"));
        m.links.push(Link::new("tapd", "https://tapd.example/1"));
        m.links.push(Link::new("备注", ""));
        m.notes = "随手记".into();
        m.stamp_merged("dev");
        m.mr.insert(
            "demo-web".into(),
            MrRecord {
                iid: 42,
                url: "https://mr/42".into(),
                state: "opened".into(),
            },
        );
        m.save(&path).unwrap();

        let back = WorkspaceMeta::load(&path).unwrap();
        assert_eq!(m, back);
        assert_eq!(
            back.doc.unwrap().relative_path(),
            "workspaces/示例需求/feature-20261010-smart-reach"
        );
        assert_eq!(back.merged.get("dev"), Some(&Some(crate::date::today())));
        assert_eq!(back.mr["demo-web"].iid, 42);
    }

    #[test]
    fn missing_optional_fields_get_defaults() {
        // 手写的精简元数据不应因为缺字段而加载失败
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(META_FILE);
        std::fs::write(
            &path,
            r#"{"schema":2,"branch":"feature-x","title":"需求","created":"2026-10-10","modules":["a"]}"#,
        )
        .unwrap();

        let m = WorkspaceMeta::load(&path).unwrap();
        assert!(m.base.is_empty());
        assert!(m.links.is_empty());
        assert_eq!(m.prefix, "");
    }

    #[test]
    fn rejects_wrong_schema_version() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(META_FILE);
        // 旧版 schema 1 的元数据必须被明确拒绝，而不是被误当成新格式解析
        std::fs::write(
            &path,
            r#"{"schema":1,"name":"x","branch":"feature-x","modules":["a"]}"#,
        )
        .unwrap();
        let err = WorkspaceMeta::load(&path).unwrap_err();
        assert!(err.to_string().contains("版本不支持"), "{err}");
    }

    #[test]
    fn rejects_duplicate_modules() {
        let mut m = sample();
        m.modules = vec!["a".into(), "a".into()];
        let err = m.validate().unwrap_err().to_string();
        assert!(err.contains("重复"), "{err}");
    }

    #[test]
    fn allows_zero_modules() {
        // 模块被全部 drop 掉后工作区仍然存在，元数据不该因此变成非法
        let mut m = sample();
        m.modules.clear();
        m.validate().unwrap();
    }

    #[test]
    fn rejects_invalid_branch_and_module_names() {
        let mut m = sample();
        m.branch = "a/b".into();
        assert!(m.validate().is_err());

        let mut m = sample();
        m.modules = vec!["../escape".into()];
        assert!(m.validate().is_err());
    }

    #[test]
    fn save_is_atomic_and_leaves_no_tmp() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(META_FILE);
        sample().save(&path).unwrap();
        assert!(path.exists());
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn link_distinguishes_url_from_plain_text() {
        assert!(Link::new("tapd", "https://x").is_link());
        assert!(!Link::new("备注", "").is_link());
        assert!(!Link::new("备注", "   ").is_link());
    }

    #[test]
    fn stamp_merged_overwrites_previous_date() {
        let mut m = sample();
        m.stamp_merged("dev");
        m.merged.insert("dev".into(), Some("2000-01-01".into()));
        m.stamp_merged("dev");
        assert_eq!(m.merged.get("dev"), Some(&Some(crate::date::today())));
    }
}