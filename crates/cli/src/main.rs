//! `gws` —— GWS的命令行入口。
//!
//! 本 crate **只做参数解析与结果渲染**，所有逻辑在 `gws-core`。
//! 桌面端直接调用 core，不经过这里（设计方案 §1.3）。

mod render;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use gws_core::dochub::{AttachMode, DocHub, PeriodMeta};
use gws_core::env::EnvDef;
use gws_core::gitlab::{self, Client};
use gws_core::hub::{GitIdentity, GitlabBindingInput, Hub, InitOptions};
use gws_core::progress::stderr_reporter;
use gws_core::repo;
use gws_core::workspace::lifecycle::{self, GetOptions, NewOptions};
use gws_core::workspace::mr::{self, MrOptions, ReviewerOverride};
use gws_core::workspace::sync;
use gws_core::workspace::{ModuleFilter, Workspace};
use gws_core::{date, name, Error, Result};

/// 打开 hub 时可能做过目录布局迁移与元数据清理，**要让人知道动了什么**。
/// （上次迁移失败是静默的，差点没发现。）
fn report_migration(hub: &Hub) {
    let m = hub.migration();
    for (from, to) in &m.moved {
        println!("▸ 目录已迁移：{from} → {to}");
    }
    for (branch, module) in &m.pruned_modules {
        println!("▸ 已清掉残留的模块登记：{branch} / {module}（文档不再算模块）");
    }
    for f in &m.failed {
        eprintln!("⚠ 文档目录迁移未完成：{f}");
    }
}

const ABOUT: &str = "GWS—— 多仓库需求开发管理：拉代码、合环境、提 MR、写文档";

#[derive(Parser)]
#[command(name = "gws", version, about = ABOUT)]
struct Cli {
    /// 显式指定 hub 路径（缺省从当前目录向上找 .gws-hub）
    #[arg(long, global = true, value_name = "路径")]
    hub: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 创建新的 hub
    Init(InitArgs),

    /// 仓库管理
    #[command(subcommand)]
    Repo(RepoCmd),

    /// 环境管理（环境标识与分支名解耦）
    #[command(subcommand)]
    Env(EnvCmd),

    /// 新建需求工作区
    New(NewArgs),

    /// 导入他人已推送的需求（自动识别涉及哪些仓库）
    Get(GetArgs),

    /// 列出所有需求
    Ls,

    /// 查看当前需求各模块状态
    St {
        #[command(flatten)]
        filter: FilterArgs,
    },

    /// 往当前需求增加模块
    Add {
        /// 模块（仓库名），可多个
        #[arg(required = true)]
        modules: Vec<String>,
    },

    /// 移除模块；不带参数则清理「目录已被直接删除」的模块
    Drop {
        modules: Vec<String>,
        /// 有未提交改动时也强行移除
        #[arg(long)]
        force: bool,
    },

    /// 删除需求工作区（连同本地分支一并清干净）
    Rm {
        #[arg(required = true)]
        branches: Vec<String>,
        /// 有未保存的工作时也强行删除
        #[arg(long)]
        force: bool,
    },

    /// 撤销工作区里未提交的改动（缺省只预览，加 --yes 执行）
    Revert(FilterArgsWithYes),

    /// 把需求强制回退到远程分支的样子（缺省只预览，加 --yes 执行）
    ResetRemote(FilterArgsWithYes),

    /// 清理已合并的远程需求分支（缺省只预览，加 --yes 执行）
    BranchPrune(FilterArgsWithYes),

    /// 批量修改需求分支名（缺省只预览，加 --yes 执行）
    BranchRename {
        /// 新分支名
        new_branch: String,
        #[command(flatten)]
        filter: FilterArgs,
        /// 真正执行（缺省只预览）
        #[arg(long)]
        yes: bool,
    },

    /// 清理 ws/ 下没有元数据的残留目录
    Clean {
        /// 真正执行删除（缺省只列出）
        #[arg(long)]
        apply: bool,
    },

    /// 推送需求分支到远程
    Push {
        #[command(flatten)]
        filter: FilterArgs,
    },

    /// 拉取他人在同一需求分支上的提交
    Pull {
        #[command(flatten)]
        filter: FilterArgs,
        /// 用 rebase 而不是 merge
        #[arg(long)]
        rebase: bool,
    },

    /// 把需求合并到环境分支
    Merge(MergeArgs),

    /// 把基线的最新提交合进当前需求
    SyncMain(SyncMainArgs),

    /// 同步所有仓库主干与环境（reset 到远程）
    Sync,

    /// 检查需求是否已全部并入主干
    Done {
        #[command(flatten)]
        filter: FilterArgs,
    },

    /// 提 Merge Request 到主干（缺省只预览，加 --create 才真正创建）
    Mr(MrArgs),

    /// GitLab 凭据管理
    #[command(subcommand)]
    Gitlab(GitlabCmd),

    /// 查看/设置这个 hub 的提交身份（不设则用 git 全局配置）
    Identity(IdentityArgs),

    /// 需求文档（文档库）
    #[command(subcommand)]
    Doc(DocCmd),

    /// 查看版本
    Version,

    /// 检查更新（缺省只看，加 --apply 真正更新）
    Update {
        /// 真正下载并替换（缺省只检查）
        #[arg(long)]
        apply: bool,
    },
}

// ══ 参数 ═══════════════════════════════════════════════════════════════

