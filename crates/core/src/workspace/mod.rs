//! 需求工作区：**一个需求 = 一个工作区 = 一组 worktree**。
//!
//! 每个需求在 `ws/<完整分支名>/` 下有一个目录，里面**只包含该需求会改动的模块**
//! （每个模块是该仓库的一个 git worktree，全部固定在同一分支上）。检索范围即
//! 改动范围，切需求即切目录，永不切分支。
//!
//! ## 目录名为什么是完整分支名
//!
//! 试过用「短名」（从分支名反推的友好名），但它会撞车：同一需求做 V1、V2 时
//! `feature-20260901-smart-reach` 与 `feature-20261010-smart-reach` 反推出的短名
//! 都是 `smart-reach`——而这恰恰是最常见的用法。分支名天然唯一且双方一致，
//! 中文备注放在元数据的 `title` 里（导入时从 doc-hub 读取，保证一致）。

pub mod branch;
pub mod lifecycle;
pub mod meta;
pub mod mr;
pub mod revert;
pub mod share;
pub mod sync;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::hub::Hub;
use crate::repo::Repo;
use crate::{Error, Result};

pub use meta::{DocRef, Link, MrRecord, WorkspaceMeta, CLAUDE_FILE, META_FILE};

/// 一个已打开的需求工作区。
#[derive(Debug, Clone)]
pub struct Workspace {
    dir: PathBuf,
    meta: WorkspaceMeta,
}

impl Workspace {
    /// 打开 `ws/<branch>/`。
    pub fn open(hub: &Hub, branch: &str) -> Result<Self> {
        Self::from_dir(&hub.ws_dir().join(branch))
    }

    /// 从工作区目录打开（校验元数据存在）。
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let dir = crate::hub::absolutize(dir)?;
        let meta_path = dir.join(META_FILE);
        if !meta_path.is_file() {
            return Err(Error::WorkspaceNotFound(
                dir.file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
            ));
        }
        let meta = WorkspaceMeta::load(&meta_path)?;
        Ok(Self { dir, meta })
    }

    /// 从任意目录向上找 `.workspace.json`——等价于旧版的 `gws cur_ws()`。
    pub fn locate(start: &Path) -> Result<Self> {
        let mut dir = crate::hub::absolutize(start)?;
        loop {
            if dir.join(META_FILE).is_file() {
                return Self::from_dir(&dir);
            }
            if !dir.pop() {
                return Err(Error::other(
                    "不在任何需求工作区内（当前目录及上级均无 .workspace.json）。\
                     工作区命令需在 ws/<分支名>/ 内执行",
                ));
            }
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn meta(&self) -> &WorkspaceMeta {
        &self.meta
    }

    pub fn meta_mut(&mut self) -> &mut WorkspaceMeta {
        &mut self.meta
    }

    /// 分支名（同时也是目录名）。
    pub fn branch(&self) -> &str {
        &self.meta.branch
    }

    pub fn title(&self) -> &str {
        &self.meta.title
    }

    pub fn save_meta(&self) -> Result<()> {
        self.meta.save(&self.dir.join(META_FILE))
    }

    /// 某个模块在工作区里的路径。
    pub fn module_path(&self, module: &str) -> PathBuf {
        self.dir.join(module)
    }

    /// **实际存在**的模块目录（可能与元数据登记的不一致——模块目录被手工删过，
    /// 或加模块时中途失败）。所有按模块遍历的操作都应以它为准。
    pub fn present_modules(&self) -> Vec<String> {
        self.meta
            .modules
            .iter()
            .filter(|m| self.module_path(m).join(".git").exists())
            .cloned()
            .collect()
    }

    /// **扫描磁盘**得到工作区里实际存在的模块目录。
    ///
    /// 与 [`Workspace::present_modules`] 的区别：后者以元数据登记名单做过滤，
    /// 本方法只认磁盘。`add_modules` 新增的模块此时还没写进元数据，
    /// 用前者会把刚加进来的模块过滤掉（列表永远回写不上去）。
    pub fn scan_module_dirs(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.join(".git").exists() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            // 挡掉非模块的目录（如 .idea、用户随手建的目录）
            if crate::name::validate_component(&name, "模块名").is_err() {
                continue;
            }
            out.push(name);
        }
        out.sort();
        out
    }

    /// 打开某个模块对应的仓库（用于拿到主干名等）。
    pub fn repo(&self, hub: &Hub, module: &str) -> Result<Repo> {
        Repo::open(hub, module)
    }

    /// 工作区根 `CLAUDE.md` 的路径。
    pub fn claude_path(&self) -> PathBuf {
        self.dir.join(CLAUDE_FILE)
    }
}

