//! hub：工作区根目录的**定位、初始化与访问**。
//!
//! ## 什么是 hub
//!
//! hub 是统一管理所有仓库与需求工作区的根目录，结构见设计方案 §2.1：
//!
//! ```text
//! <hub>/
//! ├── .gws-hub            标记文件（靠它向上定位 hub）
//! ├── gws.json            hub 级配置
//! ├── repos/<仓库名>/      唯一的 .git 存放处（主干镜像）
//! ├── envs/<环境标识>/     各环境的 worktree
//! ├── ws/<完整分支名>/     需求工作区
//! └── repos/doc-hub/    文档库（特殊仓库：跟其它仓库一样在 repos/ 下，但不可移除）
//! ```
//!
//! ## 标记文件为什么是 `.gws-hub`
//!
//! 新旧两版工具对 hub 的内部约定不同（`ws/` 下目录名、是否有 `docs/`、
//! 配置文件格式），**互相接管对方的 hub 会出错**。用不同的标记文件，
//! 新版走进旧 hub 时会直接报「不在任何 hub 目录内」，而不是自信地做错事。

pub mod config;
pub mod repair;

use std::path::{Path, PathBuf};

use crate::env::EnvDef;
use crate::{Error, Result};

pub use config::{Defaults, GitIdentity, GitlabBinding, HubConfig, QuickLink, CONFIG_FILE, SCHEMA};

/// hub 标记文件名。
pub const MARKER_FILE: &str = ".gws-hub";

pub const REPOS_DIR: &str = "repos";
pub const ENVS_DIR: &str = "envs";
pub const WS_DIR: &str = "ws";
/// 文档库目录名的**兜底值**（配置的 URL 解析不出仓库名时用）。
///
/// 正常情况下不会用到 —— 名字一律由 [`Hub::dochub_name`] 从 URL 推导。
pub const FALLBACK_DOCHUB_NAME: &str = "doc-hub";

/// 历史上用过的文档库目录名，打开 hub 时会把它们搬到现在的位置。
///
/// ⚠️ **绝不能包含 [`Hub::dochub_name`] 可能返回的值** —— 那会让迁移逻辑把
/// 当前目录当成历史目录搬走。所以这里只列确实用过的旧名。
pub const LEGACY_DOCHUB_NAMES: [&str; 2] = ["doc-hub", "x-doc-hub"];

/// 旧版需求里文档目录的名字。只在迁移时读。
pub const LEGACY_DOCS_DIR: &str = "docs";

pub const CLAUDE_FILE: &str = "CLAUDE.md";
pub const AGENTS_FILE: &str = "AGENTS.md";

/// 约定说明文件的**两个名字**。
///
/// `AGENTS.md` 是**全文**（Codex / Cursor 等一批工具都读它，业界正在往它收敛）；
/// `CLAUDE.md` 只写一句「请先阅读同目录 AGENTS.md」的指针。
///
/// 以前两份写同样的内容，结果是同一段话复制两遍、改一处忘一处。
pub const AGENT_DOC_NAMES: [&str; 2] = [CLAUDE_FILE, AGENTS_FILE];

fn claude_pointer() -> String {
    "请先阅读同目录的 AGENTS.md。\n\n\
     完整约定写在 AGENTS.md 里；这份 CLAUDE.md 只是指针，避免两份内容各自漂移。\n"
        .to_string()
}

/// 写 CLAUDE.md 指针。
///
/// 只在「文件不存在 / 已经是指针 / 还是旧版生成的全文」时改写；
/// **人手工改过的内容不覆盖**。
fn write_claude_pointer(path: &Path, old_full_text: &str) -> Result<()> {
    let pointer = claude_pointer();
    if let Ok(existing) = std::fs::read_to_string(path) {
        let existing = existing.trim();
        if existing != old_full_text.trim() && existing != pointer.trim() {
            return Ok(());
        }
    }
    std::fs::write(path, pointer).map_err(|e| Error::io(path.to_path_buf(), e))
}

/// 把说明写到约定文件名下：`AGENTS.md` 全文，`CLAUDE.md` 只写指针。
pub fn write_agent_doc(dir: &Path, text: &str) -> Result<()> {
    let agents = dir.join(AGENTS_FILE);
    std::fs::write(&agents, text).map_err(|e| Error::io(agents, e))?;
    write_claude_pointer(&dir.join(CLAUDE_FILE), text)
}

/// 同 [`write_agent_doc`]，但**只补缺的文件**。
///
/// 用在"这一期可能已经有别人写好的那份"的地方（导入别人的需求）：覆盖会丢内容，
/// 还会让本来干净的目录多出未提交改动。旧版生成过完整全文的 `CLAUDE.md`
/// 会被迁移成指针；人改过的则原样保留。
pub fn write_agent_doc_missing_only(dir: &Path, text: &str) -> Result<()> {
    let agents = dir.join(AGENTS_FILE);
    if !agents.exists() {
        std::fs::write(&agents, text).map_err(|e| Error::io(agents, e))?;
    }
    write_claude_pointer(&dir.join(CLAUDE_FILE), text)
}

/// 一个已打开的 hub。
#[derive(Debug, Clone)]
pub struct Hub {
    root: PathBuf,
    config: HubConfig,
    /// 本次打开时做的目录布局迁移（已是最新布局时为默认值）。
    /// 调用方（CLI / 桌面端）据此告诉用户「我动了什么」。
    migration: crate::dochub::MigrationOutcome,
    /// 本次打开时**修复过的失效 worktree 链接**的说明（没坏就是空）。
    ///
    /// 与 `migration` 分开是因为它是另一件事（搬动 hub 造成的），但界面
    /// 走的是同一条「我动了什么」通道（`migration_notes`）。
    repairs: Vec<String>,
}