#[derive(Args)]
struct InitArgs {
    /// 建在哪里（缺省用当前目录）
    path: Option<String>,
    /// 初始环境标识，逗号分隔
    #[arg(long, value_delimiter = ',', default_value = "dev")]
    envs: Vec<String>,
    /// hub 名称（缺省用目录名）
    #[arg(long)]
    name: Option<String>,
    /// 目录非空时仍在此初始化
    #[arg(long)]
    force: bool,
    /// 这个 hub 绑定的 GitLab 实例地址（如 https://gitlab.example.com）。
    ///
    /// 地址**落盘后不可改**，所以建 hub 时想清楚。也可以先不填，
    /// 建完用 `gws gitlab bind` 补绑。
    #[arg(long = "gitlab-url")]
    gitlab_url: Option<String>,
    /// GitLab Personal Access Token（需要 api 权限）。
    /// 缺省从标准输入读 —— 写在命令行里会留在 shell 历史里。
    #[arg(long = "gitlab-token")]
    gitlab_token: Option<String>,
    /// 文档库仓库地址（你自己的任何 git 仓库都行，
    /// 如 https://gitlab.example.com/team/docs.git）。
    #[arg(long = "doc")]
    doc: Option<String>,
    /// 提交身份的名字（不填则用 git 全局配置）。
    #[arg(long = "author-name")]
    author_name: Option<String>,
    /// 提交身份的邮箱（不填则用 git 全局配置）。
    #[arg(long = "author-email")]
    author_email: Option<String>,
}

/// 从标准输入读 token。
///
/// **不读命令行参数**：写进 argv 会留在 shell 历史里，也会出现在进程列表里。
fn read_token_interactive() -> Result<String> {
    use std::io::Write as _;
    eprint!("粘贴 GitLab Personal Access Token（需要 api 权限）: ");
    std::io::stderr().flush().ok();
    let mut buf = String::new();
    std::io::stdin()
        .read_line(&mut buf)
        .map_err(|e| Error::other(format!("读取输入失败: {e}")))?;
    let t = buf.trim().to_string();
    if t.is_empty() {
        return Err(Error::other("token 不能为空"));
    }
    Ok(t)
}

#[derive(Subcommand)]
enum RepoCmd {
    /// 添加仓库（clone 到 repos/）
    Add {
        #[arg(required = true)]
        urls: Vec<String>,
    },
    /// 列出仓库
    Ls,
    /// 移除仓库（仍被需求或环境使用时拒绝）
    Rm {
        #[arg(required = true)]
        names: Vec<String>,
    },
}

#[derive(Subcommand)]
enum EnvCmd {
    /// 增加环境
    Add {
        #[arg(required = true)]
        ids: Vec<String>,
        /// 中文显示名
        #[arg(long)]
        label: Option<String>,
        /// 分支名覆盖，格式 `<仓库>=<分支>`；可重复。`*` 表示所有仓库的默认
        #[arg(long = "set", value_name = "仓库=分支")]
        set: Vec<String>,
    },
    /// 列出环境
    Ls,
    /// 移除环境
    Rm {
        #[arg(required = true)]
        ids: Vec<String>,
    },
}

#[derive(Args)]
struct NewArgs {
    /// 分支后缀。分支名拼成 `<前缀>-<日期>-<后缀>`
    name: Option<String>,
    /// 直接用这个完整分支名（同时是目录名），不做拼接
    #[arg(long)]
    branch: Option<String>,
    /// 需求名称（给人看的名字；缺省同分支名）
    #[arg(long, short)]
    title: Option<String>,
    /// 涉及的代码模块，逗号分隔；不写则**只挂文档库**（与界面默认一致，不拉代码）
    #[arg(long, short, value_delimiter = ',')]
    modules: Vec<String>,
    /// 基线来源，逗号分隔，顺序即优先级（`主干` / `ws:<上游分支>` / `<ref>`）
    #[arg(long, short = 'f', value_delimiter = ',')]
    from: Vec<String>,
    /// 分支前缀（缺省取 hub 配置）
    #[arg(long, short)]
    prefix: Option<String>,
    /// 需求分类中文名；不填默认按年月（如 2026-10）
    #[arg(long = "category")]
    category: Option<String>,
}

#[derive(Args)]
struct GetArgs {
    /// 远程分支名
    branch: String,
    /// 需求名称（给人看的名字；缺省同分支名）
    #[arg(long, short)]
    title: Option<String>,
    /// 需求分类中文名（不填就按远程/文档里已有的归类）
    #[arg(long = "category")]
    category: Option<String>,
}

#[derive(Args)]
struct MergeArgs {
    /// 环境标识
    env: String,
    /// 合并后直接推送到远程（缺省只合到本地）
    #[arg(long)]
    push: bool,
    /// 丢弃 envs 本地未推送的合并成果，从远程重新合
    #[arg(long)]
    force: bool,
    #[command(flatten)]
    filter: FilterArgs,
}

#[derive(Args)]
struct SyncMainArgs {
    /// 覆盖基线来源，逗号分隔；缺省取需求里登记的基线
    #[arg(long, short = 'f', value_delimiter = ',')]
    from: Vec<String>,
    #[command(flatten)]
    filter: FilterArgs,
}

#[derive(Args)]
struct MrArgs {
    /// 真正创建（缺省只预览，不写任何东西）
    #[arg(long)]
    create: bool,
    /// 关闭本需求的 MR（只关闭，不碰分支）
    #[arg(long, conflicts_with = "create")]
    close: bool,
    /// MR 标题（缺省用需求中文名）
    #[arg(long, short)]
    title: Option<String>,
    /// MR 描述文件（markdown）
    #[arg(long, value_name = "文件")]
    desc_file: Option<String>,
    /// 所有模块统一指定评审人（username），不再区分前后端
    #[arg(long)]
    reviewer: Option<String>,
    /// 只改前端评审人
    #[arg(long)]
    frontend_reviewer: Option<String>,
    /// 只改后端评审人
    #[arg(long)]
    backend_reviewer: Option<String>,
    #[command(flatten)]
    filter: FilterArgs,
}

#[derive(Subcommand)]
enum GitlabCmd {
    /// 一次性绑定这个 hub 的 GitLab（地址 + token + 文档库地址）。已绑定则拒绝
    Bind {
        /// GitLab 实例地址，如 https://gitlab.example.com（**之后不可改**）
        #[arg(long)]
        url: String,
        /// 文档库仓库地址（**之后不可改**）
        #[arg(long = "doc")]
        doc: String,
        /// Personal Access Token（缺省从标准输入读，不留在 shell 历史里）
        #[arg(long)]
        token: Option<String>,
    },
    /// 改这个 hub 的 token（地址不可改）
    Token {
        /// 直接给出 token（缺省从标准输入读）
        #[arg(long)]
        token: Option<String>,
    },
    /// 查看当前 hub 绑定的 GitLab 与凭据来源，并测试连接
    Status,
    /// 删除这个 hub 保存的 token
    Logout,
}

