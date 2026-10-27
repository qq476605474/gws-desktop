//! 仓库管理。
//!
//! `repos/` 是 hub 里**唯一的 `.git` 存放处**——所有需求工作区与环境目录都是它的
//! worktree，共享同一份对象库（磁盘开销远低于重复克隆）。
//!
//! 因此 `repos/<仓库名>/` 是「生产干净代码」的镜像：`gws sync` 会把它 reset 到
//! `origin/<主干>`。**不要在这里改代码**（详见 hub 根 `CLAUDE.md` 的目录语义地图）。

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::git::{self, Git};
use crate::hub::Hub;
use crate::name;
use crate::{Error, Result};


/// 一个仓库（`repos/<名字>/`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub name: String,
    pub path: PathBuf,
}

impl Repo {
    /// 打开指定名字的仓库。
    pub fn open(hub: &Hub, name: &str) -> Result<Self> {
        name::validate_component(name, "仓库名")?;
        let path = hub.repos_dir().join(name);
        if !path.join(".git").exists() {
            return Err(Error::RepoNotFound(name.to_string()));
        }
        Ok(Self { name: name.to_string(), path })
    }

    pub fn git(&self) -> Git {
        Git::new(&self.path)
    }

    /// 主干分支名（各仓库 `main`/`master` 混用，逐个探测）。
    pub fn main_branch(&self) -> Result<String> {
        git::detect_main_branch(&self.path)
    }

    pub fn remote_url(&self) -> Option<String> {
        self.git().remote_url()
    }
}

/// 单个操作的结果。部分成功是常态（多仓库批量操作），所以逐个回报而不是
/// 一失败就整体中断。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Outcome {
    Ok,
    /// 未执行，但不是错误（已存在 / 不在使用中等）。
    Skipped(String),
    Failed(String),
}

impl Outcome {
    pub fn is_ok(&self) -> bool {
        matches!(self, Outcome::Ok)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Outcome::Failed(_))
    }
}

/// 一次「名字 + 结果」的记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepoOutcome {
    pub name: String,
    pub outcome: Outcome,
}

/// 列出 hub 下所有仓库（按名字排序）。
///
/// 只返回**含 `.git` 的目录**——半个残留目录不算可用仓库。
pub fn list(hub: &Hub) -> Result<Vec<Repo>> {
    let dir = hub.repos_dir();
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut repos = Vec::new();
    let entries = std::fs::read_dir(&dir).map_err(|e| Error::io(dir.clone(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(dir.clone(), e))?;
        if !entry.path().join(".git").exists() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue; // 非 UTF-8 文件名，跳过而非让整个列表失败
        };
        repos.push(Repo { name, path: entry.path() });
    }
    repos.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(repos)
}