impl Hub {
    /// 从 `start` 向上逐级查找标记文件，返回 hub 根目录。
    pub fn locate(start: &Path) -> Result<PathBuf> {
        let mut dir = absolutize(start)?;
        loop {
            if dir.join(MARKER_FILE).is_file() {
                return Ok(dir);
            }
            if !dir.pop() {
                return Err(Error::HubNotFound);
            }
        }
    }

    /// 打开一个已有 hub。`root` 必须是 hub 根目录本身，不会向上查找。
    ///
    /// 打开时顺手把文档目录迁到新布局（实测反馈 #9）。放在这里是因为
    /// **所有路径计算都在此之后**——晚一步迁，读到 `ws/<分支>/doc-hub`
    /// 就会当成"文档没挂"，界面显示成未挂载，而文件其实还在旧位置。
    /// 迁移是幂等的：已是新布局时零开销。
    pub fn open(root: &Path) -> Result<Self> {
        let root = absolutize(root)?;
        if !root.join(MARKER_FILE).is_file() {
            return Err(Error::InvalidHub(root));
        }
        let config = HubConfig::load(&root.join(CONFIG_FILE))?;
        let mut hub = Self {
            root,
            config,
            migration: Default::default(),
            repairs: Vec::new(),
        };
        // CLAUDE.md 由工具拥有（内容随版本更新），打开时刷新一次 ——
        // 否则老 hub 里会一直留着旧文案。它明确声明了「别在它上手写」。
        let _ = hub.write_claude_md();
        // 迁移失败**不阻断打开**：旧布局也还能用（只是文档目录名不同）。
        // 结果挂在 hub 上由调用方取用 —— core 不打印任何东西（§1 硬约束）。
        hub.migration = crate::dochub::migrate_layout(&hub);
        // worktree 链接的修复放在迁移**之后**：迁移会移动目录，先迁完再修链接，
        // 免得修的是马上又要挪的位置。**没坏时零代价**（只读几个小文件）。
        hub.repairs = repair::repair_broken_worktrees(&hub);
        // 凭据文件的 gitignore 防护：老 hub 可能还没有这一行。
        // 失败不阻断打开 —— 这只是个防护，不是 hub 能不能用的前提。
        let _ = ensure_gitignore(&hub.root);
        Ok(hub)
    }

    /// 从任意目录打开：先定位，再打开。
    pub fn open_from(start: &Path) -> Result<Self> {
        Self::open(&Self::locate(start)?)
    }

    /// 初始化一个新 hub。
    pub fn init(target: &Path, opts: InitOptions) -> Result<Self> {
        let root = absolutize(target)?;

        if root.join(MARKER_FILE).is_file() {
            return Err(Error::other(format!(
                "该目录已是 hub: {}（直接使用: cd {}）",
                root.display(),
                root.display()
            )));
        }

        // 嵌套 hub 会让「向上查找标记文件」产生歧义：从内层 hub 的 ws/ 往上一路
        // 都能找到标记，究竟属于哪个 hub 无从判断。旧版同款检查。
        if let Some(parent) = root.parent() {
            if let Ok(outer) = Self::locate(parent) {
                return Err(Error::NestedHub(outer));
            }
        }

        if root.exists() && !opts.allow_non_empty && is_non_empty_dir(&root)? {
            return Err(Error::other(format!(
                "目录非空: {}（换空目录，或用 --force 确认在此初始化）",
                root.display()
            )));
        }

        let name = opts.name.clone().unwrap_or_else(|| {
            root.file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "hub".to_string())
        });

        let mut config = HubConfig::new(name);
        if !opts.envs.is_empty() {
            config.envs = opts.envs.clone();
        }
        // 提交身份：建 hub 时就写进去（之后可在设置页改，与地址不同、不锁定）。
        // 留空 = 用 git 全局配置，`apply_identity` 会因此清掉仓库里的旧值。
        config.git = opts.identity.clone();

        // **连通性校验放在建目录之前。**
        //
        // 地址写错 / token 无效 / 网络不通是这里最常见的失败。先校验就不需要
        // 「回滚已建目录」那套逻辑 —— 失败时磁盘上什么都没留下，这是最可靠的
        // 「不留残骸」实现。
        if let Some(b) = &opts.gitlab {
            let host = validate_binding(b)?;
            let client = crate::gitlab::Client::new(b.base_url.clone(), b.token.clone())?;
            client.current_user().map_err(|e| {
                Error::other(format!(
                    "连不上这个 GitLab（或 token 无效），hub 未创建：\n  {e}"
                ))
            })?;
            // 文档库也要真的可用（同域名 + 存在 + 有权限）—— 它同样落盘后不可改，
            // 建出一个指向不存在仓库的 hub 等于直接把它废掉
            let doc_path = validate_dochub(&client, &host, &b.dochub_url)?;
            config.gitlab.base_url = b.base_url.trim().to_string();
            // 统一存成 https 形式：clone / fetch 时能直接带 token，
            // 而 `git@` 形式要靠 SSH key（桌面端拿不到用户的 ssh-agent）
            config.dochub.url = format!("https://{host}/{doc_path}.git");
        }
        config.validate()?;

        // 先把结构建齐再写标记文件——标记文件存在即代表 hub 可用。
        // 中途失败时不要留下一个"标记有了但目录不全"的半成品 hub。
        for sub in [REPOS_DIR, WS_DIR] {
            let p = root.join(sub);
            std::fs::create_dir_all(&p).map_err(|e| Error::io(p, e))?;
        }
        for env in &config.envs {
            let p = root.join(ENVS_DIR).join(&env.id);
            std::fs::create_dir_all(&p).map_err(|e| Error::io(p, e))?;
        }

        std::fs::write(root.join(MARKER_FILE), MARKER_CONTENT)
            .map_err(|e| Error::io(root.join(MARKER_FILE), e))?;
        config.save(&root.join(CONFIG_FILE))?;
        // 凭据写在 hub 目录里（0600），并保证 `.gitignore` 挡住它
        if let Some(b) = &opts.gitlab {
            crate::gitlab::token::store(&root, &b.token)?;
        }
        ensure_gitignore(&root)?;