#[derive(Args)]
struct IdentityArgs {
    /// git 提交用的名字（如「张三」）
    #[arg(long)]
    name: Option<String>,
    /// git 提交用的邮箱
    #[arg(long)]
    email: Option<String>,
    /// 清除配置，回到 git 全局配置
    #[arg(long)]
    clear: bool,
}

// ══ 提交身份 ═══════════════════════════════════════════════════════════

/// 查看/设置这个 hub 的**提交身份**。
///
/// 身份写在 hub 里每个仓库的 local config 上（worktree 与主 clone 共享同一份），
/// 所以用户在 `ws/<分支>/<模块>/` 里**手工 `git commit`** 用的也是这个身份 ——
/// 这正是「跟 git 全局配置解耦」的意义：同一台机器上多套身份不会串。
fn cmd_identity(args: IdentityArgs) -> Result<ExitCode> {
    let mut hub = Hub::open_from(&cwd()?)?;
    let cur = hub.config().git.clone();

    // 什么都不给 = 查看
    if !args.clear && args.name.is_none() && args.email.is_none() {
        if cur.is_set() {
            println!("   名字: {}", cur.author_name);
            println!("   邮箱: {}", cur.author_email);
            println!("   （hub 内已有仓库都写好了 local config，worktree 一并生效）");
        } else {
            println!("   未设置 —— 用 git 全局配置");
        }
        println!();
        println!("   设置: gws identity --name 张三 --email zhangsan@example.com");
        println!("   清除: gws identity --clear");
        return Ok(ExitCode::SUCCESS);
    }

    let next = if args.clear {
        GitIdentity::default()
    } else {
        GitIdentity {
            author_name: args.name.clone().unwrap_or_else(|| cur.author_name.clone()),
            author_email: args.email.clone().unwrap_or_else(|| cur.author_email.clone()),
        }
    };
    if !next.is_set() && !args.clear {
        // 只给一半会写出半截身份，配置校验也会拦 —— 但在这里给人话更好
        return Err(Error::other("「名字」和「邮箱」要一起给（清除用 --clear）"));
    }

    let n = hub.set_identity(next.clone())?;
    if next.is_set() {
        render::ok(format!(
            "提交身份已设为 {} <{}>",
            next.author_name, next.author_email
        ));
    } else {
        render::ok("提交身份已清除，之后用 git 全局配置");
    }
    if n > 0 {
        println!("   已同步到 {n} 个仓库（它们的 worktree 一并生效）");
    }
    Ok(ExitCode::SUCCESS)
}

#[derive(Subcommand)]
enum DocCmd {
    /// 把文档库挂到当前需求（新建需求时已自动挂）
    Attach {
        /// 指定需求分类中文名（缺省按年月归组，如 2026-10）
        #[arg(long = "category")]
        category: Option<String>,
    },
    /// 新建一个带模板的文档
    New {
        /// 文档类型（需求文档 / 技术方案 / 提测单 / 测试用例 / sql / 发布步骤 …）
        kind: String,
    },
    /// 列出本需求的文档
    Ls,
    /// 查看待提交的文档变更
    Status,
    /// 提交文档；加 --push 推到远程文档分支
    Sync {
        #[arg(long, short)]
        message: Option<String>,
        /// 推到远程（缺省只本地提交）
        #[arg(long)]
        push: bool,
    },
    /// 列出所有需求分类与期次（读历史文档用）
    Families,
}

#[derive(Args)]
struct FilterArgsWithYes {
    #[command(flatten)]
    filter: FilterArgs,
    /// 真正执行（缺省只预览）
    #[arg(long)]
    yes: bool,
}

/// 模块选择（设计方案 §6.4）。默认全选。
#[derive(Args, Default, Clone)]
struct FilterArgs {
    /// 只处理这些模块，逗号分隔，可重复
    #[arg(long, value_delimiter = ',', value_name = "模块")]
    only: Vec<String>,
    /// 排除这些模块，逗号分隔，可重复
    #[arg(long, value_delimiter = ',', value_name = "模块")]
    exclude: Vec<String>,
}

impl FilterArgs {
    fn to_filter(&self) -> ModuleFilter {
        ModuleFilter {
            only: if self.only.is_empty() {
                None
            } else {
                Some(self.only.iter().cloned().collect())
            },
            exclude: self.exclude.iter().cloned().collect(),
        }
    }
}