/// **模块选择**（设计方案 §6.4）。
///
/// 所有模块级操作（merge / push / pull / sync-main / 撤销改动 / 强制回退 / 提 MR …）
/// 都接受它。默认全选——不做成「自动只选有改动的模块」，因为对 merge/pull 这类
/// 操作，「没改动」也可能是需要处理的（只落后远程要 pull），自动过滤会漏。
/// 界面负责把每个模块的状态标出来让人自己判断。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModuleFilter {
    /// 只处理这些模块。`None` = 不限制。
    pub only: Option<BTreeSet<String>>,
    /// 排除这些模块。
    pub exclude: BTreeSet<String>,
}

impl ModuleFilter {
    pub fn all() -> Self {
        Self::default()
    }

    pub fn only<I, S>(modules: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            only: Some(modules.into_iter().map(Into::into).collect()),
            exclude: BTreeSet::new(),
        }
    }

    pub fn excluding<I, S>(modules: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            only: None,
            exclude: modules.into_iter().map(Into::into).collect(),
        }
    }

    /// 从全部模块中筛出本次要处理的。
    pub fn resolve<'a>(&self, all: &'a [String]) -> Vec<&'a String> {
        all.iter()
            .filter(|m| self.only.as_ref().is_none_or(|o| o.contains(*m)))
            .filter(|m| !self.exclude.contains(*m))
            .collect()
    }

    /// 指定了 `--only` 却不匹配任何模块 → 多半是名字敲错了，应当报错而不是静默什么都不做。
    pub fn unmatched_only(&self, all: &[String], dochub_name: &str) -> Vec<String> {
        match &self.only {
            None => Vec::new(),
            Some(only) => only
                .iter()
                // **文档库（doc-hub）不算「未匹配的模块」**。
                // 它不是代码模块（有自己的挂载流程），偶尔被谁顺带列进来
                // 不该报「指定的模块不在本工作区」把整个操作拦下 ——
                // 实测反馈里 MR 预览就是这么被拦的。当成"不参与"、跳过即可。
                .filter(|m| !all.contains(m) && m.as_str() != dochub_name)
                .cloned()
                .collect(),
        }
    }
}