        let hub = Self {
            root,
            config,
            migration: Default::default(),
            repairs: Vec::new(),
        };
        hub.write_claude_md()?;
        Ok(hub)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn repos_dir(&self) -> PathBuf {
        self.root.join(REPOS_DIR)
    }

    pub fn envs_dir(&self) -> PathBuf {
        self.root.join(ENVS_DIR)
    }

    pub fn env_dir(&self, env_id: &str) -> PathBuf {
        self.envs_dir().join(env_id)
    }

    pub fn ws_dir(&self) -> PathBuf {
        self.root.join(WS_DIR)
    }

    /// 文档库主 clone 的**规范位置**：`repos/<文档库仓库名>`。
    ///
    /// 只在迁移里用。判断"用哪个目录"一律走 [`Hub::dochub_dir`]。
    pub fn dochub_new_dir(&self) -> PathBuf {
        self.root.join(REPOS_DIR).join(self.dochub_name())
    }

    /// 历史上出现过的文档库位置（hub 根下），逐个列出。
    ///
    /// 返回**所有**候选（含新位置），调用方挑第一个存在的 —— 因为旧名可能有好几个
    /// （`doc-hub`、`x-doc-hub`……），而且它们与新位置不一定同名。
    pub fn legacy_dochub_dirs(&self) -> Vec<PathBuf> {
        LEGACY_DOCHUB_NAMES
            .iter()
            .map(|n| self.root.join(n))
            .collect()
    }

    /// **实际要用的**文档库目录。
    ///
    /// 文档库在 hub 里的**目录名** —— 从配置的 URL 推导，与你在 GitLab 上看到的
    /// 仓库名一致。
    ///
    /// # 为什么不是写死的常量（这是一个真实踩到的坑）
    ///
    /// 早先这里写死成 `doc-hub`：用户填的文档库是 `x-doc-hub`，hub 里却建出个
    /// `doc-hub` 目录 —— 对不上号，看着就像"文档没拉下来"（实测反馈）。
    /// 现在一律取 URL 里的仓库名：填 `x-doc-hub` 就叫 `x-doc-hub`，
    /// 填 `docs` 就叫 `docs`。
    ///
    /// URL 为空或解析不出时退回 [`FALLBACK_DOCHUB_NAME`]。
    pub fn dochub_name(&self) -> String {
        crate::repo::name_from_url(&self.config.dochub.url)
            .unwrap_or_else(|| FALLBACK_DOCHUB_NAME.to_string())
    }

    /// 这个名字是不是「本 hub 的文档库」。
    ///
    /// 判据只有这一处 —— 别在别处再写 `== "doc-hub"`，那会散成好几份，
    /// 而且改起来必漏。
    pub fn is_dochub(&self, name: &str) -> bool {
        name == self.dochub_name()
    }

    /// 旧位置还在就以它为准 —— 迁移完成前它才是那个"名下挂着各需求文档 worktree"
    /// 的 clone。这一点在遇到**同名重复 clone** 时尤其重要：
    /// 用户可能早就把 `doc-hub` 当普通仓库加过一次（于是 `repos/doc-hub`
    /// 是个没有 worktree 的空壳）。若只看新路径，工具会打开那个空壳，
    /// 而真正的文档库被晾在一边，文档功能看起来"全是空的"。
    ///
    /// 迁移成功之后旧位置不复存在，这里自然落到新路径。
    pub fn dochub_dir(&self) -> PathBuf {
        for legacy in self.legacy_dochub_dirs() {
            if legacy.join(".git").is_dir() {
                return legacy;
            }
        }
        self.dochub_new_dir()
    }

    /// 本次打开时发生的目录迁移（空 = 没动任何东西）。
    pub fn migration(&self) -> &crate::dochub::MigrationOutcome {
        &self.migration
    }

    pub fn config(&self) -> &HubConfig {
        &self.config
    }

    /// 取可变配置。改完必须调 [`Hub::save_config`]，否则只在内存里。
    /// 本次打开时修复失效 worktree 链接的说明（空 = 什么都没坏）。
    pub fn repairs(&self) -> &[String] {
        &self.repairs
    }

    pub fn config_mut(&mut self) -> &mut HubConfig {
        &mut self.config
    }

    pub fn save_config(&self) -> Result<()> {
        self.config.save(&self.root.join(CONFIG_FILE))?;
        Ok(())
    }

    /// 本 hub 绑定的 GitLab 根地址。未绑定时给一条**带下一步**的错误。
    pub fn gitlab_base_url(&self) -> Result<String> {
        let u = self.config.gitlab.base_url.trim();
        if u.is_empty() {
            return Err(Error::other(
                "这个 hub 还没绑定 GitLab 地址。\n  \
                 命令行：gws gitlab bind --url https://your.gitlab --token <PAT>\n  \
                 桌面端：设置 → GitLab 里填写地址",
            ));
        }
        Ok(u.to_string())
    }

    /// 本 hub 绑定的 GitLab 主机名（仓库来源校验用）。
    pub fn gitlab_host(&self) -> Result<String> {
        let url = self.gitlab_base_url()?;
        let host = crate::gitlab::remote_host(&url);
        if host.is_empty() {
            return Err(Error::other(format!("hub 绑定的 GitLab 地址看不懂: {url}")));
        }
        Ok(host.to_string())
    }

    /// 本 hub 的凭据文件路径（`<hub>/gitlab-token`）。
    pub fn token_file(&self) -> PathBuf {
        crate::gitlab::token::token_file(&self.root)
    }

    /// 设置提交身份，并把身份**同步到 hub 里已有的所有仓库**。
    ///
    /// 返回实际写过的仓库数。新加的仓库由 `repo::add` 在 clone 后自动带上身份。
    pub fn set_identity(&mut self, id: GitIdentity) -> Result<usize> {
        self.config.git = id;
        self.save_config()?;
        self.sync_identity_to_repos()
    }