/// 从 git 地址推断仓库名：去掉 `.git` 后缀取最后一段。
///
/// 支持 `https://host/group/repo.git` 与 `git@host:group/repo.git` 两种形态。
pub fn name_from_url(url: &str) -> Option<String> {
    // 分隔符要连**反斜杠**一起认：Windows 上本地镜像路径是
    // `C:\Users\me\mirror\demo-api.git`，只按 `/` 与 `:` 切的话最后一段是
    // `\Users\me\mirror\demo-api`（整条路径），会被当成仓库名 → 「名称非法」。
    // 这个错只在 Windows 上现形（CI 的 Windows job 抓到的）。
    let trimmed = url.trim().trim_end_matches(['/', '\\']);
    let last = trimmed.rsplit(['/', ':', '\\']).next()?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// 批量添加仓库（clone 到 `repos/`）。
///
/// 已存在的跳过，单个失败不影响其余。**失败的目录会被清掉**，不留半个 clone
/// ——否则它会以「含 `.git` 的空仓库」形态混进 [`list`]。
///
/// `progress` 用于逐个上报（克隆 22 个仓库要几十秒，界面需要看到进行到哪了）。
pub fn add(
    hub: &Hub,
    urls: &[String],
    progress: crate::progress::Progress<'_>,
) -> Result<Vec<RepoOutcome>> {
    let repos_dir = hub.repos_dir();
    std::fs::create_dir_all(&repos_dir).map_err(|e| Error::io(repos_dir.clone(), e))?;

    // 允许的主机在循环外解析一次：hub 还没绑定 GitLab 时直接整体报错，
    // 而不是逐条刷一屏「Failed」（那会让人以为是地址写错了）。
    let allowed_host = hub.gitlab_host()?;

    let mut rep = crate::progress::Reporter::start(progress, urls.len(), "拉取仓库");
    let mut results = Vec::with_capacity(urls.len());
    for url in urls {
        // **只接受本 hub 绑定的 GitLab 上的仓库**（本地路径放行，见 `check_remote`）：
        // 混进来别的内容之后分支约定、MR、文档库这些假设全都不成立。
        // 逐条报 Failed 而不是整体 `?` 掉 —— 一次填多个地址时，不该因为其中一个
        // 不合格就让整批都不知所云。
        if let Err(e) = crate::gitlab::check_remote(url, &allowed_host) {
            rep.fail(url.clone(), e.to_string());
            results.push(RepoOutcome {
                name: url.clone(),
                outcome: Outcome::Failed(e.to_string()),
            });
            continue;
        }
        let Some(name) = name_from_url(url) else {
            rep.fail(url.clone(), "无法从地址解析仓库名");
            results.push(RepoOutcome {
                name: url.clone(),
                outcome: Outcome::Failed(format!("无法从地址解析仓库名: {url}")),
            });
            continue;
        };

        if let Err(e) = name::validate_component(&name, "仓库名") {
            rep.fail(name.clone(), e.to_string());
            results.push(RepoOutcome { name, outcome: Outcome::Failed(e.to_string()) });
            continue;
        }

        let dest = repos_dir.join(&name);
        if dest.join(".git").exists() {
            if repo_usable(&dest) {
                rep.ok(format!("{name}（已存在，跳过）"));
                results.push(RepoOutcome {
                    name,
                    outcome: Outcome::Skipped("已存在".to_string()),
                });
                continue;
            }
            // **目录在、但不是个能用的仓库** —— 半截克隆：上次克隆被打断/被杀，
            // 只留下一个没有 HEAD 的 `.git`。
            //
            // 这种必须**删掉重来**：当成"已存在"跳过的话，后面 `git worktree add`
            // 会以「fatal: Could not reset index file to revision 'HEAD'」这种
            // 完全看不出原因的方式失败（实测反馈，其他人那边卡的就是这条）。
            //
            // 删掉是安全的：`repos/` 是**镜像**，工作区在 `ws/`（worktree 目录不在
            // 这下面）；而主 clone 坏掉时那些 worktree 本来也已经用不了 ——
            // 重新克隆之后打开 hub，`hub::repair` 会把它们重新接上。
            rep.ok(format!("{name}（本地副本不完整，已重新拉取）"));
            let _ = std::fs::remove_dir_all(&dest);
        }

        // **带 token 克隆**：私有仓库不允许匿名拉，而 `git clone` 在没有人
        // 输密码的地方（我们这个 GUI/CLI）只会得到一句
        // `could not read Username for 'https://…'`（实测：Windows 上添加私有仓库）。
        // 应用里本来就存着 token（推送/合并那条路在用），这里用同一个来源。
        //
        // 带 token **失败再退回匿名**：token 过期、或没有该仓库权限时，
        // 公开仓库不带凭据反而拉得下来 —— 不能因为一个坏 token 把公开仓库也拖下水。
        let token = crate::gitlab::token::resolve(hub.root()).ok();
        let mut cloned = git::clone_with_token(url, &dest, token.as_deref());
        if cloned.is_err() && token.is_some() {
            let _ = std::fs::remove_dir_all(&dest);
            cloned = git::clone_with_token(url, &dest, None);
        }
        match cloned {
            Ok(()) => {
                // 提交身份跟着 hub 走：clone 完就写进这个仓库的 local config，
                // 之后它的所有 worktree 都继承（用户在 ws/ 里手工提交也用对身份）。
                // 失败只记一条日志，不算 clone 失败 —— 仓库本身是好的。
                if let Err(e) = crate::git::apply_identity(&dest, &hub.config().git) {
                    eprintln!("⚠ 写入提交身份失败（{name}）：{e}");
                }
                rep.ok(name.clone());
                results.push(RepoOutcome { name, outcome: Outcome::Ok });
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dest);
                rep.fail(name.clone(), first_line(&e.to_string()));
                results.push(RepoOutcome { name, outcome: Outcome::Failed(e.to_string()) });
            }
        }
    }
    rep.finish();
    Ok(results)
}

/// 本地这份仓库副本**能不能用**（`HEAD` 解得出来）。
///
/// 判据取 `rev-parse --verify HEAD`：半截克隆留下的 `.git` 里没有 HEAD，
/// 这个命令会失败；正常克隆一定成功。**不要用「目录在不在」当判据** ——
/// 那正是「半截克隆被当成已存在」那个坑。
fn repo_usable(dir: &Path) -> bool {
    git::Git::new(dir).out(&["rev-parse", "--verify", "HEAD"]).is_ok()
}