/// 列出 hub 下所有需求工作区（按分支名排序）。
pub fn list(hub: &Hub) -> Result<Vec<Workspace>> {
    let dir = hub.ws_dir();
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&dir).map_err(|e| Error::io(dir.clone(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(dir.clone(), e))?;
        let path = entry.path();
        // 只看带元数据的目录——被手工删了一半的残留目录不算工作区
        if !path.join(META_FILE).is_file() {
            continue;
        }
        match Workspace::from_dir(&path) {
            Ok(ws) => out.push(ws),
            // 单个工作区元数据损坏不应让整个列表失败；由 `gws doctor` 单独报
            Err(_) => continue,
        }
    }
    out.sort_by(|a, b| a.branch().cmp(b.branch()));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::InitOptions;
    use tempfile::TempDir;

    /// 造一个「像工作区」的目录（不建 worktree，只放元数据）。
    fn fake_workspace(hub: &Hub, branch: &str, modules: &[&str]) -> Workspace {
        let dir = hub.ws_dir().join(branch);
        std::fs::create_dir_all(&dir).unwrap();
        let meta = WorkspaceMeta::new(
            branch,
            "测试需求",
            "feature",
            vec!["主干".into()],
            modules.iter().map(|s| s.to_string()).collect(),
        );
        meta.save(&dir.join(META_FILE)).unwrap();

        // 元数据存在 → 可打开
        Workspace::from_dir(&dir).unwrap()
    }

    #[test]
    fn open_and_read_metadata() {
        let tmp = TempDir::new().unwrap();
        let hub = Hub::init(&tmp.path().join("hub"), InitOptions::default()).unwrap();
        let ws = fake_workspace(&hub, "feature-20261010-demo", &["a"]);

        assert_eq!(ws.branch(), "feature-20261010-demo");
        assert_eq!(ws.title(), "测试需求");
        assert_eq!(ws.dir(), hub.ws_dir().join("feature-20261010-demo"));
    }

    #[test]
    fn open_missing_workspace_reports_not_found() {
        let tmp = TempDir::new().unwrap();
        let hub = Hub::init(&tmp.path().join("hub"), InitOptions::default()).unwrap();
        let err = Workspace::open(&hub, "nope").unwrap_err();
        assert!(matches!(err, Error::WorkspaceNotFound(_)), "{err}");
    }

    #[test]
    fn locate_walks_up_from_module_subdir() {
        let tmp = TempDir::new().unwrap();
        let hub = Hub::init(&tmp.path().join("hub"), InitOptions::default()).unwrap();
        fake_workspace(&hub, "feature-x", &["a"]);

        let deep = hub.ws_dir().join("feature-x").join("a").join("src/main");
        std::fs::create_dir_all(&deep).unwrap();

        let ws = Workspace::locate(&deep).unwrap();
        assert_eq!(ws.branch(), "feature-x");
    }

    #[test]
    fn locate_fails_outside_any_workspace() {
        let tmp = TempDir::new().unwrap();
        let err = Workspace::locate(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("不在任何需求工作区"), "{err}");
    }

    #[test]
    fn list_returns_sorted_and_skips_leftovers() {
        let tmp = TempDir::new().unwrap();
        let hub = Hub::init(&tmp.path().join("hub"), InitOptions::default()).unwrap();
        fake_workspace(&hub, "feature-b", &["a"]);
        fake_workspace(&hub, "feature-a", &["a"]);
        // 残留目录：没有元数据，不该出现在列表里
        std::fs::create_dir_all(hub.ws_dir().join("leftover")).unwrap();

        let branches: Vec<_> = list(&hub)
            .unwrap()
            .iter()
            .map(|w| w.branch().to_string())
            .collect();
        assert_eq!(branches, vec!["feature-a", "feature-b"]);
    }

    #[test]
    fn list_skips_corrupt_metadata_without_failing() {
        let tmp = TempDir::new().unwrap();
        let hub = Hub::init(&tmp.path().join("hub"), InitOptions::default()).unwrap();
        fake_workspace(&hub, "feature-good", &["a"]);

        let bad = hub.ws_dir().join("feature-bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join(META_FILE), "{ 坏掉的 json").unwrap();

        // 一个坏工作区不该让整个列表失败
        let branches: Vec<_> = list(&hub)
            .unwrap()
            .iter()
            .map(|w| w.branch().to_string())
            .collect();
        assert_eq!(branches, vec!["feature-good"]);
    }

    #[test]
    fn present_modules_filters_out_missing_dirs() {
        let tmp = TempDir::new().unwrap();
        let hub = Hub::init(&tmp.path().join("hub"), InitOptions::default()).unwrap();
        let ws = fake_workspace(&hub, "feature-x", &["there", "gone"]);

        // 只把 "there" 造成真实的 worktree 形态（目录 + .git 文件）
        let m = ws.module_path("there");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join(".git"), "gitdir: /nowhere").unwrap();

        assert_eq!(ws.present_modules(), vec!["there"]);
    }

    #[test]
    fn save_meta_persists_edits() {
        let tmp = TempDir::new().unwrap();
        let hub = Hub::init(&tmp.path().join("hub"), InitOptions::default()).unwrap();
        let mut ws = fake_workspace(&hub, "feature-x", &["a"]);

        ws.meta_mut().title = "改过的标题".into();
        ws.meta_mut().base = vec!["ws:feature-upstream".into()];
        ws.save_meta().unwrap();

        let back = Workspace::open(&hub, "feature-x").unwrap();
        assert_eq!(back.title(), "改过的标题");
        assert_eq!(back.meta().base, vec!["ws:feature-upstream"]);
    }
}