    /// 把当前身份写进每个已 clone 仓库的 local config。
    ///
    /// 只遍历 `repos/` 下的**主 clone** —— hub 里的 worktree 与它们共享同一份
    /// local config（见 [`crate::git::apply_identity`]），所以不需要（也不该）
    /// 逐个 worktree 去设。
    pub fn sync_identity_to_repos(&self) -> Result<usize> {
        let Ok(entries) = std::fs::read_dir(self.repos_dir()) else {
            // 还没有 repos/ 目录 = 没有任何仓库要同步，不是错误
            return Ok(0);
        };
        let mut n = 0;
        for e in entries.flatten() {
            let p = e.path();
            // 只认主 clone：它的 `.git` 是目录；worktree 的 `.git` 是文件
            if !p.join(".git").is_dir() {
                continue;
            }
            crate::git::apply_identity(&p, &self.config.git)?;
            n += 1;
        }
        Ok(n)
    }

    /// 一次性绑定 GitLab：校验格式与连通性 → 写凭据 → 落盘地址。
    ///
    /// **已绑定则拒绝** —— 这是「地址不可改」的第一道闸门；第二道在
    /// [`HubConfig::save`] 里，兜住任何别的修改路径。
    pub fn bind_gitlab(&mut self, input: &GitlabBindingInput) -> Result<()> {
        self.bind_gitlab_with(input, |c| {
            c.current_user()?;
            // 文档库同样要联网校验（存在 + 有权限）；「同域名」那条在
            // `bind_gitlab_with` 里已经拦过，跑不掉
            validate_dochub(c, crate::gitlab::remote_host(&input.base_url), &input.dochub_url)?;
            Ok(())
        })
    }

    /// 同 [`Hub::bind_gitlab`]，但连通性探测可注入 —— 离线单测用它验证
    /// 「校验失败时磁盘上不留东西」。
    pub fn bind_gitlab_with(
        &mut self,
        input: &GitlabBindingInput,
        probe: impl Fn(&crate::gitlab::Client) -> Result<()>,
    ) -> Result<()> {
        if self.config.gitlab.is_bound() {
            return Err(Error::other(
                "这个 hub 已经绑定过 GitLab 地址，不允许修改。\n  \
                 地址是「哪些仓库能纳管」与文档库的依据，落盘后就固定了；\n  \
                 确实要换实例，请新建一个 hub。",
            ));
        }
        let host = validate_binding(input)?;
        // 「文档库同域名 + 格式」是**纯本地**校验，放在这里 ——
        // 即使调用方注入了别的 probe，这条也跑不掉
        let doc_path = check_dochub_host(&host, &input.dochub_url)?;

        // 先校验、后落盘：不会留下「地址写了但 token 没写成」这种半截状态
        let client = crate::gitlab::Client::new(input.base_url.clone(), input.token.clone())?;
        probe(&client)?;

        crate::gitlab::token::store(&self.root, &input.token)?;
        self.config.gitlab.base_url = input.base_url.trim().to_string();
        // 统一存成 https 形式（clone / fetch 时能直接带 token）
        self.config.dochub.url = format!("https://{host}/{doc_path}.git");
        self.save_config()?;
        ensure_gitignore(&self.root)?;
        Ok(())
    }

    /// 写出（或刷新）hub 根的说明文件。
    ///
    /// `AGENTS.md` 是全文、由工具拥有，打开时刷新；`CLAUDE.md` 只是指向它的
    /// 指针（人改过的不会覆盖）。
    pub fn write_claude_md(&self) -> Result<()> {
        // AGENTS.md 全文 + CLAUDE.md 指针（见 AGENT_DOC_NAMES）
        write_agent_doc(&self.root, &hub_claude_md(self))
    }

    /// 重新从磁盘读配置（外部改了文件时用）。
    pub fn reload_config(&mut self) -> Result<()> {
        self.config = HubConfig::load(&self.root.join(CONFIG_FILE))?;
        Ok(())
    }
}