// ══ 入口 ═══════════════════════════════════════════════════════════════

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        // 预检失败在 core 里已格式化成多行（分类列出并给出下一步），别再套前缀
        Err(Error::Precheck(f)) => {
            render::precheck_failure(&f);
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("✗ {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        Command::Init(args) => cmd_init(args),
        Command::Repo(cmd) => cmd_repo(open_hub(&cli.hub)?, cmd),
        Command::Env(cmd) => cmd_env(open_hub(&cli.hub)?, cmd),
        Command::New(args) => cmd_new(open_hub(&cli.hub)?, args),
        Command::Get(args) => cmd_get(open_hub(&cli.hub)?, args),
        Command::Ls => cmd_ls(open_hub(&cli.hub)?),
        Command::Clean { apply } => cmd_clean(open_hub(&cli.hub)?, apply),
        Command::Revert(args) => cmd_revert(open_hub(&cli.hub)?, args),
        Command::ResetRemote(args) => cmd_reset_remote(open_hub(&cli.hub)?, args),
        Command::BranchPrune(args) => cmd_branch_prune(open_hub(&cli.hub)?, args),
        Command::BranchRename {
            new_branch,
            filter,
            yes,
        } => cmd_branch_rename(open_hub(&cli.hub)?, new_branch, filter, yes),
        Command::Update { apply } => cmd_update(apply),
        Command::Version => {
            println!("gws {}  ({ABOUT})", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }

        // ── 以下命令都要在需求工作区内执行 ──
        Command::St { filter } => {
            let (hub, ws) = current()?;
            render::workspace_meta(&ws, hub.root());
            println!();
            render::status_report(&sync::status(&hub, &ws, &filter.to_filter())?);
            Ok(ExitCode::SUCCESS)
        }
        Command::Add { modules } => {
            let (hub, mut ws) = current()?;
            let results = lifecycle::add_modules(&hub, &mut ws, &modules)?;
            render::repo_outcomes(&results);
            Ok(exit_for(&results))
        }
        Command::Drop { modules, force } => {
            let (hub, mut ws) = current()?;
            if modules.is_empty() {
                let out = lifecycle::drop_missing_modules(&hub, &mut ws)?;
                render::drop_missing_outcome(&out);
                return Ok(ExitCode::SUCCESS);
            }
            let mut failed = false;
            for m in &modules {
                match lifecycle::drop_module(&hub, &mut ws, m, force)? {
                    repo::Outcome::Ok => render::ok(format!("{m} 已移出")),
                    repo::Outcome::Skipped(why) => println!("  - {m}  {why}"),
                    repo::Outcome::Failed(why) => {
                        println!("  ✗ {m}  {why}");
                        failed = true;
                    }
                }
            }
            Ok(if failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
        Command::Rm { branches, force } => {
            let hub = open_hub(&cli.hub)?;
            let mut failed = false;
            for b in &branches {
                let out = lifecycle::remove(&hub, b, force)?;
                render::remove_outcome(&out);
                if !out.risks.is_empty() && !force {
                    failed = true;
                }
            }
            Ok(if failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
        Command::Push { filter } => {
            let (hub, ws) = current()?;
            let out = sync::push(&hub, &ws, &filter.to_filter())?;
            render::push_outcome(&out);
            Ok(if out.is_clean() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Command::Pull { filter, rebase } => {
            let (hub, ws) = current()?;
            let out = sync::pull(&hub, &ws, &filter.to_filter(), rebase)?;
            render::pull_outcome(&out);
            Ok(if out.conflicts.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Command::Merge(args) => {
            let (hub, ws) = current()?;
            let out = sync::merge(
                &hub,
                &ws,
                &args.env,
                &args.filter.to_filter(),
                args.push,
                args.force,
            )?;
            render::merge_outcome(&out);
            Ok(if out.has_conflicts() {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
        Command::SyncMain(args) => {
            let (hub, ws) = current()?;
            let from = if args.from.is_empty() {
                None
            } else {
                Some(args.from.as_slice())
            };
            let out = sync::sync_main(&hub, &ws, &args.filter.to_filter(), from)?;
            render::sync_main_outcome(&out, &ws);
            Ok(if out.is_clean() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Command::Sync => {
            let hub = open_hub(&cli.hub)?;
            let out = sync::sync_all(&hub, Some(&gws_core::progress::stderr_reporter))?;
            render::sync_outcome(&out, &hub);
            Ok(
                if out.repo_failures.is_empty() && out.env_failures.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
                },
            )
        }
        Command::Mr(args) => cmd_mr(args),
        Command::Gitlab(cmd) => cmd_gitlab(cmd),
        Command::Identity(args) => cmd_identity(args),
        Command::Doc(cmd) => cmd_doc(cmd),
        Command::Done { filter } => {
            let (hub, ws) = current()?;
            // 每个模块都要 fetch 才能跟主干比，跑几十秒，必须报进度
            let out = sync::done(&hub, &ws, &filter.to_filter(), Some(&stderr_reporter))?;
            render::done_outcome(&out);
            Ok(if out.is_done() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
    }
}

fn exit_for(results: &[repo::RepoOutcome]) -> ExitCode {
    if results.iter().any(|r| r.outcome.is_failed()) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

// ══ hub / 工作区定位 ═══════════════════════════════════════════════════

fn cwd() -> Result<PathBuf> {
    std::env::current_dir().map_err(|e| Error::other(format!("读不到当前目录: {e}")))
}

/// 打开 hub：显式 `--hub` 优先，否则从当前目录向上找标记文件。
fn open_hub(explicit: &Option<String>) -> Result<Hub> {
    match explicit {
        Some(p) => Hub::open(Path::new(p)),
        None => Hub::open_from(&cwd()?),
    }
}

/// 当前需求工作区，以及它所属的 hub。
///
/// 从当前目录向上找 `.workspace.json`（等价于旧版 `gws cur_ws`）——
/// 所以用户可以在工作区内任意子目录执行命令。
fn current() -> Result<(Hub, Workspace)> {
    let ws = Workspace::locate(&cwd()?)?;
    let hub = Hub::open_from(ws.dir())?;
    Ok((hub, ws))
}

// ══ 各命令实现 ═════════════════════════════════════════════════════════

/// 从 init 参数组装 GitLab 绑定；没给地址就是 `None`（之后用 `gitlab bind` 补）。
fn build_binding(args: &InitArgs) -> Result<Option<GitlabBindingInput>> {
    let Some(url) = args.gitlab_url.as_ref().filter(|s| !s.trim().is_empty()) else {
        return Ok(None);
    };
    let token = match args.gitlab_token.as_ref().filter(|s| !s.trim().is_empty()) {
        Some(t) => t.trim().to_string(),
        None => read_token_interactive()?,
    };
    let doc = args.doc.clone().unwrap_or_default();
    if doc.trim().is_empty() {
        return Err(Error::other(
            "给了 --gitlab-url 就要一起给 --doc <文档库地址>：\n  \
             每个需求都要写文档，文档库地址也是建 hub 时固定下来的。",
        ));
    }
    Ok(Some(GitlabBindingInput {
        base_url: url.clone(),
        token,
        dochub_url: doc,
    }))
}

fn cmd_init(args: InitArgs) -> Result<ExitCode> {
    let target = match &args.path {
        Some(p) => PathBuf::from(p),
        None => cwd()?,
    };

    let opts = InitOptions {
        name: args.name.clone(),
        envs: args
            .envs
            .iter()
            .map(|e| EnvDef::new(e, label_of(e)))
            .collect(),
        allow_non_empty: args.force,
        gitlab: build_binding(&args)?,
        // 提交身份：与桌面端一致，建 hub 时就能定下来（之后 `gws identity` 可改）
        identity: GitIdentity {
            author_name: args.author_name.clone().unwrap_or_default().trim().to_string(),
            author_email: args.author_email.clone().unwrap_or_default().trim().to_string(),
        },
    };
    let hub = Hub::init(&target, opts)?;

    render::ok(format!("hub 已创建: {}", hub.root().display()));
    println!("   环境: {}", args.envs.join(" "));
    if hub.config().gitlab.is_bound() {
        println!("   GitLab: {}", hub.config().gitlab.base_url);
        println!("   文档库: {}", hub.config().dochub.url);
    } else {
        println!("   GitLab: 未绑定（用 gws gitlab bind 补上，否则加仓库/提 MR 都用不了）");
    }
    let id = &hub.config().git;
    if id.is_set() {
        println!("   提交身份: {} <{}>", id.author_name, id.author_email);
    } else {
        println!("   提交身份: 未设置（用 git 全局配置；随时可改: gws identity）");
    }
    println!();
    println!("   下一步");
    println!("     1. 添加仓库:  gws repo add <git地址>");
    println!("     2. 建需求:    gws new <需求名称>");

    Ok(ExitCode::SUCCESS)
}

fn label_of(env_id: &str) -> String {
    match env_id {
        "dev" => "开发",
        "pre" => "预发",
        "prod" => "生产",
        "test" => "测试",
        other => other,
    }
    .to_string()
}

fn cmd_repo(hub: Hub, cmd: RepoCmd) -> Result<ExitCode> {
    match cmd {
        RepoCmd::Add { urls } => {
            let results = repo::add(&hub, &urls, Some(&stderr_reporter))?;
            render::repo_outcomes(&results);
            if results.iter().any(|r| r.outcome.is_ok()) {
                println!();
                println!("  补建环境 worktree: gws sync");
            }
            Ok(exit_for(&results))
        }
        RepoCmd::Ls => {
            report_migration(&hub);
            let mut rows = Vec::new();
            for r in repo::list(&hub)? {
                let main = r.main_branch().unwrap_or_else(|_| "?".into());
                rows.push((r.name, main));
            }
            render::repo_table(&rows);
            Ok(ExitCode::SUCCESS)
        }
        RepoCmd::Rm { names } => {
            let results = repo::remove(&hub, &names)?;
            render::repo_outcomes(&results);
            Ok(exit_for(&results))
        }
    }
}

fn cmd_env(hub: Hub, cmd: EnvCmd) -> Result<ExitCode> {
    match cmd {
        EnvCmd::Add { ids, label, set } => cmd_env_add(hub, ids, label, set),
        EnvCmd::Ls => {
            render::env_table(&hub.config().envs);
            Ok(ExitCode::SUCCESS)
        }
        EnvCmd::Rm { ids } => {
            let mut hub = hub;
            let mut failed = false;
            for id in &ids {
                if hub.config().env(id).is_none() {
                    println!("  - {id}  不存在");
                    continue;
                }
                // 目录里有 worktree：先摘掉再删目录
                let dir = hub.env_dir(id);
                if dir.exists() {
                    for r in repo::list(&hub)? {
                        let wt = dir.join(&r.name);
                        if wt.exists() {
                            let _ = r.git().worktree_remove(&wt, true);
                        }
                        let _ = r.git().worktree_prune();
                    }
                    if let Err(e) = lifecycle::remove_dir_verified(&dir) {
                        println!("  ✗ {id}  目录删除失败: {e}");
                        failed = true;
                        continue;
                    }
                }
                hub.config_mut().envs.retain(|e| &e.id != id);
                if let Err(e) = hub.save_config() {
                    println!("  ✗ {id}  配置保存失败: {e}");
                    failed = true;
                    continue;
                }
                render::ok(format!("{id} 已移除"));
            }
            Ok(if failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
    }
}

fn cmd_env_add(
    mut hub: Hub,
    ids: Vec<String>,
    label: Option<String>,
    set: Vec<String>,
) -> Result<ExitCode> {
    // 解析 --set 仓库=分支
    let mut overrides: Vec<(String, String)> = Vec::new();
    for item in &set {
        match item.split_once('=') {
            Some((r, b)) if !r.trim().is_empty() && !b.trim().is_empty() => {
                overrides.push((r.trim().to_string(), b.trim().to_string()));
            }
            _ => {
                return Err(Error::other(format!(
                    "--set 的格式应为 <仓库>=<分支>，收到: {item}"
                )))
            }
        }
    }

    let mut failed = false;
    for id in &ids {
        if hub.config().env(id).is_some() {
            println!("  - {id}  已存在");
            continue;
        }
        let mut env = EnvDef::new(id, label.clone().unwrap_or_else(|| label_of(id)));
        for (r, b) in &overrides {
            env.branches.insert(r.clone(), b.clone());
        }
        if let Err(e) = env.validate() {
            println!("  ✗ {id}  {e}");
            failed = true;
            continue;
        }

        let dir = hub.env_dir(id);
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(dir.clone(), e))?;

        hub.config_mut().envs.push(env);
        if let Err(e) = hub.save_config() {
            println!("  ✗ {id}  配置保存失败: {e}");
            failed = true;
            continue;
        }
        render::ok(format!("{id} 已创建"));
        println!("     目录: {}", dir.display());
    }

    if !failed {
        println!();
        println!("  补建各模块的环境 worktree: gws sync");
    }
    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn cmd_new(hub: Hub, args: NewArgs) -> Result<ExitCode> {
    let branch = match (&args.branch, &args.name) {
        (Some(b), _) => b.clone(),
        (None, Some(name)) => {
            let prefix = args
                .prefix
                .clone()
                .unwrap_or_else(|| hub.config().defaults.branch_prefix.clone());
            let digits = hub.config().defaults.branch_date_digits;
            name::compose_branch(&prefix, &date::today_with_digits(digits), name)
        }
        (None, None) => {
            return Err(Error::other(
                "请给出需求名称，或用 --branch 直接指定完整分支名\n   \
                 例: gws new 收银台改版       → feature-20261010-收银台改版\n   \
                 例: gws new --branch feature-20261010-checkout",
            ))
        }
    };

    // **与「新建需求」界面的默认一致**：不指定模块 = 只挂文档库，
    // 一个代码仓库都不拉。旧版默认全部仓库，在 22 个仓库的 hub 上一次就是
    // 几十 GB 的 worktree —— 只是要写个需求文档时完全没必要（实测反馈）。
    let modules = default_new_modules(&args.modules, &hub.dochub_name());
    if args.modules.is_empty() {
        render::info("未指定代码模块：默认只挂文档库（需要代码时用 gws add <模块名>）");
        }
    let doc_family = args
        .category
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_string);

    let opts = NewOptions {
        // CLI 暂不支持建的时候填备注/自定义信息（桌面端可以）；
        // 建完用 `gws meta` 一类的入口补，或直接改 .workspace.json
        notes: String::new(),
        links: Vec::new(),
        branch: branch.clone(),
        title: args.title.clone().unwrap_or_else(|| branch.clone()),
        prefix: args
            .prefix
            .clone()
            .unwrap_or_else(|| hub.config().defaults.branch_prefix.clone()),
        base: if args.from.is_empty() {
            vec!["主干".to_string()]
        } else {
            args.from.clone()
        },
        modules,
        doc_family: doc_family.clone(),
    };

    let out = lifecycle::new(&hub, &opts)?;
    render::new_outcome(&out);
    if out.created.is_empty() {
        render::info("这个需求还没有代码模块；需要时: gws add <模块名>");
    }

    // 自动挂文档（需求 #2：每个新建需求都自动拉取同步文档目录）。
    // **失败不阻断**：代码工作区已经建好了，文档挂不上不该让它白建——
    // 离线、文档库不可达时都属这种情况，用户之后可以 gws doc attach 补上。
    // 新建用 Create（本地还没有这一期时生成期次目录），导入用 ReuseOnly。
    attach_docs_quietly(&hub, &out.branch, doc_family.as_deref(), AttachMode::Create);
    Ok(ExitCode::SUCCESS)
}

/// 不指定模块时的默认值：**只有文档库**，与界面「新建需求」的默认勾选一致。
fn default_new_modules(requested: &[String], dochub_name: &str) -> Vec<String> {
    if requested.is_empty() {
        vec![dochub_name.to_string()]
    } else {
        requested.to_vec()
    }
}

/// 尽力挂上文档目录，失败只告警。
///
/// `mode` 与界面一致：`new` 用 **Create**（本地还没有这一期时生成期次目录，
/// 否则"只挂文档"等于什么都没挂）；**ReuseOnly** 则只复用已有文档、绝不凭空写
/// meta.json / CLAUDE.md / AGENTS.md（那几个未跟踪文件会挡住之后的 `git pull`）。
fn attach_docs_quietly(hub: &Hub, branch: &str, family: Option<&str>, mode: AttachMode) {
    if !hub.config().dochub.enabled {
        return;
    }
    match DocHub::ensure_with(hub, None).and_then(|dh| {
        let mut ws = Workspace::open(hub, branch)?;
        dh.attach_with(&mut ws, family, mode)
    }) {
        Ok(a) => {
            println!();
            render::doc_attach(&a);
            if a.attached {
                println!(
                    "   写文档: {}/   （目录里的 CLAUDE.md 说明了文档规范）",
                    a.relative_dir
                );
            }
        }
        Err(e) => {
            render::warn(format!("文档目录未挂上（不影响代码开发）: {e}"));
            println!("   稍后可重试: gws doc attach");
        }
    }
}

fn cmd_get(hub: Hub, args: GetArgs) -> Result<ExitCode> {
    render::info(format!("扫描各仓库是否存在 origin/{}", args.branch));
    let opts = GetOptions {
        notes: String::new(),
        links: Vec::new(),
        branch: args.branch.clone(),
        title: args.title.clone().unwrap_or_else(|| args.branch.clone()),
        doc_family: args.category.clone(),
        base: Vec::new(),
        modules: None,
    };
    let out = lifecycle::get(&hub, &opts, Some(&gws_core::progress::stderr_reporter))?;

    render::ok(format!("已拉取需求: {}", out.branch));
    println!("   模块: {} 个", out.modules.len());
    for m in &out.modules {
        if out.reused.contains(m) {
            println!("     ✓ {m}  (复用本地已有分支)");
        } else {
            println!("     ✓ {m}");
        }
    }
    if !out.not_involved.is_empty() {
        println!(
            "   未参与本需求的仓库: {} 个（正常）",
            out.not_involved.len()
        );
    }
    println!();
    println!("   各模块的上游已指向 origin/{}", out.branch);
    println!("   开工前同步其他人最新代码: gws pull");
    println!("   下一步: cd ws/{}", out.branch);
    Ok(ExitCode::SUCCESS)
}

fn cmd_ls(hub: Hub) -> Result<ExitCode> {
    let list = gws_core::workspace::list(&hub)?;
    render::workspace_table(&list);
    Ok(ExitCode::SUCCESS)
}

fn cmd_clean(hub: Hub, apply: bool) -> Result<ExitCode> {
    let out = lifecycle::clean(&hub, apply)?;
    let failed = !out.failed.is_empty();
    render::clean_outcome(&out, apply);
    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}
// ══ MR ════════════════════════════════════════════════════════════════

fn cmd_mr(args: MrArgs) -> Result<ExitCode> {
    let (hub, mut ws) = current()?;
    let client = Client::for_hub(&hub)?;
    let filter = args.filter.to_filter();

    if args.close {
        let out = mr::close(&hub, &mut ws, &client, &filter)?;
        render::mr_close(&out);
        return Ok(if out.failed.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        });
    }

    let description = match &args.desc_file {
        Some(path) => Some(
            std::fs::read_to_string(path)
                .map_err(|e| Error::other(format!("读取描述文件失败 {path}: {e}")))?,
        ),
        None => None,
    };

    let opts = MrOptions {
        title: args.title.clone(),
        description,
        filter,
        reviewers: ReviewerOverride {
            all: args.reviewer.clone(),
            frontend: args.frontend_reviewer.clone(),
            backend: args.backend_reviewer.clone(),
        },
    };

    let plan = mr::run(
        &hub,
        &mut ws,
        &client,
        &opts,
        args.create,
        Some(&gws_core::progress::stderr_reporter),
    )?;
    let has_failure = plan.failed().next().is_some();
    render::mr_plan(&plan);

    if args.create {
        println!();
        println!("{}", "=".repeat(66));
        println!("可复制给评审人的清单：");
        println!();
        // 文档链接由文档库那一阶段补上，这里先留空
        print!("{}", render::mr_manifest(&plan, &[]));
    }

    Ok(if has_failure {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

// ══ GitLab 凭据 ═══════════════════════════════════════════════════════

fn cmd_gitlab(cmd: GitlabCmd) -> Result<ExitCode> {
    match cmd {
        GitlabCmd::Bind { url, doc, token } => {
            let token = match token {
                Some(t) => t.trim().to_string(),
                None => read_token_interactive()?,
            };
            // 从当前目录向上找 hub —— 和别的命令一样，在 hub 里任意子目录都能跑
            let mut hub = Hub::open_from(&cwd()?)?;
            hub.bind_gitlab(&GitlabBindingInput {
                base_url: url,
                token,
                dochub_url: doc,
            })?;
            render::ok(format!("已绑定 GitLab: {}", hub.config().gitlab.base_url));
            println!("   文档库: {}", hub.config().dochub.url);
            println!("   地址与文档库地址**之后不可修改**；换 token 用 gws gitlab token");
            Ok(ExitCode::SUCCESS)
        }

        GitlabCmd::Token { token } => {
            let token = match token {
                Some(t) => t.trim().to_string(),
                None => read_token_interactive()?,
            };
            let hub = Hub::open_from(&cwd()?)?;
            // 先确认地址已绑定：没绑就写 token 是没意义的，不如早点说清
            hub.gitlab_base_url()?;
            gitlab::token::store(hub.root(), &token)?;
            render::ok(format!("token 已保存到 {}", hub.token_file().display()));

            // 立刻验证：存进去但用不了的话，现在就该让人知道
            match Client::for_hub(&hub)?.current_user() {
                Ok(u) => render::ok(format!("连接成功: {} ({})", u.name, u.username)),
                Err(e) => {
                    eprintln!("✗ token 已保存，但连接失败: {e}");
                    return Ok(ExitCode::FAILURE);
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        GitlabCmd::Status => {
            let hub = Hub::open_from(&cwd()?)?;
            let stored = gitlab::token::has_stored(hub.root());
            // 逐层保留真实原因：解析凭据失败和连接失败要给不同的提示
            let connected = Client::for_hub(&hub).map_err(|e| e.to_string()).and_then(|c| {
                    c.current_user()
                        .map(|u| (u.name, u.username))
                        .map_err(|e| e.to_string())
                });
            let ok = connected.is_ok();
            render::gitlab_status(&hub, connected, stored);
            Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }

        GitlabCmd::Logout => {
            let hub = Hub::open_from(&cwd()?)?;
            gitlab::token::clear(hub.root())?;
            render::ok("已删除这个 hub 保存的 token");
            if ["GWS_GITLAB_TOKEN", "GITLAB_TOKEN"].iter().any(|k| {
                std::env::var(k)
                    .map(|v| !v.trim().is_empty())
                    .unwrap_or(false)
            }) {
                render::warn("注意：环境变量里仍有 token，它会继续优先生效");
                println!("   清除: unset GITLAB_TOKEN（并删掉 ~/.zshrc 里那一行）");
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

// ══ 需求文档 ══════════════════════════════════════════════════════════

fn cmd_doc(cmd: DocCmd) -> Result<ExitCode> {
    let (hub, mut ws) = current()?;
    let dh = DocHub::ensure_with(&hub, Some(&stderr_reporter))?;

    match cmd {
        DocCmd::Attach { category } => {
            let out = dh.attach(&mut ws, category.as_deref())?;
            render::doc_attach(&out);
            // 目录名**取自 hub 配置**（用户填的地址推导），不写死 doc-hub
            println!(
                "   目录: ws/{}/{}/{}",
                ws.branch(),
                hub.dochub_name(),
                out.relative_dir
            );
            Ok(ExitCode::SUCCESS)
        }

        DocCmd::New { kind } => {
            let path = dh.new_doc(&ws, &kind)?;
            render::ok(format!("已创建 {}", path.display()));
            Ok(ExitCode::SUCCESS)
        }

        DocCmd::Ls => {
            let period = dh.period_dir(&ws);
            let meta = PeriodMeta::load(&period.join("meta.json"))?;
            println!("{}   期次 {}   分支 {}", meta.name, meta.label, meta.branch);
            println!();
            let mut files = Vec::new();
            collect_files(&period, &period, &mut files)?;
            files.sort();
            render::doc_files(&period, &files);
            Ok(ExitCode::SUCCESS)
        }

        DocCmd::Status => {
            render::doc_status(&dh.status(&ws)?);
            Ok(ExitCode::SUCCESS)
        }

        DocCmd::Sync { message, push } => {
            let out = dh.sync(&ws, message.as_deref().unwrap_or(""), push)?;
            render::doc_sync(&out);
            Ok(ExitCode::SUCCESS)
        }

        DocCmd::Families => {
            render::doc_families(&dh.families()?);
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// 递归列出期次目录下的文件（相对期次目录的路径）。
fn collect_files(
    root: &std::path::Path,
    dir: &std::path::Path,
    out: &mut Vec<String>,
) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.display().to_string());
        }
    }
    Ok(())
}

// ══ 危险操作 ══════════════════════════════════════════════════════════
//
// 统一两段式：不给 --yes 就只预览。CLI 上做不到「手输分支名确认」，
// 所以把「预览 → 加 --yes 重跑」当作确认动作——多跑一次本身就是一次停顿。

fn cmd_revert(hub: Hub, args: FilterArgsWithYes) -> Result<ExitCode> {
    let (_, ws) = current()?;
    let f = args.filter.to_filter();
    if !args.yes {
        for l in gws_core::workspace::revert::revert_lines(
            &gws_core::workspace::revert::revert_plan(&hub, &ws, &f)?,
        ) {
            println!("{l}");
        }
        println!();
        println!("  确认撤销请加 --yes");
        return Ok(ExitCode::SUCCESS);
    }
    let out = gws_core::workspace::revert::revert_apply(&hub, &ws, &f)?;
    for (m, n) in &out.reverted {
        println!("✓ {m}  已撤销 {n} 处改动");
    }
    for (m, why) in &out.failed {
        println!("✗ {m}  {why}");
    }
    Ok(if out.failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn cmd_reset_remote(hub: Hub, args: FilterArgsWithYes) -> Result<ExitCode> {
    let (_, ws) = current()?;
    let f = args.filter.to_filter();
    use gws_core::workspace::revert as rv;
    if !args.yes {
        for l in rv::reset_lines(&rv::reset_plan(&hub, &ws, &f)?) {
            println!("{l}");
        }
        println!();
        println!("  确认回退请加 --yes");
        return Ok(ExitCode::SUCCESS);
    }
    let out = rv::reset_apply(&hub, &ws, &f)?;
    for (m, t) in &out.reset {
        println!("✓ {m}  → {t}");
    }
    for (m, why) in &out.skipped {
        println!("- {m}  {why}");
    }
    for (m, why) in &out.failed {
        println!("✗ {m}  {why}");
    }
    Ok(if out.failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn cmd_branch_prune(hub: Hub, args: FilterArgsWithYes) -> Result<ExitCode> {
    let (_, ws) = current()?;
    let client = Client::for_hub(&hub)?;
    let f = args.filter.to_filter();
    use gws_core::workspace::branch;
    if !args.yes {
        for l in branch::prune_lines(&branch::prune_plan(&hub, &ws, &f, &client)?) {
            println!("{l}");
        }
        println!();
        println!("  确认删除请加 --yes（只会删「已并入主干且本地无未推送提交」的）");
        return Ok(ExitCode::SUCCESS);
    }
    let out = branch::prune_apply(&hub, &ws, &f, &client)?;
    for (m, b) in &out.deleted {
        println!("✓ {m}  已删除远程分支 {b}");
    }
    for (m, why) in &out.skipped {
        println!("- {m}  {why}");
    }
    for (m, why) in &out.failed {
        println!("✗ {m}  {why}");
    }
    Ok(if out.failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn cmd_branch_rename(
    hub: Hub,
    new_branch: String,
    filter: FilterArgs,
    yes: bool,
) -> Result<ExitCode> {
    let (_, mut ws) = current()?;
    // 没有令牌也允许预览/执行「完全未推送」的改名；core 会拦住已推送的情况
    let client = Client::for_hub(&hub).ok();
    let f = filter.to_filter();
    use gws_core::workspace::branch;
    let plan = branch::rename_plan(&hub, &ws, &f, &new_branch, client.as_ref())?;
    for l in branch::rename_lines(&plan) {
        println!("{l}");
    }
    if !yes {
        println!();
        println!("  确认改名请加 --yes");
        return Ok(ExitCode::SUCCESS);
    }
    let out = branch::rename_apply(&hub, &mut ws, &f, &new_branch, client.as_ref())?;
    println!();
    for m in &out.renamed {
        println!("✓ {m}  本地分支已改名");
    }
    for m in &out.pushed {
        println!("✓ {m}  已推送新分支");
    }
    for m in &out.deleted_remote {
        println!("✓ {m}  已删除远程旧分支");
    }
    for (m, why) in &out.failed {
        println!("✗ {m}  {why}");
    }
    Ok(if out.failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

// ══ 自更新 ════════════════════════════════════════════════════════════

/// 检查/执行更新。走 GitHub Releases（本软件自己的发布页），与 hub 无关。
fn cmd_update(apply: bool) -> Result<ExitCode> {
    let current = env!("CARGO_PKG_VERSION");
    // **不需要 hub，也不需要 token**：Release 是公开的，不该因为「没配 token」
    // 或「不在 hub 目录里」就看不到新版本。
    let check = gws_core::update::check(current)?;

    match &check.status {
        gws_core::update::UpdateStatus::UpToDate => {
            println!("✓ 已是最新版本 {current}");
            return Ok(ExitCode::SUCCESS);
        }
        gws_core::update::UpdateStatus::Ahead(rel) => {
            println!("远程版本 {} 低于当前 {current}，不降级", rel.version);
            println!("  若确实要换回旧版，请手工替换二进制");
            return Ok(ExitCode::SUCCESS);
        }
        gws_core::update::UpdateStatus::Available(rel) => {
            println!("有新版本可用: {current} → {}", rel.version);
            println!("  资产: {}", gws_core::update::asset_name());
            for a in &rel.assets {
                println!("    - {}", a.name);
            }
            if !rel.web_url.is_empty() {
                println!("  发布页: {}", rel.web_url);
            }
        }
    }

    if !apply {
        println!();
        println!("  执行更新: gws update --apply");
        return Ok(ExitCode::SUCCESS);
    }

    println!();
    let path = gws_core::update::apply(current, false, Some(&stderr_reporter))?;
    println!();
    println!("✓ 已更新: {}", path.display());
    println!("  重新运行 gws 即为新版本");
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults_to_docs_only_like_the_gui() {
        // 界面上「新建需求」默认只勾文档库；CLI 不传 --modules 必须是同一个含义。
        assert_eq!(
            default_new_modules(&[], "doc-hub"),
            vec!["doc-hub".to_string()]
        );
        // 明确点名就原样用 —— 包括「只要代码、不要文档」
        let asked = vec!["demo-payment".to_string(), "demo-api".to_string()];
        assert_eq!(default_new_modules(&asked, "doc-hub"), asked);
    }
}
