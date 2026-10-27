//! 需求期次的元数据 `meta.json`（设计方案 §4.2）。
//!
//! 放在 `doc-hub/workspaces/<需求分类中文名>/<完整分支名>/meta.json`，
//! 是**这个期次在文档库里的身份证**。
//!
//! # 为什么这个文件是承重的
//!
//! 需求 #5（中文备注一致）、#17（归组与时间排序）、#20（自定义信息）全靠它：
//!
//! - 别人 `gws get` 导入需求时，从它读中文名与自定义链接 → 双方一致
//! - V1 / V2 / bugfix 归到同一个需求分类目录，靠它记录 `label` 与 `date` 排序
//! - 提 MR 时清单末尾的文档链接取自它的 `links`
//!
//! 所以**必须由工具自动维护**（建工作区、改分支名时自动写），不能靠人手改。

use serde::{Deserialize, Serialize};

use crate::workspace::meta::Link;
use crate::{Error, Result};

/// 元数据文件名。
pub const META_FILE: &str = "meta.json";
/// 期次目录里的 AI 约定文件名。
pub const CLAUDE_FILE: &str = "CLAUDE.md";
/// 当前结构版本。
pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeriodMeta {
    pub schema: u32,

    /// 完整分支名（同时也是工作区目录名）。**族内唯一**，用它定位期次。
    pub branch: String,

    /// 需求分类中文名 —— `workspaces/<族>/` 的目录名。
    /// 同一需求的 V1/V2/bugfix 共用同一个族目录。
    pub family: String,

    /// 需求中文名（本期的）。
    pub name: String,

    /// 期次标签，人读：`V1` / `V2` / `BUGFIX`。缺省按创建顺序自动编号。
    #[serde(default)]
    pub label: String,

    /// 日期 `YYYY-MM-DD`，排序用。
    pub date: String,

    #[serde(default)]
    pub owner: String,

    /// 一句话说明本期做了什么。
    #[serde(default)]
    pub summary: String,

    /// 本期涉及的仓库。
    #[serde(default)]
    pub repos: Vec<String>,

    /// 自定义信息（需求 #20）。
    #[serde(default)]
    pub links: Vec<Link>,

    #[serde(default)]
    pub notes: String,
}

impl PeriodMeta {
    pub fn new(
        branch: impl Into<String>,
        family: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            schema: SCHEMA,
            branch: branch.into(),
            family: family.into(),
            name: name.into(),
            label: String::new(),
            date: crate::date::today(),
            owner: current_user(),
            summary: String::new(),
            repos: Vec::new(),
            links: Vec::new(),
            notes: String::new(),
        }
    }

    pub fn load(path: &std::path::Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path.to_path_buf(), e))?;
        crate::schema::check_version(&text, path, SCHEMA)?;
        serde_json::from_str(&text).map_err(|source| Error::Config {
            path: path.to_path_buf(),
            source,
        })
    }

    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        let mut text = serde_json::to_string_pretty(self)
            .map_err(|e| Error::other(format!("序列化期次元数据失败: {e}")))?;
        text.push('\n');
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| Error::io(tmp.clone(), e))?;
        std::fs::rename(&tmp, path).map_err(|e| Error::io(path.to_path_buf(), e))?;
        Ok(())
    }

    /// 期次目录在 doc-hub 仓库里的相对路径：`workspaces/<族>/<分支>/`。
    pub fn relative_dir(&self) -> String {
        format!("workspaces/{}/{}", self.family, self.branch)
    }
}