/// hub 根 `AGENTS.md` 的内容（设计方案 §7.1；`CLAUDE.md` 只是指针）。
///
/// **这是 AI 进入 hub 时最先读到的东西**，要一次说清：同一个仓库在这里有多份
/// 副本、各自是什么、该看哪份；以及文档该怎么放、哪些不能动。
pub fn hub_claude_md(hub: &Hub) -> String {
    // 文档库目录名**从配置推导**（用户填的地址决定它叫 doc-hub 还是 x-doc-hub）——
    // 这份说明是 AI 进 hub 读的第一份文件，写死一个名字会让它去一个不存在的目录里找。
    let doc = hub.dochub_name();
    let envs = hub
        .config()
        .envs
        .iter()
        .map(|e| e.id.clone())
        .collect::<Vec<_>>()
        .join(" / ");
    format!(
        "# GWS · 工作区说明\n\
         \n\
         > 本目录由 GWS管理。**同一个仓库在这里有多份副本，用途完全不同，\
         分析前务必先确认你要看的是哪一份。**\n\
         \n\
         ## 目录语义\n\
         \n\
         | 目录 | 是什么 | 代码状态 | AI 该不该看 |\n\
         |---|---|---|---|\n\
         | `repos/<仓库>/` | **生产干净代码**。各仓库的主干镜像 | `gws sync` 会 reset 到 `origin/<主干>` | ✅ 想了解「线上现状是什么」时看这里。**只读，不要改** |\n\
         | `envs/<环境>/<仓库>/` | **测试环境代码**。环境分支的 worktree | 跟随 `origin/<环境分支>`，由发布流程推进 | ✅ 想了解「测试环境上是什么」时看这里。**默认只读** —— 唯一例外见下面「合错代码」 |\n\
         | `ws/<分支名>/<仓库>/` | **需求目录**。当前需求的开发工作区 | 各仓库都固定在 `<分支名>` 上，含本需求全部改动 | ✅✅ **改代码、查本需求引入的问题，只看这里** |\n\
         | `repos/{doc}/` | **文档库**（特殊仓库）。文档库的 clone | 已归档的历史文档在 `main` 上 | ✅ 查需求文档、技术方案、历史设计时看这里 |\n\
         \n\
         环境：{envs}\n\
         \n\
         ## 文档规则（重要）\n\
         \n\
         文档分两块，**别搞混**：\n\
         \n\
         1. **已归档的历史文档** —— `repos/{doc}/workspaces/<需求分类>/<分支>/` 里，\
            该分支**已并入 `main`** 的那些。\n\
            ⚠️ **只读**：它们是当时的事实记录，是背景资料。**不要修改、不要重写、\
            不要「顺手更新一下」。** 需要纠正历史记录时，写在当前需求的文档里说明。\n\
         \n\
         2. **正在开发的需求文档** —— `ws/<当前分支>/{doc}/workspaces/<需求分类>/<分支>/`。\n\
            ✅ **AI 产生的文档一律写这里**，不要写进 `repos/{doc}/` 的工作树。\n\
            常用类型：需求文档、设计方案、技术方案、说明文档、使用说明、代码导读、\
            提测单、测试用例、sql、发布步骤、实施步骤。用 `gws doc new <类型>` 生成模板。\n\
         \n\
         ## 改代码的规则\n\
         \n\
         1. **只改 `ws/<当前分支>/` 下的代码。** `repos/` 与 `envs/` 默认都是镜像，\
            改动会被下一次同步抹掉。唯一的例外是下面那节「合错代码」。\n\
         2. **分析需求时不要跨目录检索**：只在 `ws/<当前分支>/` 里搜，\
            检索范围即改动范围。需要对照线上现状时再去 `repos/` 读。\n\
         3. **不要切分支**：每个模块目录已固定在需求分支上。\n\
         \n\
         ## 合错代码的处理（改 `envs/` 的唯一例外）\n\
         \n\
         如果某次合并**把环境搞错了** —— 不该进的代码合了进去、该合的没合上、\
         或者留下冲突残留 —— 允许**临时**直接在 `envs/<环境>/<仓库>/` 里改，\
         并在**该环境分支**上提交，把环境修回它该有的样子。\n\
         \n\
         这是「改 `ws/`」规则的唯一例外，边界要守住：\n\
         \n\
         - **只处理合错本身**：撤掉误合的改动、把漏掉的提交补进来、清掉冲突残留。\n\
         - **不是需求开发**。凡是需求本身的改动，一律回 `ws/<分支名>/` 去改。\
           在环境分支上写需求代码会让环境与需求分支分叉，两边再也对不上。\n\
         - **动手前先看清现在是什么**：`git log --oneline -5`，确认你要撤的正是那笔\
           合错的改动，别把别人的东西一起改了。\n\
         - **提交说明写清「为什么动环境分支」**，否则下一个人看到环境分支上多出\
           一笔莫名的提交，无从判断。\n\
         - **改完告诉人**：动了哪个仓库、哪个分支、改了什么、为什么。\n\
           推送由人执行 —— 环境分支推上去就等于改了测试环境。\n\
         \n\
         ## 提交约定（硬性）\n\
         \n\
         - **AI 只能 `git add`**：不得自行 `git commit` / `git push`。\n\
           范围是**在这个 hub 里写的任何文件** —— 代码、文档、脚本、配置都算，\n\
           **不只是文档**。\n\
         - 改完只 `git add`，或者干脆只展示变更；由人 review 后手动提交，\n\
           除非人明确说「直接提交」。\n\
         - **原因**：AI 自己提交之后，人还没读过就已经推上去了；提交几次之后，\n\
           谁也说不清它到底改了什么。这是最实际的一条痛点。\n\
         - 文档同理：`gws doc status` 看变更 → 人确认 → `gws doc sync`。\n\
         \n\
         > **例外只有一处**：上面「合错代码的处理」那种情形，允许 AI 在**环境分支**上\
         提交修正。其余一切提交仍然要人来做。\n",
        envs = envs,
    )
}

const MARKER_CONTENT: &str = "\
# GWS hub 标记文件 —— 靠它向上定位 hub 根目录，勿删。\n\
# 详细目录说明见 hub 根目录的 AGENTS.md（CLAUDE.md 只是指向它的指针）。\n";

/// [`Hub::init`] 的可选项。
#[derive(Debug, Clone, Default)]
pub struct InitOptions {
    /// hub 名称。缺省用目录名。
    pub name: Option<String>,
    /// 初始环境列表。缺省只建 `dev`。
    pub envs: Vec<EnvDef>,
    /// 目录非空时是否仍继续。
    pub allow_non_empty: bool,
    /// GitLab 绑定。建 hub 时给（CLI / 桌面端都要求必填）。
    /// core 允许 `None`，以便离线创建、之后再 `bind_gitlab` 补绑，也方便测试。
    pub gitlab: Option<GitlabBindingInput>,
    /// 提交身份（`user.name` / `user.email`）。
    ///
    /// 建 hub 时一并写进配置 —— 桌面端会拿 git 全局配置**预填**，用户改一下即可。
    /// 留空则用 git 全局配置。**之后随时可改**（与地址不同，它不锁定）。
    pub identity: GitIdentity,
}

/// 绑定一个 GitLab 实例需要的三样东西。
///
/// 放在一起是因为它们必须**同时**提供：地址决定能纳管哪些仓库、token 决定以
/// 什么身份操作、文档库地址决定文档挂在哪 —— 少一样都不成立。
#[derive(Debug, Clone)]
pub struct GitlabBindingInput {
    /// 实例根地址，如 `https://gitlab.example.com`。
    pub base_url: String,
    /// 有 `api` 权限的 Personal Access Token。
    pub token: String,
    /// 文档库仓库地址（`https://…/doc-hub.git` 或 `git@…`）。
    pub dochub_url: String,
}