/// 取错误信息的首行——进度行是一行一条，多行会把界面撑乱。
fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

/// 批量移除仓库。
///
/// **仍被需求工作区或环境使用的仓库拒删**——删掉会让那些 worktree 的元数据
/// 指向不存在的对象库，且 `gws sync` 会尝试重建它们。旧版同款守卫。
pub fn remove(hub: &Hub, names: &[String]) -> Result<Vec<RepoOutcome>> {
    let mut results = Vec::with_capacity(names.len());
    for name in names {
        results.push(RepoOutcome {
            name: name.clone(),
            outcome: remove_one(hub, name),
        });
    }
    Ok(results)
}

fn remove_one(hub: &Hub, name: &str) -> Outcome {
    // 文档库是 hub 的固定部件，不是"某个仓库"——删掉它所有需求的文档都没了。
    // 它在 repos/ 下、跟其它仓库一样被列出来，但**不可移除**（实测反馈 #9）。
    if hub.is_dochub(name) {
        return Outcome::Failed("文档库不可移除（所有需求的文档都放在它里面）".to_string());
    }
    let path = hub.repos_dir().join(name);
    if !path.join(".git").exists() {
        return Outcome::Skipped("不存在".to_string());
    }

    // **只拦「被需求引用」**。环境里的检出是镜像，跟着一起删（见 `env_usages`）。
    let used_in = workspace_usages(hub, name);
    if !used_in.is_empty() {
        return Outcome::Failed(format!(
            "仍被这些需求使用: {}（先删掉对应需求，或从需求里移除这个模块）",
            used_in.join("、")
        ));
    }

    // 环境里的检出跟着删：先删目录，再清掉环境的分支覆盖配置。
    // 失败只记一条不阻断 —— 目录没删干净也不会留下坏数据（下次 sync 会重建）。
    for env in env_usages(hub, name) {
        let env_id = env.trim_start_matches("envs/");
        let dir = hub.env_dir(env_id).join(name);
        if dir.exists() {
            let _ = crate::workspace::lifecycle::remove_dir_verified(&dir);
        }
    }
    // 环境的分支覆盖表里也把它摘掉 —— 否则配置里留着一条指向不存在仓库的规则，
    // 界面上「分支覆盖」那一栏会一直显示它。
    // 只改 `envs[].branches`，不碰地址，所以直接落盘是安全的。
    //
    // ⚠️ 这里**不能吞掉 save 的错误**：仓库目录已经删了、配置却没写下去时，
    // 配置里会留下一条指向不存在仓库的规则（界面上的「分支覆盖」照旧显示它），
    // 而函数还报「已移除」—— 正是本项目最忌讳的「失败还报成功」。
    // 配置没写成不改判定为「移除失败」，但要说清：目录已经删了，只有配置需要重来。
    let mut config_stale: Option<String> = None;
    {
        let mut cfg = hub.config().clone();
        let mut changed = false;
        for env in &mut cfg.envs {
            if env.branches.remove(name).is_some() {
                changed = true;
            }
        }
        if changed {
            if let Err(e) = cfg.save(&hub.root().join(crate::hub::CONFIG_FILE)) {
                config_stale = Some(e.to_string());
            }
        }
    }

    // 先清掉 worktree 元数据，避免删掉目录后 repos/ 里留下悬空引用
    let git = Git::new(&path);
    let _ = git.status(&["worktree", "prune"]);

    if let Err(e) = std::fs::remove_dir_all(&path) {
        return Outcome::Failed(format!("删除失败: {e}"));
    }
    // 旧版正是在这里静默失败的（无错误检查却无条件报成功，见设计方案 §8 第 11 条）。
    // 删除后必须校验目标真的消失了。
    if path.exists() {
        return Outcome::Failed(format!("删除后路径仍然存在: {}", path.display()));
    }
    // 目录确实删掉了 —— 这是主要意图，算成功；但配置没写下去要如实说
    if let Some(why) = config_stale {
        return Outcome::Skipped(format!(
            "仓库目录已删除，但环境分支覆盖没从配置里清掉（{why}）——\
             配置里会留着一条指向它的规则，重新打开 hub 后再移除一次即可清掉"
        ));
    }
    Outcome::Ok
}