/// 当前登录的系统用户名。取不到就留空——**不猜**（猜成主机名之类的只会误导）。
fn current_user() -> String {
    ["USER", "USERNAME", "LOGNAME"]
        .iter()
        .find_map(|k| std::env::var(k).ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_default()
}

/// 期次的 AI 约定文件内容（需求 #2：说明所有 AI 产生的文档统一放这里）。
pub fn render_claude_md(meta: &PeriodMeta) -> String {
    format!(
        "# {name}\n\
         \n\
         > 需求分类 `{family}` · 期次 `{label}` · 分支 `{branch}` · 起始 {date}\n\
         \n\
         ## 文档都放这里\n\
         \n\
         **本目录**就是本期全部文档的唯一去处。AI 产生的任何文档都必须写在这里，\n\
         不要散落到别处。\n\
         \n\
         > 不写具体路径是**故意的**：分类名、需求名、分支名都会变，写进来就会过期。\n\
         > 你已经在「本目录」里了，不需要靠一串路径确认。\n\
         \n\
         常用类型（用 `gws doc new <类型>` 生成带模板的文件）：\n\
         \n\
         | 类型 | 文件名 | 内容要点 |\n\
         |---|---|---|\n\
         | 需求文档 | 需求文档.md | 背景、目标、范围、验收标准 |\n\
         | 设计方案 | 设计方案.md | 整体设计与取舍 |\n\
         | 技术方案 | 技术方案.md | 接口、表结构、改动点 |\n\
         | 说明文档 | 说明文档.md | 实现说明、注意事项 |\n\
         | 使用说明 | 使用说明.md | 怎么用，给使用方看 |\n\
         | 代码导读 | 代码导读.md | 关键文件与阅读顺序 |\n\
         | 提测单 | 提测单.md | 提测范围、环境、验证点 |\n\
         | 测试用例 | 测试用例.md | 用例表 |\n\
         | sql | sql/ | 数据变更脚本 |\n\
         | 发布步骤 | 发布步骤.md | 上线操作顺序 |\n\
         | 实施步骤 | 实施步骤.md | 现场实施顺序 |\n\
         \n\
         ## 同一需求的其它期次\n\
         \n\
         在**上一级目录**下，按需阅读。改需求做 V2 时，先读 V1 那一期，避免重复踩坑。\n\
         \n\
         ## 提交约定（与 hub 根那份一致）\n\
         \n\
         - **AI 在这个 hub 里写的任何文件都不要自行 `git commit` / `git push`** ——\n\
           代码、文档、脚本都算，**不只是文档**。只 `git add` 或直接展示变更，\n\
           由人看过之后再提交（除非人明确说「直接提交」）。\n\
         - 原因：AI 自己提交之后，人还没读过就被推上去了；几次之后谁也说不清改了什么。\n\
         - 文档这条流程是：`gws doc status` 看变更 → 人确认 → `gws doc sync`（提交并推送文档分支）。\n\
         - 文档走**分支 + Merge Request**，不直接提交 `main`（没有 main 推送权限）。\n\
         - 本目录所属分支与当前需求分支同名，**不用切分支**。\n",
        name = meta.name,
        family = meta.family,
        label = if meta.label.is_empty() { "（未标注）" } else { &meta.label },
        branch = meta.branch,
        date = meta.date,
    )
}

/// 文档类型（需求 #2 列出的常用类型）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocType {
    /// 类型名，也是命令里的参数。
    pub key: &'static str,
    /// 默认文件名（相对期次目录）。
    pub file: &'static str,
    /// 模板正文里的章节提示。
    pub outline: &'static str,
}

/// 全部内置文档类型。顺序即展示顺序。
pub const DOC_TYPES: &[DocType] = &[
    DocType {
        key: "需求文档",
        file: "需求文档.md",
        outline: "## 背景\n\n## 目标\n\n## 范围\n\n## 不做什么\n\n## 验收标准\n",
    },
    DocType {
        key: "设计方案",
        file: "设计方案.md",
        outline: "## 整体设计\n\n## 取舍与备选方案\n\n## 影响面\n",
    },
    DocType {
        key: "技术方案",
        file: "技术方案.md",
        outline: "## 接口\n\n## 数据表\n\n## 改动点\n\n## 兼容与回滚\n",
    },
    DocType {
        key: "说明文档",
        file: "说明文档.md",
        outline: "## 实现说明\n\n## 注意事项\n\n## 已知限制\n",
    },
    DocType {
        key: "使用说明",
        file: "使用说明.md",
        outline: "## 面向谁\n\n## 怎么用\n\n## 常见问题\n",
    },
    DocType {
        key: "代码导读",
        file: "代码导读.md",
        outline: "## 关键文件\n\n## 阅读顺序\n\n## 数据流\n",
    },
    DocType {
        key: "提测单",
        file: "提测单.md",
        outline: "## 提测范围\n\n## 测试环境\n\n## 依赖与前置\n\n## 验证点\n\n## 自测情况\n",
    },
    DocType {
        key: "测试用例",
        file: "测试用例.md",
        outline: "## 用例\n\n| 编号 | 场景 | 步骤 | 预期 |\n|---|---|---|---|\n",
    },
    DocType {
        key: "sql",
        file: "sql/README.md",
        outline: "## 变更说明\n\n## 执行顺序\n\n## 回滚脚本\n",
    },
    DocType {
        key: "发布步骤",
        file: "发布步骤.md",
        outline: "## 发布前\n\n## 发布顺序\n\n## 发布后验证\n\n## 回滚步骤\n",
    },
    DocType {
        key: "实施步骤",
        file: "实施步骤.md",
        outline: "## 前置条件\n\n## 实施顺序\n\n## 验证\n",
    },
];