/// 校验绑定三件套的**格式**。不联网 —— 连通性由调用方的 probe 负责。
/// 校验绑定的**格式**，返回 GitLab 的主机名（后续用来校验文档库同域名）。
/// 不联网 —— 连通性由调用方的 probe 负责。
fn validate_binding(b: &GitlabBindingInput) -> Result<String> {
    let url = b.base_url.trim();
    if url.is_empty() {
        return Err(Error::other("GitLab 地址不能为空"));
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(Error::other(format!(
            "GitLab 地址必须是 http/https 开头，当前为 '{url}'"
        )));
    }
    if b.token.trim().is_empty() {
        return Err(Error::other("GitLab token 不能为空（需要 api 权限）"));
    }
    let doc = b.dochub_url.trim();
    if doc.is_empty() {
        return Err(Error::other("文档库地址不能为空"));
    }
    Ok(crate::gitlab::remote_host(url).to_string())
}

/// 从 git 地址里解析出 GitLab 项目路径（`group/repo`）。
///
/// 认 `https://<host>/<group>/<repo>.git`、`http://…`、`git@<host>:<group>/<repo>.git`、
/// `ssh://git@<host>/<group>/<repo>.git` 四种写法。解析不出来返回 `None`。
fn project_path_from_url(url: &str) -> Option<String> {
    let u = url.trim().trim_end_matches('/');
    let rest = if let Some(r) = u.strip_prefix("git@") {
        r.split_once(':')?.1
    } else if let Some(r) = u.strip_prefix("ssh://") {
        r.split_once('/')?.1
    } else {
        u.split_once("://")?.1.split_once('/')?.1
    };
    let path = rest.trim_end_matches(".git").trim_matches('/');
    // 必须是 `组/仓库` —— 只有一段说明地址给错了（比如只填了 GitLab 根地址）
    (path.contains('/') && !path.ends_with('/')).then(|| path.to_string())
}

/// 校验文档库地址的**域名与格式**（纯本地，不联网），返回项目路径 `组/仓库`。
///
/// 单独拆出来有两个原因：
/// 1. 「必须同域」这条要能被离线测试钉住（联网那半截没法在单测里跑）；
/// 2. 即使调用方给 `bind_gitlab_with` 注入了别的 probe，这条也跑不掉。
fn check_dochub_host(hub_host: &str, url: &str) -> Result<String> {
    let host = crate::gitlab::remote_host(url);
    if host.is_empty() {
        return Err(Error::other(format!(
            "看不懂的文档库地址，hub 未创建：\n  {url}\n  \
             应为 https://<你的GitLab>/<组>/<仓库>.git\n  \
             或     git@<你的GitLab>:<组>/<仓库>.git"
        )));
    }
    // **必须同一个域名**：一个 hub 只绑一个实例 —— token、仓库来源校验都以它
    // 为准。跨实例的文档库既连不上（token 不是那个实例的），也会让
    // 「哪些仓库能纳管」这条规则失效。
    if host != hub_host {
        return Err(Error::other(format!(
            "文档库必须和 GitLab 在**同一个域名**下，hub 未创建：\n  \
             GitLab: {hub_host}\n  文档库: {host}\n  \
             一个 hub 只绑定一个实例，token 与仓库来源校验都以它为准。"
        )));
    }
    project_path_from_url(url).ok_or_else(|| {
        Error::other(format!(
            "看不懂的文档库地址，hub 未创建：\n  {url}\n  \
             要写成 <组>/<仓库> 的形式，例如 https://{hub_host}/team/docs.git"
        ))
    })
}

/// 校验文档库：**同域名 + 真的存在 + 当前 token 能访问**。
///
/// # 为什么必须联网校验（这是一个真实踩到的坑）
///
/// 文档库地址和 GitLab 地址一样，**落盘后不可改**。原先只校验了「是不是 http
/// 开头」这种格式，于是随手填一个不存在的仓库也能把 hub 建出来 —— 而建完就
/// 改不了了，整个 hub 只能删掉重建（里面可能已经有拉好的代码）。
/// 实测反馈：试用时故意乱填，结果 hub 直接废掉。
///
/// 返回 GitLab 上的真实路径（`group/repo`）。
pub fn validate_dochub(
    client: &crate::gitlab::Client,
    hub_host: &str,
    url: &str,
) -> Result<String> {
    let path = check_dochub_host(hub_host, url)?;
    let p = client.project(&path).map_err(|e| {
        Error::other(format!(
            "文档库地址不可用，hub 未创建：\n  {url}\n  {e}\n  \
             请确认这个仓库**存在**，且当前 token 有权限访问。\n  \
             没有的话先在 GitLab 上建一个空仓库（建议勾上「初始化 README」），\n  \
             再把它的地址填到这里 —— 文档库地址建完之后不可修改。"
        ))
    })?;
    Ok(p.path_with_namespace)
}

/// 保证 hub 根 `.gitignore` 里有那两行（挡住凭据文件与配置的临时文件）。
///
/// **只补缺失的行，不覆盖用户已有内容** —— 有人会把整个 hub 当仓库管理，
/// 那份 `.gitignore` 是他自己的东西。
fn ensure_gitignore(root: &Path) -> Result<()> {
    let path = root.join(".gitignore");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut text = existing.clone();
    let mut changed = false;
    for line in crate::gitlab::token::GITIGNORE_LINES {
        if !existing.lines().any(|l| l.trim() == line) {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(line);
            text.push('\n');
            changed = true;
        }
    }
    if changed {
        std::fs::write(&path, text).map_err(|e| Error::io(path, e))?;
    }
    Ok(())
}