/// 仓库被哪些**需求**引用（`ws/<分支>`）。
///
/// 只有这一类会拦住「移除仓库」——需求里有真实的开发工作，删掉就丢了。
pub fn workspace_usages(hub: &Hub, name: &str) -> Vec<String> {
    let mut used = Vec::new();
    if let Ok(list) = std::fs::read_dir(hub.ws_dir()) {
        for entry in list.flatten() {
            if !entry.path().join(name).exists() {
                continue;
            }
            let branch = entry.file_name().to_string_lossy().to_string();
            // 报**需求名称（分支名）**，而不是 `ws/<分支>` 这种路径：
            // 分支名里那串日期人认不出来，而需求名称是使用者自己起的、一眼就知道是哪个。
            // 名称与分支名相同时（用户没起名）只报分支名，不重复一遍。
            let title = crate::workspace::Workspace::from_dir(&entry.path())
                .ok()
                .map(|ws| ws.title().to_string())
                .filter(|t| !t.is_empty() && t != &branch);
            used.push(match title {
                Some(t) => format!("{t}（{branch}）"),
                None => branch,
            });
        }
    }
    used.sort();
    used
}

/// 仓库在哪些**环境**里有检出（`envs/<标识>`）。
///
/// 这些是**镜像**，随时能从远程重建 —— 所以删仓库时**不拦**，跟着一起清掉。
/// （原来把它和需求一视同仁，于是「想删个仓库，得先把 dev 环境整个删掉」，
///  而环境是常驻的，等于根本删不掉。实测反馈。）
pub fn env_usages(hub: &Hub, name: &str) -> Vec<String> {
    let mut used = Vec::new();
    if let Ok(list) = std::fs::read_dir(hub.envs_dir()) {
        for entry in list.flatten() {
            if entry.path().join(name).exists() {
                used.push(format!("envs/{}", entry.file_name().to_string_lossy()));
            }
        }
    }
    used.sort();
    used
}

/// 并发对每个仓库执行同一操作，返回 `(仓库名, 结果)`。
///
/// 这些操作都是「起一个 git 子进程然后等网络」，纯 IO 等待。串行跑 22 个仓库的
/// `fetch` 要几十秒，界面就卡在那里——这正是需求 #8 抱怨的卡顿。
///
/// **用线程而不是 async**：这里没有需要异步运行时的细粒度等待，每个任务就是
/// 阻塞在一个子进程上，`std::thread::scope` 足够且零依赖。将来若要做「边跑边
/// 推结果给界面」的流式进度，再考虑换 tokio。
pub fn map_parallel<T, F>(hub: &Hub, f: F) -> Result<Vec<(String, Result<T>)>>
where
    T: Send,
    F: Fn(&Repo) -> Result<T> + Sync,
{
    let repos = list(hub)?;
    let f = &f;
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = repos
            .iter()
            .map(|r| scope.spawn(move || (r.name.clone(), f(r))))
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|_| {
                    (String::new(), Err(Error::other("工作线程异常退出")))
                })
            })
            .collect::<Vec<_>>()
    });
    Ok(results)
}

/// 让 `Path` 参与的调试输出更稳定（供上层展示相对路径）。
pub fn display_relative(hub: &Hub, path: &Path) -> String {
    path.strip_prefix(hub.root())
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_from_url_handles_common_forms() {
        assert_eq!(
            name_from_url("https://gitlab.example.com/team/demo-api.git").unwrap(),
            "demo-api"
        );
        assert_eq!(
            name_from_url("git@gitlab.example.com:team/demo-api.git").unwrap(),
            "demo-api"
        );
        assert_eq!(name_from_url("https://host/group/repo").unwrap(), "repo");
        assert_eq!(name_from_url("https://host/group/repo/").unwrap(), "repo");
        assert_eq!(name_from_url("https://host/group/repo.git/").unwrap(), "repo");
        assert!(name_from_url("").is_none());
    }

    #[test]
    fn name_from_url_handles_windows_local_paths() {
        // Windows 上的本地镜像路径用**反斜杠**：只按 `/` 与 `:` 切的话，最后一段是
        // 整条路径（`\Users\me\mirror\demo-api`），会被当成仓库名 →「名称非法」。
        // 这个错只在 Windows 上现形（CI 的 Windows job 抓到的）。
        assert_eq!(
            name_from_url(r"C:\Users\me\mirror\demo-api.git").unwrap(),
            "demo-api"
        );
        assert_eq!(
            name_from_url(r"\\srv\share\demo-api.git").unwrap(),
            "demo-api"
        );
        assert_eq!(name_from_url(r"C:\mirror\demo-api.git\").unwrap(), "demo-api");
    }

    #[test]
    fn outcome_helpers() {
        assert!(Outcome::Ok.is_ok() && !Outcome::Ok.is_failed());
        assert!(!Outcome::Skipped("x".into()).is_ok());
        assert!(Outcome::Failed("x".into()).is_failed());
    }
}