pub fn doc_type(key: &str) -> Option<&'static DocType> {
    DOC_TYPES.iter().find(|t| t.key == key)
}

/// 生成一个文档的初始内容。
pub fn template(doc: &DocType, meta: &PeriodMeta) -> String {
    format!(
        "# {title}\n\
         \n\
         > 需求 {name} · 期次 {label} · 分支 `{branch}`\n\
         \n\
         {outline}\n",
        title = doc.key,
        name = meta.name,
        label = if meta.label.is_empty() {
            "-"
        } else {
            &meta.label
        },
        branch = meta.branch,
        outline = doc.outline,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_dir_follows_the_contract() {
        let m = PeriodMeta::new("feature-20261010-x", "示例需求", "示例需求 V2");
        assert_eq!(
            m.relative_dir(),
            "workspaces/示例需求/feature-20261010-x"
        );
    }

    #[test]
    fn new_fills_defaults() {
        let m = PeriodMeta::new("feature-x", "族", "名字");
        assert_eq!(m.schema, SCHEMA);
        assert_eq!(m.date, crate::date::today());
        assert!(m.links.is_empty());
        // owner 取不到就留空，不猜
        assert!(m.owner.is_empty() || !m.owner.contains('/'));
    }

    #[test]
    fn legacy_status_field_is_ignored_and_dropped_on_save() {
        // 旧文件里写着「开发中」；真实状态以是否并入主干为准，字段不再维护。
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join(META_FILE);
        std::fs::write(
            &path,
            r#"{"schema":1,"branch":"feature-x","family":"族","name":"名字",
                "date":"2026-10-20","status":"开发中"}"#,
        )
        .unwrap();

        let m = PeriodMeta::load(&path).expect("旧文件必须还能打开");
        m.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("\"status\""),
            "保存后不该再写 status：{text}"
        );
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join(META_FILE);

        let mut m = PeriodMeta::new("feature-x", "族", "名字");
        m.label = "V2".into();
        m.summary = "增加智能触达".into();
        m.links.push(Link::new("tapd", "https://tapd/1"));
        m.save(&path).unwrap();

        let back = PeriodMeta::load(&path).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn rejects_unknown_schema() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join(META_FILE);
        std::fs::write(
            &path,
            r#"{"schema":99,"branch":"x","family":"f","name":"n","date":"2026-10-10"}"#,
        )
            .unwrap();
        let err = PeriodMeta::load(&path).unwrap_err();
        assert!(err.to_string().contains("版本不支持"), "{err}");
    }

    #[test]
    fn every_doc_type_is_looked_up_by_key() {
        for t in DOC_TYPES {
            let found = doc_type(t.key).expect("每个类型都应能按键查到");
            assert_eq!(found.file, t.file);
            assert!(!t.file.is_empty());
        }
        assert!(doc_type("不存在的类型").is_none());
    }

    #[test]
    fn doc_types_cover_the_requested_list() {
        // 需求 #2 列出的类型一个都不能少
        for key in [
            "需求文档",
            "说明文档",
            "使用说明",
            "代码导读",
            "技术方案",
            "提测单",
            "测试用例",
            "sql",
            "发布步骤",
            "实施步骤",
            "设计方案",
        ] {
            assert!(doc_type(key).is_some(), "缺少文档类型: {key}");
        }
    }

    #[test]
    fn template_carries_requirement_context() {
        let m = PeriodMeta::new("feature-20261010-x", "示例需求", "示例需求 V2");
        let t = template(doc_type("技术方案").unwrap(), &m);
        assert!(t.starts_with("# 技术方案"), "{t}");
        assert!(t.contains("feature-20261010-x"), "{t}");
        assert!(t.contains("示例需求 V2"), "{t}");
    }

    #[test]
    fn claude_md_explains_where_docs_go() {
        let m = PeriodMeta::new("feature-x", "示例需求", "需求名");
        let md = render_claude_md(&m);
        assert!(md.contains("唯一去处"), "{md}");
        assert!(md.contains("gws doc sync"), "{md}");
        // 每一期都要能指向同族的其它期次（做 V2 时要读 V1）
        assert!(md.contains("上一级目录"), "{md}");

        // **不写死路径**：分类名/需求名/分支名都会变（改分类要挪目录、改中文名
        // 要同步、分支名也能改），把路径写进来必然过期。实测反馈。
        assert!(
            !md.contains("workspaces/"),
            "不该把会变的路径写进说明：{md}"
        );
        assert!(
            !md.contains("doc-hub/"),
            "也不该出现旧布局的 doc-hub/ 路径：{md}"
        );
    }
}