/// 展开 `~`，转为绝对路径，并尽量消解 `..`。
///
/// 目标目录可能尚不存在，所以只能 canonicalize **父目录**再拼上最后一段。
pub fn absolutize(path: &Path) -> Result<PathBuf> {
    let expanded = expand_tilde(path);
    let abs = std::path::absolute(&expanded)
        .map_err(|e| Error::other(format!("无法解析路径 {}: {e}", expanded.display())))?;

    if let (Some(parent), Some(name)) = (abs.parent(), abs.file_name()) {
        if let Ok(real_parent) = parent.canonicalize() {
            // **去掉 Windows 的 `\\?\` 前缀**：canonicalize 在 Windows 上返回逐字路径，
            // 那玩意喂给 git 会直接失败（实测反馈：添加仓库时
            // `git clone <url> \\?\D:\…\repos\x` 报错），显示给人看也难看。
            return Ok(crate::paths::strip_verbatim_prefix(&real_parent.join(name)));
        }
    }
    Ok(abs)
}

fn expand_tilde(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if s == "~" {
        if let Some(home) = crate::paths::home_dir() {
            return home;
        }
    } else if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = crate::paths::home_dir() {
            return home.join(rest);
        }
    }
    path.to_path_buf()
}

fn is_non_empty_dir(path: &Path) -> Result<bool> {
    let mut entries = std::fs::read_dir(path).map_err(|e| Error::io(path.to_path_buf(), e))?;
    Ok(entries.next().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn agent_doc_writes_full_agents_and_pointer_claude() {
        let dir = TempDir::new().unwrap();
        write_agent_doc(dir.path(), "# 完整约定\n\n只改 ws/<当前分支>/\n").unwrap();

        let agents = std::fs::read_to_string(dir.path().join(AGENTS_FILE)).unwrap();
        let claude = std::fs::read_to_string(dir.path().join(CLAUDE_FILE)).unwrap();
        assert!(agents.contains("完整约定"), "{agents}");
        assert!(claude.contains("请先阅读同目录的 AGENTS.md"), "{claude}");
        assert!(
            !claude.contains("只改 ws/<当前分支>/"),
            "指针里不该复制全文：{claude}"
        );
    }

    #[test]
    fn old_full_claude_is_migrated_but_user_edits_are_kept() {
        let dir = TempDir::new().unwrap();
        let full = "# 旧版全文\n";
        std::fs::write(dir.path().join(CLAUDE_FILE), full).unwrap();
        write_agent_doc(dir.path(), full).unwrap();
        assert!(std::fs::read_to_string(dir.path().join(CLAUDE_FILE))
            .unwrap()
            .contains("请先阅读同目录的 AGENTS.md"));

        // 人改过的 CLAUDE.md 保留，不被指针覆盖
        let mine = "# 我自己写的说明\n";
        std::fs::write(dir.path().join(CLAUDE_FILE), mine).unwrap();
        write_agent_doc(dir.path(), "# 新版全文\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join(CLAUDE_FILE)).unwrap(),
            mine
        );
        // AGENTS.md 仍然刷新（它是工具拥有的全文）
        assert!(std::fs::read_to_string(dir.path().join(AGENTS_FILE))
            .unwrap()
            .contains("新版全文"));
    }

    #[test]
    fn missing_only_does_not_overwrite_existing_agents() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(AGENTS_FILE), "# 别人的精简版\n").unwrap();
        write_agent_doc_missing_only(dir.path(), "# 工具全文\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join(AGENTS_FILE)).unwrap(),
            "# 别人的精简版\n"
        );
        // CLAUDE 缺 → 补指针
        assert!(dir.path().join(CLAUDE_FILE).is_file());
    }

    #[test]
    fn init_creates_structure_and_marker() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("myhub");

        let hub = Hub::init(&root, InitOptions::default()).unwrap();

        assert!(root.join(MARKER_FILE).is_file());
        assert!(root.join(CONFIG_FILE).is_file());
        assert!(hub.repos_dir().is_dir());
        assert!(hub.ws_dir().is_dir());
        assert!(hub.env_dir("dev").is_dir());
        assert_eq!(hub.config().name, "myhub");
    }

    #[test]
    fn init_honours_custom_envs() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("h");
        let opts = InitOptions {
            envs: vec![EnvDef::new("dev", "开发"), EnvDef::new("pre", "预发")],
            ..Default::default()
        };
        let hub = Hub::init(&root, opts).unwrap();

        assert!(hub.env_dir("dev").is_dir());
        assert!(hub.env_dir("pre").is_dir());
        assert!(hub.config().env("pre").is_some());
    }

    #[test]
    fn locate_walks_up_from_nested_dir() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("hub");
        Hub::init(&root, InitOptions::default()).unwrap();

        let deep = root.join("ws").join("feature-x").join("some-repo");
        std::fs::create_dir_all(&deep).unwrap();

        // Windows 的 `canonicalize` 会带 `\\?\` 逐字前缀，而 `Hub::locate` 按设计
        // 会把它剥掉（见 `paths::strip_verbatim_prefix`）—— 断言要跟着剥，否则只有
        // Windows 红。
        let want = crate::paths::strip_verbatim_prefix(&root.canonicalize().unwrap());
        assert_eq!(Hub::locate(&deep).unwrap(), want);
    }

    #[test]
    fn locate_fails_outside_any_hub() {
        let dir = TempDir::new().unwrap();
        let err = Hub::locate(dir.path()).unwrap_err();
        assert!(matches!(err, Error::HubNotFound), "{err}");
    }

    #[test]
    fn open_from_locates_then_opens() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("hub");
        Hub::init(&root, InitOptions::default()).unwrap();

        let nested = root.join("repos");
        let hub = Hub::open_from(&nested).unwrap();
        let want = crate::paths::strip_verbatim_prefix(&root.canonicalize().unwrap());
        assert_eq!(hub.root(), want);
    }

    #[test]
    fn open_rejects_non_hub_dir() {
        let dir = TempDir::new().unwrap();
        let err = Hub::open(dir.path()).unwrap_err();
        assert!(matches!(err, Error::InvalidHub(_)), "{err}");
    }

    #[test]
    fn init_rejects_nested_hub() {
        let dir = TempDir::new().unwrap();
        let outer = dir.path().join("outer");
        Hub::init(&outer, InitOptions::default()).unwrap();

        let inner = outer.join("ws").join("inside");
        let err = Hub::init(&inner, InitOptions::default()).unwrap_err();
        assert!(matches!(err, Error::NestedHub(_)), "{err}");
    }

    #[test]
    fn init_rejects_existing_hub() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("hub");
        Hub::init(&root, InitOptions::default()).unwrap();
        let err = Hub::init(&root, InitOptions::default()).unwrap_err();
        assert!(err.to_string().contains("已是 hub"), "{err}");
    }

    #[test]
    fn init_rejects_non_empty_dir_unless_forced() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("hub");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("占位.txt"), "x").unwrap();

        assert!(Hub::init(&root, InitOptions::default()).is_err());

        let opts = InitOptions {
            allow_non_empty: true,
            ..Default::default()
        };
        assert!(Hub::init(&root, opts).is_ok());
    }

    #[test]
    fn init_does_not_leave_marker_when_config_invalid() {
        // 环境标识非法 → 校验失败 → 不应留下标记文件（否则是个半成品 hub）
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("hub");
        let opts = InitOptions {
            envs: vec![EnvDef::new("bad env", "坏")],
            ..Default::default()
        };

        assert!(Hub::init(&root, opts).is_err());
        assert!(!root.join(MARKER_FILE).exists());
    }

    #[test]
    fn config_survives_reopen_and_edit() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("hub");
        let mut hub = Hub::init(&root, InitOptions::default()).unwrap();

        hub.config_mut().envs.push(EnvDef::new("pre", "预发"));
        hub.config_mut()
            .env_mut("pre")
            .unwrap()
            .branches
            .insert("a".into(), "pre-202609".into());
        hub.save_config().unwrap();

        let reopened = Hub::open(&root).unwrap();
        assert_eq!(
            reopened.config().env("pre").unwrap().branch_for("a"),
            "pre-202609"
        );
    }

    #[test]
    fn absolutize_expands_tilde() {
        let home = crate::paths::home_dir().expect("测试需要主目录");
        assert_eq!(
            absolutize(Path::new("~/some/hub")).unwrap(),
            home.join("some/hub")
        );
    }

    #[test]
    fn absolutize_resolves_parent_dotdot() {
        let dir = TempDir::new().unwrap();
        let a = dir.path().join("a");
        std::fs::create_dir_all(&a).unwrap();
        let resolved = absolutize(&a.join("..").join("b")).unwrap();
        let want = crate::paths::strip_verbatim_prefix(&dir.path().canonicalize().unwrap());
        assert_eq!(resolved, want.join("b"));
    }
}
#[cfg(test)]
mod dochub_binding_tests {
    use super::{check_dochub_host, project_path_from_url};

    const HOST: &str = "gitlab.example.com";

    #[test]
    fn project_path_handles_all_four_forms() {
        // 文档库地址支持与仓库地址相同的四种写法
        let p = |u| project_path_from_url(u);
        assert_eq!(
            p("https://gitlab.example.com/team/docs.git").as_deref(),
            Some("team/docs")
        );
        assert_eq!(p("http://gitlab.example.com/team/docs").as_deref(), Some("team/docs"));
        assert_eq!(p("git@gitlab.example.com:team/docs.git").as_deref(), Some("team/docs"));
        assert_eq!(
            p("ssh://git@gitlab.example.com/team/docs.git").as_deref(),
            Some("team/docs")
        );
        // 结尾的斜杠不该影响
        assert_eq!(p("https://gitlab.example.com/team/docs/").as_deref(), Some("team/docs"));
        // 多级 group 也要完整保留
        assert_eq!(
            p("https://gitlab.example.com/a/b/docs.git").as_deref(),
            Some("a/b/docs")
        );
    }

    #[test]
    fn one_segment_url_is_not_a_project_path() {
        // 只填了 GitLab 根地址、或只有一段路径 —— 都不是「组/仓库」
        for bad in [
            "https://gitlab.example.com/",
            "https://gitlab.example.com",
            "https://gitlab.example.com/docs.git",
            "",
            "不是地址",
        ] {
            assert_eq!(project_path_from_url(bad), None, "不该被当成项目路径: {bad}");
        }
    }

    #[test]
    fn dochub_on_the_same_host_is_accepted() {
        let path = check_dochub_host(HOST, "https://gitlab.example.com/team/docs.git")
            .expect("同域名应当通过");
        assert_eq!(path, "team/docs");
    }

    /// 用户明确要求：**文档库必须和 GitLab 在同一个域名下**。
    ///
    /// 一个 hub 只绑一个实例 —— token、仓库来源校验都以它为准；跨实例的文档库
    /// 既连不上（token 不是那个实例的），也让「哪些仓库能纳管」失效。
    #[test]
    fn dochub_on_another_host_is_rejected_and_says_why() {
        for bad in [
            "https://other.example.com/team/docs.git",
            "git@other.example.com:team/docs.git",
            "https://gitlab.example.com.evil.com/team/docs.git", // 后缀伪装也不行
        ] {
            let err = check_dochub_host(HOST, bad).unwrap_err().to_string();
            assert!(err.contains("同一个域名"), "要说清是域名问题：{err}");
        }
    }

    #[test]
    fn unparseable_dochub_url_is_rejected_as_such() {
        let err = check_dochub_host(HOST, "不是地址").unwrap_err().to_string();
        assert!(err.contains("看不懂"), "解析不出来要给「看不懂」而不是「域名不对」：{err}");
    }
}
