//! git 命令的**唯一出口**。
//!
//! ## 为什么调 `git` 二进制而不是用 `git2`（libgit2 绑定）
//!
//! 本工具的所有操作都要求与「人工在终端里敲 git 命令」**行为完全一致**——用户可能
//! 前脚用 `gws` 合了代码，后脚就在 IDE 里继续操作同一个 worktree。
//! libgit2 在 `worktree`、`merge --autostash`、`branch --unset-upstream` 等边缘语义上
//! 与 git 二进制存在偏差，而这些恰恰是本工具的核心依赖。详见设计方案 §1.1.1。

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};
use std::time::{Duration, Instant};

use crate::{Error, Result};

/// 在指定目录下执行 git 命令。
pub struct Git {
    cwd: PathBuf,
    /// GitLab token。仅在需要写私有仓库（推送 doc-hub 等）时才设。
    auth: Option<String>,
}

impl Git {
    /// `cwd` 接受 `&Path` / `&PathBuf` / `PathBuf`。
    ///
    /// **持有所有权而不是借用**：绝大多数调用点写的是
    /// `Git::new(ws.module_path(m))` 或 `Git::new(&some_path)`（借用检查器会
    /// 拒绝只借用临时 `PathBuf` 的写法）。为每个调用点先绑定一个局部变量能绕过，
    /// 但那把这个负担推给了几十处调用；克隆一个 `PathBuf` 便宜得多。
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            auth: None,
        }
    }

    /// 携带 GitLab token 执行（私有仓库的推送需要）。
    ///
    /// token 通过**环境变量注入 git 配置**（`GIT_CONFIG_COUNT` 机制，git ≥ 2.31），
    /// 而不是写进 argv 或 `.git/config`：
    /// - 写进 argv → 同机其它进程用 `ps` 就能看到
    /// - 写进 `.git/config` → 长期留在磁盘上，且会被 `git remote -v` 打印出来
    ///
    /// 用 Basic 认证是因为 GitLab 对 git-over-HTTP 明确支持
    /// 「任意非空用户名 + PAT 作密码」，Bearer 头在部分版本上不被接受。
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.auth = Some(token.into());
        self
    }

    /// 提交前确认「有可用的提交身份」，没有就报一条**能照做**的错。
    ///
    /// 为什么不直接把 git 的报错透出去：没配身份时 git 只说
    /// `fatal: empty ident name (for <user@host>) not allowed` —— 用户看不懂，
    /// 也不知道该去哪儿配。而这条路径（文档提交）是唯一由工具直接 commit 的地方，
    /// 在没配过 git 全局身份的机器上（CI、新装的 Windows）必然踩到。
    pub fn ensure_commit_identity(dir: &Path) -> Result<()> {
        let git = Git::new(dir);
        // `git var GIT_AUTHOR_IDENT` 会把「local → global → 环境变量」整条链解出来，
        // 全都没有时才失败 —— 正是我们要判的
        if git.out(&["var", "GIT_AUTHOR_IDENT"]).is_ok() {
            return Ok(());
        }
        Err(Error::other(
            "还没有配「提交身份」，无法提交。\n               桌面端：设置 → 提交身份，填名字与邮箱\n               命令行：gws identity --name 张三 --email zhangsan@example.com\n               （也可以 `git config --global user.name / user.email` 配全局的）",
        ))
    }

    /// 带上**当前 hub** 的 GitLab token（hub 没配就匿名）。
    ///
    /// # 为什么写远程的路径都要走它
    ///
    /// `Git` 不带 token 时，git 会自己去翻 credential helper / `~/.git-credentials` /
    /// ssh key —— 那是**全局凭据**，可能跟这个 hub 配的账号根本不是同一个。
    /// 同一台机器多套凭据正是这个项目要防的事，而症状很隐蔽：
    /// 界面里「推送环境」走 hub token、而 `gws push` / 合并后自动推送走全局凭据，
    /// 同一个 hub 里出现两种身份，提交记录上还看不出来。
    ///
    /// 私有仓库更直接：不带 token 会失败在 `could not read Username`。
    pub fn with_hub_token(self, hub: &crate::hub::Hub) -> Self {
        match crate::gitlab::token::resolve(hub.root()) {
            Ok(t) => self.with_token(t),
            Err(_) => self,
        }
    }

    /// 命令的工作目录。
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// 执行并捕获 stdout（去掉行尾换行）。非零退出码转成 [`Error::Git`]。
    pub fn out(&self, args: &[&str]) -> Result<String> {
        let output = self.raw(args)?;
        if !output.status.success() {
            return Err(Error::Git {
                args: args.join(" "),
                stderr: stderr_tail(&output.stderr),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string())
    }

    /// 执行且要求成功，丢弃输出。
    pub fn status(&self, args: &[&str]) -> Result<()> {
        let output = self.raw(args)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(Error::Git {
                args: args.join(" "),
                stderr: stderr_tail(&output.stderr),
            })
        }
    }

    /// **探测语义**：只关心成功与否，失败返回 `false` 而不构造错误。
    ///
    /// 这是本模块唯一允许「吞掉错误」的地方，因为调用方问的是
    /// 「这个 ref 存不存在」这类是非问题，不存在本身不是错误。
    /// 用于 `rev-parse --verify -q` / `show-ref --verify -q`。
    ///
    /// 注意：spawn 失败（git 未安装等）同样返回 `false`。需要区分这种情况时
    /// 请用 [`Git::out`]。
    pub fn ok(&self, args: &[&str]) -> bool {
        self.raw(args).map(|o| o.status.success()).unwrap_or(false)
    }

    /// **当前检出的**分支名（detached 时为空串）。
    ///
    /// 环境 worktree 的校验要用它：配置里说「这个环境该在 X 分支上」，
    /// 而磁盘上可能因为历史原因不在 X 上 —— 这时候往里合就是**合到别人的分支上**。
    pub fn current_branch(&self) -> Result<String> {
        self.out(&["branch", "--show-current"])
    }

    /// 把**已存在的 worktree** 切到另一条分支上。
    ///
    /// 本地有该分支就 `switch`，没有就从 `from`（如 `origin/dev`）建一条同名分支。
    /// 用在「环境 worktree 检出的分支与配置不符」时的纠正 ——
    /// 这种漂移一旦存在，往里合就会合到错的分支上（实测事故）。
    pub fn checkout_switch(&self, branch: &str, from: &str) -> Result<()> {
        if self.local_branch_exists(branch) {
            self.status(&["switch", "--quiet", branch])
        } else {
            self.status(&["switch", "--quiet", "-c", branch, from])
        }
    }

    /// 本地分支是否已存在（`refs/heads/<name>`）。
    pub fn local_branch_exists(&self, name: &str) -> bool {
        self.ok(&["show-ref", "--verify", "-q", &format!("refs/heads/{name}")])
    }

    /// 远程跟踪分支是否存在（`refs/remotes/origin/<name>`）。
    pub fn remote_branch_exists(&self, name: &str) -> bool {
        self.ok(&["rev-parse", "--verify", "-q", &format!("origin/{name}")])
    }

    /// 任意 ref 是否存在（`origin/main`、`HEAD`、`<sha>` 均可）。
    pub fn ref_exists(&self, refname: &str) -> bool {
        self.ok(&["rev-parse", "--verify", "-q", refname])
    }

    /// origin 的 URL。无 origin 时返回 `None`。
    pub fn remote_url(&self) -> Option<String> {
        self.out(&["remote", "get-url", "origin"])
            .ok()
            .filter(|s| !s.is_empty())
    }

    // ── 远程 ──────────────────────────────────────────────────────────

    /// `git fetch --prune origin`（静默）。
    ///
    /// 返回 `Result` 而不是静默吞掉：调用方多数时候只想尽力刷新引用，可以
    /// `let _ =`，但**不应由这里替调用方决定失败是否重要**。
    pub fn fetch(&self) -> Result<()> {
        // **网络操作，必须带墙钟上限**：远程不可达时 git 会一直挂着，
        // 界面就一直转圈、还不报错（见 NET_TIMEOUT 的说明）。
        let args = ["fetch", "--prune", "--quiet", "origin"];
        let output = self.capped(&args, NET_TIMEOUT)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(Error::Git {
                args: args.join(" "),
                stderr: stderr_tail(&output.stderr),
            })
        }
    }

    /// 短超时的 best-effort fetch，用于删除前的复核。
    ///
    /// 返回是否真的刷新成功；失败时调用方继续按本地 refs 判断。
    /// 用 [`QUICK_FETCH_TIMEOUT`] 而不是 [`NET_TIMEOUT`]：断网时不能让人
    /// 干等三分钟才看到风险提示。
    pub fn fetch_quick(&self) -> bool {
        self.capped(
            &["fetch", "--prune", "--quiet", "origin"],
            QUICK_FETCH_TIMEOUT,
        )
        .map(|o| o.status.success())
        .unwrap_or(false)
    }

    /// 推送分支。`set_upstream` 为真时同时建立跟踪关系。
    pub fn push(&self, branch: &str, set_upstream: bool) -> Result<()> {
        let flag = if set_upstream { "-u" } else { "" };
        let args = ["push", "--quiet", flag, "origin", branch]
            .iter()
            .copied()
            .filter(|a| !a.is_empty())
            .collect::<Vec<_>>();
        let output = self.capped(&args, NET_TIMEOUT)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(Error::Git {
                args: args.join(" "),
                stderr: stderr_tail(&output.stderr),
            })
        }
    }

    // ── 变更类操作 ────────────────────────────────────────────────────
    //
    // 这些操作的共同点：**非零退出未必是失败**。merge 冲突时 git 返回非零，
    // 但现场是好的、必须保留给人解决；用 `status()` 会把现场当异常抛掉。
    // 所以统一走 `capture()`，由调用方看 `unmerged_paths()` / `in_merge()` 判断。

    /// 执行命令，返回 `(是否成功, stdout+stderr)`，**不因非零退出报错**。
    pub fn capture(&self, args: &[&str]) -> Result<(bool, String)> {
        let output = self.raw(args)?;
        let mut text = String::from_utf8_lossy(&output.stdout).to_string();
        if !output.stderr.is_empty() {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&String::from_utf8_lossy(&output.stderr));
        }
        Ok((output.status.success(), text.trim_end().to_string()))
    }

    /// `git merge --no-edit [--autostash] <target>`。
    ///
    /// `autostash` 会在合并前把未提交改动 stash、合完回填——否则 git 会因为
    /// 「本地改动会被覆盖」整个拒绝合并，等于把人挡在门外。
    pub fn merge(&self, target: &str, autostash: bool) -> Result<(bool, String)> {
        if autostash {
            self.capture(&["merge", "--no-edit", "--autostash", target])
        } else {
            self.capture(&["merge", "--no-edit", target])
        }
    }

    pub fn merge_abort(&self) -> Result<()> {
        self.capture(&["merge", "--abort"]).map(|_| ())
    }

    pub fn rebase(&self, onto: &str) -> Result<(bool, String)> {
        self.capture(&["rebase", onto])
    }

    /// 用既有的合并信息补一次提交（解决冲突后收尾）。
    ///
    /// `-c core.editor=true` 避免 git 打开编辑器等人输入——非交互场景下
    /// 那会一直挂着。
    pub fn commit_no_edit(&self) -> Result<()> {
        self.status(&["-c", "core.editor=true", "commit", "--quiet", "--no-edit"])
    }

    pub fn reset_hard(&self, target: &str) -> Result<()> {
        self.status(&["reset", "--hard", "--quiet", target])
    }

    /// 清理未跟踪文件。
    ///
    /// `keep` 是要保留的路径模式（如 `node_modules`、`target`）——它们体积大且
    /// 重新生成成本高，清掉会让下一次构建变得很慢。
    pub fn clean_untracked(&self, keep: &[&str]) -> Result<()> {
        let mut args = vec!["clean", "-fdq"];
        for k in keep {
            args.push("-e");
            args.push(k);
        }
        self.status(&args)
    }

    /// stash 条目列表（`stash@{0}` …）。
    pub fn stash_list(&self) -> Vec<String> {
        self.out(&["stash", "list", "--format=%gd"])
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    pub fn stash_drop(&self, stash_ref: &str) -> Result<()> {
        self.status(&["stash", "drop", stash_ref])
    }

    // ── worktree ──────────────────────────────────────────────────────

    /// 清理僵尸 worktree 元数据（手工删目录 / 中断残留）。
    ///
    /// **所有 worktree 操作前都应先跑它。** 旧版踩过的坑：不 prune
    /// 就直接 `worktree add`，会撞上 `already exists` / 占用同名路径。
    pub fn worktree_prune(&self) -> Result<()> {
        self.status(&["worktree", "prune"])
    }

    /// 新建 worktree。
    ///
    /// `create_branch` 为真时等价于 `worktree add -b <branch> <path> <base>`；
    /// 为假时检出已存在的分支 `worktree add <path> <branch>`。
    pub fn worktree_add(
        &self,
        path: &Path,
        branch: &str,
        create_branch: bool,
        base: Option<&str>,
    ) -> Result<()> {
        let path_str = path.to_string_lossy().to_string();
        let attempt = |g: &Self| match (create_branch, base) {
            (true, Some(base)) => g.status(&["worktree", "add", "-b", branch, &path_str, base]),
            (true, None) => g.status(&["worktree", "add", "-b", branch, &path_str]),
            (false, _) => g.status(&["worktree", "add", &path_str, branch]),
        };

        let first = attempt(self);
        if first.is_ok() {
            return Ok(());
        }

        // ── 失败就先「修一遍半截状态」，再重试**一次** ──────────────────
        //
        // 上一次操作被打断（被杀/断电/手工删目录）会留下两类东西，都会让本次失败：
        //
        //   ① **指向已消失目录的 worktree 登记** —— 分支因此"被占着"，
        //      报错是 `'<分支>' is already used by worktree at <已不存在的路径>`
        //   ② **目标目录的半截残留**
        //
        // 两步都只清理**本来就不该存在**的东西（登记指向的目标已经没了、
        // 我们要创建的那个目录里的残留），不会碰任何有用的内容。
        // 实测反馈里其他人卡在 Windows 上的 `Could not reset index file` 就属于这一类
        // —— 报错完全看不出该怎么办，而它其实只差一次清理。
        let _ = self.worktree_prune();
        if path.exists() {
            let _ = std::fs::remove_dir_all(path);
        }
        if attempt(self).is_ok() {
            return Ok(());
        }

        // ── 再换一条路：**把它当"从远程新建"来做** ─────────────────────
        //
        // 实测（Windows，其他人的机器）：目标目录没有陈旧登记、本地分支与
        // `origin/<分支>` 的 sha 完全一致，`worktree add <路径> <分支>`
        // （**复用已存在的本地分支**那条路）仍然报
        // `fatal: Could not reset index file to revision 'HEAD'`；
        // 而 `worktree add -b <分支> <路径> origin/<分支>`（从远程新建那条路）
        // 在他机器上是好的（同一个 hub 里别的需求就是这么建出来的）。
        //
        // 既然两边逐字一致，就没必要走那条会失败的路：**删掉本地分支、按远程重建**。
        //
        // ⚠️ 前提是「本地没有超前远程的提交」—— 有的话删了就**丢代码**，
        // 这时宁可失败也不能动它（判定见 `can_recreate_from_remote`）。
        if !create_branch && self.can_recreate_from_remote(branch) {
            let _ = std::fs::remove_dir_all(path);
            let _ = self.status(&["branch", "-D", branch]);
            if self
                .status(&[
                    "worktree",
                    "add",
                    "-b",
                    branch,
                    &path_str,
                    &format!("origin/{branch}"),
                ])
                .is_ok()
            {
                return Ok(());
            }
        }

        // 还是不行：把诊断信息一并给出，别让人只看到一句看不懂的 git 报错
        Err(Error::other(format!(
            "{}\n\n已经自动清理过（`worktree prune` + 清掉目标目录残留）并重试，仍然失败。\n             诊断：\n{}",
            first.unwrap_err(),
            self.worktree_add_diagnostics(branch)
        )))
    }

    /// **本地分支能不能删了从远程重建**（= 它没有超前 `origin/<分支>` 的提交）。
    ///
    /// 这条判定是「换个建法」那步的**安全闸**：只要本地有远程没有的提交，
    /// 删分支就是丢代码 —— 那种情况下宁可失败，也不能动它。
    ///
    /// 读不到 `origin/<分支>`（没推过、或离线）一律返回 `false`：**保守**。
    pub fn can_recreate_from_remote(&self, branch: &str) -> bool {
        let remote = format!("origin/{branch}");
        if !self.ref_exists(&remote) {
            return false;
        }
        matches!(self.count_range(&format!("{remote}..{branch}")), Ok(0))
    }

    /// `worktree add` 失败时收集现场，好让报错能指出下一步。
    fn worktree_add_diagnostics(&self, branch: &str) -> String {
        let mut out = String::new();
        out.push_str("  · 当前 worktree 列表：\n");
        for l in self.out(&["worktree", "list"]).unwrap_or_default().lines() {
            out.push_str(&format!("      {l}\n"));
        }
        let local = self
            .sha(&format!("refs/heads/{branch}"))
            .unwrap_or_else(|_| "（读不到）".into());
        let remote = self
            .sha(&format!("origin/{branch}"))
            .unwrap_or_else(|_| "（读不到）".into());
        out.push_str(&format!("  · 本地分支 {branch}: {local}\n"));
        out.push_str(&format!("  · origin/{branch}: {remote}\n"));
        out.push_str(&format!(
            "  · 本仓库 HEAD: {}\n",
            self.sha("HEAD").unwrap_or_else(|_| "（读不到）".into())
        ));

        // **Windows 上最常见的那一类**：仓库里有文件的完整路径超过 260 字符。
        // git 只会说 `Could not reset index file to revision 'HEAD'`，
        // 完全看不出跟路径长度有关 —— 所以这里直接把肇事文件列出来。
        let prefix = self.cwd.to_string_lossy().chars().count();
        if let Ok(listing) = self.out(&["ls-tree", "-r", "--name-only", "HEAD"]) {
            let paths: Vec<String> = listing.lines().map(str::to_string).collect();
            let over = paths_over_windows_limit(&paths, prefix, 3);
            if !over.is_empty() {
                out.push_str("  · **这些文件的完整路径超过 Windows 的 260 字符上限**（这就是 checkout 写不出来的原因）：\n");
                for (p, len) in &over {
                    out.push_str(&format!("      {len} 字符  {p}\n"));
                }
                out.push_str(
                    "  · 修法：① 把 hub 放到更短的路径下（如 D:\\gws）；                     ② 或给 Windows 启用长路径（注册表 LongPathsEnabled=1 后重启）\n",
                );
                return out;
            }
        }
        out.push_str("  · 若是「本地副本不完整」：到「仓库」页把这个仓库删掉重新拉一次");
        out
    }

    pub fn worktree_remove(&self, path: &Path, force: bool) -> Result<()> {
        let path = path.to_string_lossy().to_string();
        if force {
            self.status(&["worktree", "remove", "--force", &path])
        } else {
            self.status(&["worktree", "remove", &path])
        }
    }

    /// 修复 worktree 的元数据指向（目录被移动后使用）。
    pub fn worktree_repair(&self, path: &Path) -> Result<()> {
        let path = path.to_string_lossy().to_string();
        self.status(&["worktree", "repair", &path])
    }

    /// 移动一个 worktree 的目录位置。
    ///
    /// **必须用它，不能 `mv`**：worktree 里的 `.git` 是个文件，内容是主 clone 里
    /// gitdir 的**绝对路径**；主 clone 里 `.git/worktrees/<n>/gitdir` 又反过来
    /// 记着 worktree 的绝对路径。`mv` 会让两个方向全部失效，
    /// 而 `worktree move` 会同时维护它们。
    pub fn worktree_move(&self, from: &Path, to: &Path) -> Result<()> {
        let from = from.to_string_lossy().to_string();
        let to = to.to_string_lossy().to_string();
        self.status(&["worktree", "move", &from, &to])
    }

    /// 解除当前分支的上游跟踪。
    ///
    /// **从 `origin/<主干>` 建分支后必须调用。** 旧版踩过的坑：
    /// git 会自动把上游设成主干，导致 IDE 里的 Push 直接推向 `origin/main`
    /// 而不是需求分支。
    pub fn unset_upstream(&self) -> Result<()> {
        self.status(&["branch", "--unset-upstream"])
    }

    /// 把当前分支的上游设为 `origin/<branch>`（协作场景：直接 pull/push 该分支）。
    pub fn set_upstream(&self, branch: &str) -> Result<()> {
        self.status(&["branch", "--set-upstream-to", &format!("origin/{branch}")])
    }

    // ── 状态与计数 ────────────────────────────────────────────────────

    /// `git status --porcelain` 的原始输出。空串代表工作树干净。
    pub fn status_porcelain(&self) -> Result<String> {
        self.out(&["status", "--porcelain"])
    }

    /// 是否有未提交改动（含未跟踪文件）。
    ///
    /// 返回 `Result` 而非 `bool`：这个判断的调用方都是「要不要执行破坏性操作」，
    /// 读不到状态时**默认成"干净"会直接导致丢代码**。
    /// 有未提交的改动吗（**不计操作系统垃圾文件**，见 [`Self::dirty_count`]）。
    pub fn is_dirty(&self) -> Result<bool> {
        Ok(self.dirty_count()? > 0)
    }

    /// 未提交改动的条目数（对应旧版的 `wc -l`）。
    ///
    /// # 为什么要把操作系统垃圾文件排除掉
    ///
    /// 这是**实测踩到的死锁**：macOS 会在每个目录里撒 `.DS_Store`（Windows 是
    /// `Thumbs.db` / `desktop.ini`），而且是未跟踪文件。原来直接数 porcelain 的
    /// 行数，于是这些文件让工作区**永远是脏的**：
    ///
    /// ```text
    /// 推送 →「先提交」→ 用户确实提交了 → 状态里还剩那几个 .DS_Store → 还是「先提交」
    /// ```
    ///
    /// 用户会以为自己没提交成功。而这些文件**永远不会被提交**，`git push`
    /// 也只推提交 —— 拿它们拦推送纯属误伤。
    ///
    /// 注意：**只排除这几类系统垃圾，不排除普通的未跟踪文件** ——
    /// 新写了一个文档没 `git add`，那确实是「还没提交」，该拦。
    pub fn dirty_count(&self) -> Result<u64> {
        Ok(self.status_lines(false)?.len() as u64)
    }

    /// `status --porcelain` 的行，**滤掉操作系统垃圾文件**。
    ///
    /// # 所有「有没有未提交改动」的判断都必须走这里
    ///
    /// 这个坑踩过**两次**，第二次就是因为有两份判断各写各的：
    /// 先修了推送闸门（`dirty_count`），文档页那边自己数 porcelain 行，
    /// 于是同一个 `.DS_Store` 又让它报「4 个变更」（实测反馈）。
    /// 收到唯一出口，就不会再冒出第三条路径。
    pub fn status_lines(&self, untracked_all: bool) -> Result<Vec<String>> {
        let mut args = vec!["status", "--porcelain"];
        // -uall：默认只报未跟踪的**目录**（`?? workspaces/`），看不出到底有哪些文件
        if untracked_all {
            args.push("--untracked-files=all");
        }
        Ok(self
            .out(&args)?
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter(|l| !is_os_junk(porcelain_path(l)))
            .map(str::to_string)
            .collect())
    }

    /// `rev-list --count <revs...>`。多个 revision 必须**分开传参**。
    ///
    /// 曾经把它写成 `count("feature-x ^origin/main")` 这种空格拼接的单个字符串，
    /// git 会把整个串当成一个 revision 名解析失败——而失败又被静默当成 0。
    /// 后果是「分支领先主干几个提交」被算成 0，于是 `drop_missing_modules`
    /// 认为没有未推送提交、把分支删了。**这类静默归零是本项目明令禁止的模式。**
    pub fn count(&self, revs: &[&str]) -> Result<u64> {
        let mut args = vec!["rev-list", "--count"];
        args.extend_from_slice(revs);
        Ok(self.out(&args)?.trim().parse().unwrap_or(0))
    }

    /// `<tip>` 领先 `<base>` 几个提交（`rev-list --count <tip> ^<base>`）。
    pub fn count_ahead(&self, tip: &str, base: &str) -> Result<u64> {
        self.count(&[tip, &format!("^{base}")])
    }

    /// `<range>` 内的提交数（如 `origin/main..HEAD`）。
    pub fn count_range(&self, range: &str) -> Result<u64> {
        self.count(&[range])
    }

    /// `HEAD` 上**补丁还没有进入 `upstream`** 的提交数。
    ///
    /// 只靠 `is_ancestor` 判断会漏掉 squash / rebase 合并：改动已经在主干里，
    /// 但提交 SHA 变了、祖先关系不成立。`git cherry` 按补丁内容比对，
    /// 输出 `+` 的才是主干里真正没有的提交。
    pub fn count_not_upstream(&self, upstream: &str) -> Result<u64> {
        let out = self.out(&["cherry", upstream, "HEAD"])?;
        Ok(out.lines().filter(|l| l.starts_with('+')).count() as u64)
    }

    /// 双向计数 `HEAD...<other>` → `(ahead, behind)`。
    ///
    /// 旧版踩过的坑：只数单向时，远程被强推/重建会算出 0
    /// 等于把「提交被抹掉了」这件事藏起来。回退和分叉都必须显式报出来，
    /// 所以这里一次给出两个方向。
    pub fn ahead_behind(&self, other: &str) -> Result<(u64, u64)> {
        let out = self.out(&[
            "rev-list",
            "--left-right",
            "--count",
            &format!("HEAD...{other}"),
        ])?;
        let mut parts = out.split_whitespace();
        let ahead = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let behind = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        Ok((ahead, behind))
    }

    /// 同上，但**这个 ref 不存在时返回 `None` 而不是错误**。
    ///
    /// # 为什么要把「ref 存不存在」并进来
    ///
    /// 调用方（模块状态那一行）本来要问两件事：「这条远程分支有吗」和「落后几个」——
    /// 那是**两次 git 调用**。而 Windows 上每次 git 调用要 ~85ms（起进程才是大头，
    /// 实测），一个模块少一次就是 4 个模块省掉 0.3 秒（实测反馈：进需求要转圈等 2 秒）。
    ///
    /// `rev-list` 对不存在的 revision 会**非零退出**，正好一次调用回答两个问题。
    pub fn ahead_behind_opt(&self, other: &str) -> Option<(u64, u64)> {
        let out = self
            .raw(&[
                "rev-list",
                "--left-right",
                "--count",
                &format!("HEAD...{other}"),
            ])
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut parts = text.split_whitespace();
        let ahead = parts.next()?.parse().ok()?;
        let behind = parts.next()?.parse().ok()?;
        Some((ahead, behind))
    }

    /// `a` 是否是 `b` 的祖先。用于判断「本地 HEAD 是否还包含远程 tip」——
    /// 不包含说明远程被 force push 重建过。
    ///
    /// 探测语义：返回 `false` 同时涵盖「不是祖先」和「ref 读不到」，
    /// 两者对调用方的决策一致（都按"本地副本已作废"处理）。
    pub fn is_ancestor(&self, a: &str, b: &str) -> bool {
        self.ok(&["merge-base", "--is-ancestor", a, b])
    }

    /// **只允许快进**地合入 `target`，绝不产生合并提交。
    ///
    /// 与 [`Self::merge`] 的区别：分叉时**直接失败**（`--ff-only`），
    /// 不会留下一个合并提交、也不会弹出冲突要人来收拾。
    /// 用在「这份 clone 只是给人看的镜像」这种地方 —— 比如文档库主 clone
    /// （已归档文档的工作树），它本地不该有自己的提交。
    pub fn merge_ff_only(&self, target: &str) -> Result<()> {
        self.status(&["merge", "--ff-only", target])
    }

    /// 某 ref 的完整 SHA。
    pub fn sha(&self, refname: &str) -> Result<String> {
        self.out(&["rev-parse", refname])
    }

    /// 是否处于合并中途（存在 `MERGE_HEAD`）。
    pub fn in_merge(&self) -> bool {
        self.ref_exists("MERGE_HEAD")
    }

    /// 冲突未解决的路径列表。
    ///
    /// 旧版踩过的坑：`merge --autostash` **回填 stash 时冲突，git 仍然
    /// 返回 0**。所以判断冲突只能看这个列表，不能只看退出码。
    pub fn unmerged_paths(&self) -> Result<Vec<String>> {
        Ok(self
            .out(&["diff", "--name-only", "--diff-filter=U"])?
            .lines()
            .map(str::to_string)
            .filter(|l| !l.is_empty())
            .collect())
    }

    /// 工作树相对某个 range 的改动统计（`git diff --shortstat <range>`）。
    pub fn shortstat(&self, range: &str) -> Result<String> {
        self.out(&["diff", "--shortstat", range])
    }

    /// 有墙钟上限的执行（网络类命令用；见 [`NET_TIMEOUT`]）。
    fn capped(&self, args: &[&str], limit: Duration) -> Result<Output> {
        let mut cmd = self.make_cmd(args);
        let what = format!("git {}", args.join(" "));
        run_capped(&mut cmd, &what, limit)
    }

    /// 组装一条 git 命令（quotePath 与凭据都在这里统一处理）。
    fn make_cmd(&self, args: &[&str]) -> Command {
        let mut cmd = git_cmd();
        cmd.current_dir(&self.cwd);
        // **所有 git 命令一律关掉路径转义。**
        //
        // git 默认把非 ASCII 路径转义成八进制（`"specs/\347\233\264..."`）。
        // 任何一个「输出里带路径」的命令被它转义，都会出两类问题：
        //   ① 界面上显示成一串乱码；
        //   ② 拿这个转义串当路径去读文件 → `No such file`。
        // 而且它是**静默**的：命令成功、输出有内容，只是内容是转义过的。
        //
        // 这个坑本仓已经踩过三次（`dochub::sync`、`archived_in_main`、
        // `browse_changes`），每次都是「记得加就好、换个地方又忘」。
        // 收到唯一出口就不会再漏了 —— 需要原始转义行为的场景这里没有。
        cmd.args(COMMON_FLAGS);
        cmd.args(args);
        if let Some(token) = &self.auth {
            // GitLab：任意非空用户名 + PAT 作密码
            let basic = base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                format!("oauth2:{token}"),
            );
            cmd.env("GIT_CONFIG_COUNT", "1")
                .env("GIT_CONFIG_KEY_0", "http.extraHeader")
                .env(
                    "GIT_CONFIG_VALUE_0",
                    format!("Authorization: Basic {basic}"),
                );
        }
        cmd
    }

    fn raw(&self, args: &[&str]) -> Result<Output> {
        let mut cmd = self.make_cmd(args);
        cmd.output()
            .map_err(|e| Error::other(format!("无法执行 git（是否已安装并加入 PATH？）: {e}")))
    }
}

/// 在给定工作区前缀下，会超过 Windows 260 字符上限的仓库内路径。
///
/// 纯函数（只吃「相对路径 + 前缀长度」），所以 **在 mac 上就能测** ——
/// 而它要回答的正是「为什么 Windows 上 checkout 不出来」，
/// 那句 `Could not reset index file` 完全看不出跟路径长度有关。
///
/// 返回 `(路径, 完整长度)`，按长度倒序；最多给 `limit` 条（诊断信息不该刷屏）。
pub fn paths_over_windows_limit(
    rel_paths: &[String],
    prefix_len: usize,
    limit: usize,
) -> Vec<(String, usize)> {
    /// Windows 的传统上限（不含结尾的 NUL）。
    const WIN_MAX: usize = 260;
    let mut over: Vec<(String, usize)> = rel_paths
        .iter()
        .map(|p| (p.clone(), prefix_len + 1 + p.chars().count()))
        .filter(|(_, len)| *len > WIN_MAX)
        .collect();
    over.sort_by_key(|(_, len)| std::cmp::Reverse(*len));
    over.truncate(limit);
    over
}

/// 每一条 git 命令都要带的 `-c` 覆盖。**只此一份**，不许在别处再写一遍
/// （写两份迟早漂，而这两条都是"不加就静默出错"的）。
///
/// # `core.quotePath=false`
///
/// git 默认把非 ASCII 路径转义成八进制（`"specs/\347\233\264..."`）：界面显示成乱码、
/// 拿它当路径去读文件必然 `No such file`，而且**命令是成功的**、只是内容是转义过的。
/// 这个坑本仓踩过三次。
///
/// # `core.longpaths=true`（Windows）
///
/// Windows 的 260 字符路径上限。实测（其他人的机器，两次踩到两种表现）：
///
/// 1. **`worktree add` 失败**：仓库里有文件的完整路径太长（真实数据：相对路径 256
///    字符 + 工作区前缀 81 = 338），checkout 写不出来，git 只报
///    `fatal: Could not reset index file to revision 'HEAD'` ——
///    **完全看不出跟路径长度有关**（完整报错里其实还有一行 `Filename too long`，
///    界面上被截掉了）。同一条命令在 macOS 上成功（上限 1024），所以只在 Windows 炸。
/// 2. **`clone` 静默留下残废的仓库**：长路径下 `git clone` **退出码是 0**，
///    但那些文件根本没落盘 —— 之后 `git status` 把它们显示成"已删除"。
///    这就是「他的仓库里很多模块都没拉下来」的真身。
///
/// 在 Windows 上实测确认：不带 → `Could not reset index file`；带上 → 正常检出。
const COMMON_FLAGS: [&str; 4] = ["-c", "core.quotePath=false", "-c", "core.longpaths=true"];

/// 建一条 git 命令。**所有** git 调用都要经过它。
///
/// # Windows 上必须带 `CREATE_NO_WINDOW`
///
/// GUI 程序（没有控制台）启动**控制台子进程**时，Windows 会给它新建一个控制台窗口 ——
/// 每跑一条 git 就弹一个黑色窗口，几十条命令就是几十个窗口刷屏，而且会**抢焦点**。
/// 实测反馈：其他人那边看着就是"界面一直闪、一直弹窗口"。
///
/// `CREATE_NO_WINDOW`（0x08000000）就是为这个场景准备的：进程照跑，但不给它窗口。
/// （旧版 gws 也踩过这一条，用户明确记得"要设置个什么东西才能让它不弹出来"。）
///
/// 其余平台不需要，原样建命令即可。
fn git_cmd() -> Command {
    // Windows 下要改成可变来设置 creation_flags；其它平台用不上
    #[allow(unused_mut)]
    let mut cmd = Command::new("git");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// `git clone`。`dest` 的父目录是命令的工作目录。
///
/// 与 `Git::out` 分开是因为 clone 的作用目录是**父目录**而非目标目录本身
/// （目标目录尚不存在）。
pub fn clone(url: &str, dest: &Path) -> Result<()> {
    clone_with_token(url, dest, None)
}

/// `git clone`，可带 token（私有仓库）。公开仓库传 `None` 即可。
pub fn clone_with_token(url: &str, dest: &Path, token: Option<&str>) -> Result<()> {
    let parent = dest
        .parent()
        .ok_or_else(|| Error::other(format!("目标路径没有父目录: {}", dest.display())))?;
    let dest_str = dest.to_string_lossy().to_string();
    let mut cmd = git_cmd();
    cmd.current_dir(parent)
        .args(COMMON_FLAGS)
        .args(["clone", "--quiet", url, &dest_str]);
    if let Some(token) = token {
        let basic = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            format!("oauth2:{token}"),
        );
        cmd.env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "http.extraHeader")
            .env(
                "GIT_CONFIG_VALUE_0",
                format!("Authorization: Basic {basic}"),
            );
    }
    // **必带超时**：地址写错 / 网络不通时 git 会一直挂着（实测反馈：
    // 新建 hub、添加模块时永远 loading，没有任何报错）。
    let output = run_capped(&mut cmd, &format!("git clone {url}"), NET_TIMEOUT)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::Git {
            args: format!("clone {url} {dest_str}"),
            stderr: stderr_tail(&output.stderr),
        })
    }
}

/// 把 hub 绑定的提交身份写进这个仓库的 **local config**。
///
/// # 为什么设在 local config，而不是每次命令带 `-c`
///
/// hub 里的 worktree 与主 clone **共享同一份 local config**（实测：
/// worktree 的 `--git-common-dir` 指向主仓库 `.git`，且
/// `extensions.worktreeConfig` 未开启，所以 `git config --local` 读写的
/// 就是主仓库那一份）。于是：
///
/// - 每个仓库只需设**一次**，它的全部 worktree（`ws/`、`envs/`）自动继承；
/// - 用户在 `ws/<分支>/<模块>/` 里**手工 `git commit`** 用的也是这个身份
///   —— 这才是「读到错误身份」的真正来源，只在工具自己执行的命令上加
///   `-c` 是挡不住的。
///
/// 身份没配（两个字段留空）时**删掉** local config 里的 user.name / user.email：
/// 那表示「这个 hub 不干预身份，回去用全局配置」。只删不写是必须的 ——
/// 否则之前设过的身份会**继续生效**，用户改了配置却看不到任何变化。
/// （`--unset` 在键不存在时返回非 0，忽略即可。）
pub fn apply_identity(repo_dir: &Path, id: &crate::hub::GitIdentity) -> Result<()> {
    let git = Git::new(repo_dir);
    if !id.is_set() {
        let _ = git.out(&["config", "--local", "--unset", "user.name"]);
        let _ = git.out(&["config", "--local", "--unset", "user.email"]);
        return Ok(());
    }
    git.out(&["config", "--local", "user.name", id.author_name.trim()])?;
    git.out(&["config", "--local", "user.email", id.author_email.trim()])?;
    Ok(())
}

/// 读 git **全局**身份（`git config --global user.name / user.email`）。
///
/// 用来给「新建 hub」的提交身份字段做**预填** —— 大多数人本来就配过全局身份，
/// 让他在建 hub 时再手打一遍名字和邮箱是白费事。两个都读不到就是 `None`。
pub fn global_identity() -> Option<crate::hub::GitIdentity> {
    // 用一个不存在的 cwd 也成立：`--global` 不依赖仓库
    let dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let git = Git::new(dir);
    let read = |key: &str| {
        git.out(&["config", "--global", "--get", key])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let id = crate::hub::GitIdentity {
        author_name: read("user.name")?,
        author_email: read("user.email")?,
    };
    id.is_set().then_some(id)
}
/// 主干分支名探测：`origin/main` → `origin/master` → 查远程 HEAD。
///
/// 各仓库主干名不统一（`main`/`master` 混用），写死任何一个都会在另一批仓库上
/// 失败，所以必须逐个探测。前两种都能在本地判断，只有都不命中时才走网络查
/// `ls-remote`。
pub fn detect_main_branch(repo_dir: &Path) -> Result<String> {
    let git = Git::new(repo_dir);
    if git.remote_branch_exists("main") {
        return Ok("main".to_string());
    }
    if git.remote_branch_exists("master") {
        return Ok("master".to_string());
    }
    // 主干被改名（如 develop）→ 查远程 HEAD 指向
    let out = git.out(&["ls-remote", "--symref", "origin", "HEAD"])?;
    // 形如: ref: refs/heads/develop\tHEAD
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("ref: refs/heads/") {
            if let Some((name, _)) = rest.split_once(char::is_whitespace) {
                return Ok(name.to_string());
            }
        }
    }
    Err(Error::other(format!(
        "无法探测主干分支: {}（origin/main、origin/master 均不存在，远程 HEAD 也读不到）",
        repo_dir.display()
    )))
}

/// stderr 的最后一个非空行。git 的真实原因只在这里，不带上就只剩一个退出码。
fn stderr_tail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

/// 删除前复核用的短超时（见 [`Git::fetch_quick`]）。
pub const QUICK_FETCH_TIMEOUT: Duration = Duration::from_secs(15);

/// 网络操作的时间上限（超过就杀）。
///
/// # 为什么必须自己加超时
///
/// **git 自己没有超时**：远程不可达时（断网、VPN 没连、地址写错、对端黑洞），
/// `clone` / `fetch` / `push` 会一直挂着 —— 界面就一直转圈，**没有任何报错**，
/// 人分不清是在跑还是卡死了。实测反馈：新建 hub、添加模块时永远 loading。
///
/// 本项目的一条硬规则是「失败必须说话」，所以到点就杀，并给出能照着做的原因。
pub const NET_TIMEOUT: Duration = Duration::from_secs(180);

/// 轮询等待子进程结束；到点就**杀掉**并报**能照做**的错。
///
/// 单独抽出来是为了能直接测：「到点真的会杀、并给出原因」是这一整套的核心，
/// 而造一个「一定会挂住的子进程」比造一个「一定会挂住的 git 网络操作」稳得多。
fn wait_capped(child: &mut std::process::Child, what: &str, limit: Duration) -> Result<ExitStatus> {
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return Ok(st),
            Ok(None) => {
                if Instant::now() >= deadline {
                    // 杀掉再收尸：别留一个还在跑的 git 在背后改目录
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Error::other(format!(
                        "{what} 超过 {} 秒没有结束 —— 多半是连不上远程。\
                         检查网络 / VPN / 地址；确认能访问后再试。",
                        limit.as_secs()
                    )));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(Error::other(format!("等待 git 结束失败: {e}"))),
        }
    }
}

/// 跑一个命令，最多等 `limit`；超时就杀掉并报**能照做**的错。
///
/// 输出走**临时文件**而不是管道：用管道的话，我们只在轮询等待、不去读管道，
/// 子进程写满缓冲区就会阻塞，而我们又在等它退出 —— 互相等死。
fn run_capped(cmd: &mut Command, what: &str, limit: Duration) -> Result<Output> {
    use std::io::Read;
    use std::process::Stdio;

    // 文件名带上 pid + 纳秒，避免并发时互相覆盖
    let tag = format!(
        "gws-git-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let dir = std::env::temp_dir();
    let (out_path, err_path) = (
        dir.join(format!("{tag}.out")),
        dir.join(format!("{tag}.err")),
    );
    let out_file = std::fs::File::create(&out_path)
        .map_err(|e| Error::other(format!("创建临时文件失败: {e}")))?;
    let err_file = std::fs::File::create(&err_path)
        .map_err(|e| Error::other(format!("创建临时文件失败: {e}")))?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file));

    let mut child = cmd
        .spawn()
        .map_err(|e| Error::other(format!("无法执行 git: {e}")))?;
    let status = match wait_capped(&mut child, what, limit) {
        Ok(st) => st,
        Err(e) => {
            let _ = std::fs::remove_file(&out_path);
            let _ = std::fs::remove_file(&err_path);
            return Err(e);
        }
    };

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Ok(mut f) = std::fs::File::open(&out_path) {
        let _ = f.read_to_end(&mut stdout);
    }
    if let Ok(mut f) = std::fs::File::open(&err_path) {
        let _ = f.read_to_end(&mut stderr);
    }
    let _ = std::fs::remove_file(&out_path);
    let _ = std::fs::remove_file(&err_path);
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// 从 `git status --porcelain` 的一行里取出路径。
///
/// v1 格式是 `XY <path>`（前两列状态码，第 3 列起是路径）；重命名是
/// `XY old -> new`，取新的那个。路径可能被引号包着（含特殊字符时）。
fn porcelain_path(line: &str) -> &str {
    let rest = line.get(3..).unwrap_or("");
    let p = rest.rsplit(" -> ").next().unwrap_or(rest);
    p.trim().trim_matches('"')
}

/// 是不是操作系统自动生成的垃圾文件（永远不该被提交，也不该算「未提交改动」）。
///
/// `.DS_Store` 与 `._*` 来自 macOS，`Thumbs.db` / `desktop.ini` 来自 Windows。
fn is_os_junk(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    matches!(name, ".DS_Store" | "Thumbs.db" | "desktop.ini") || name.starts_with("._")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stderr_tail_returns_last_non_empty_line() {
        assert_eq!(stderr_tail(b"a\nb\n\n"), "b");
        assert_eq!(stderr_tail(b"only"), "only");
        assert_eq!(stderr_tail(b""), "");
        assert_eq!(stderr_tail(b"\n\n"), "");
    }

    #[test]
    fn stderr_tail_handles_invalid_utf8() {
        // git 在中文路径下可能吐出非 UTF-8 字节，不能因此 panic
        assert_eq!(stderr_tail(b"\xff\xfe bad"), "\u{fffd}\u{fffd} bad");
    }
}
#[cfg(test)]
mod quote_path_tests {
    use super::*;

    /// git 默认把非 ASCII 路径转义成八进制，而本工具到处在处理中文路径。
    /// 这个坑踩过三次（`dochub::sync` / `archived_in_main` / `browse_changes`），
    /// 所以把 `-c core.quotePath=false` 收进唯一出口，并在这里钉住它 —— 谁把
    /// 那行删了，这条测试会红。
    #[test]
    fn non_ascii_paths_are_not_escaped() {
        let dir = tempfile::TempDir::new().unwrap();
        let git = Git::new(dir.path());
        git.status(&["init", "-q", "-b", "main"]).unwrap();
        git.status(&[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ])
            .unwrap();
        std::fs::create_dir_all(dir.path().join("specs/直播间评论区优化-PC")).unwrap();
        std::fs::write(dir.path().join("specs/直播间评论区优化-PC/ver证.cjs"), "x").unwrap();
        git.status(&["add", "-A"]).unwrap();
        git.status(&[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "add",
        ])
        .unwrap();
        std::fs::write(dir.path().join("specs/直播间评论区优化-PC/ver证.cjs"), "y").unwrap();

        let out = git.out(&["status", "--porcelain"]).unwrap();
        assert!(
            out.contains("直播间评论区优化-PC"),
            "中文路径不该被转义：{out}"
        );
        assert!(!out.contains("\\3"), "不该出现八进制转义：{out}");

        // 更要紧的：拿到的路径必须真的能读（转义串拿去读会 No such file）
        let listed = git.out(&["diff", "--name-only"]).unwrap();
        for line in listed.lines().filter(|l| !l.trim().is_empty()) {
            let p = dir.path().join(line);
            assert!(p.exists(), "列出来的路径必须真的存在：{line}");
        }
    }
}

#[cfg(test)]
mod os_junk_tests {
    use super::*;

    #[test]
    fn ds_store_is_os_junk_but_a_real_new_file_is_not() {
        // 系统垃圾：不算「未提交改动」
        assert!(is_os_junk(".DS_Store"));
        assert!(is_os_junk("workspaces/示例需求/.DS_Store"));
        assert!(is_os_junk("._README.md"));
        assert!(is_os_junk("Thumbs.db"));
        assert!(is_os_junk("sub/desktop.ini"));

        // 普通的未跟踪文件**不能**被放过 —— 新写的文档没 add，那确实是没提交
        assert!(!is_os_junk("workspaces/族/分支/需求文档.md"));
        assert!(!is_os_junk("notes.txt"));
        // 名字里带 DS_Store 但不是它，也不算垃圾
        assert!(!is_os_junk("my.DS_Store.md"));
    }

    #[test]
    fn porcelain_path_handles_untracked_renames_and_quotes() {
        assert_eq!(porcelain_path("?? .DS_Store"), ".DS_Store");
        assert_eq!(
            porcelain_path("?? workspaces/a/b/.DS_Store"),
            "workspaces/a/b/.DS_Store"
        );
        assert_eq!(porcelain_path(" M src/main.rs"), "src/main.rs");
        // 重命名取新路径
        assert_eq!(porcelain_path("R  old.md -> new.md"), "new.md");
        // 含特殊字符时 git 会加引号
        assert_eq!(porcelain_path("?? \"a b/.DS_Store\""), "a b/.DS_Store");
    }
}

#[cfg(test)]
mod timeout_tests {
    use super::*;

    /// 超时必须**真的把进程杀掉并报错** —— 这是「永远 loading」那个 BUG 的根因。
    ///
    /// 用一个必定挂住的 git 命令来验：`git hash-object --stdin` 会一直等标准输入，
    /// 我们给它一个**永远不关闭**的管道，它就一直不退出。git 本身在测试环境里必有。
    #[test]
    fn a_hanging_command_is_killed_and_reported() {
        // 造一个**必定挂住**的子进程：git 等标准输入，而管道我们一直不关。
        // （不能走 run_capped —— 它会强制把 stdin 设成 null，那正是防挂死的设计之一。）
        let mut child = Command::new("git")
            .args(["hash-object", "--stdin"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("git 可执行");

        let started = Instant::now();
        let err = wait_capped(&mut child, "git hash-object", Duration::from_millis(600))
            .expect_err("挂住的进程应当超时报错，而不是一直等下去");

        let msg = err.to_string();
        assert!(msg.contains("超过"), "要说清是超时: {msg}");
        assert!(msg.contains("连不上远程"), "要给能照着做的下一步: {msg}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "应当很快返回，不能真等到 180 秒"
        );
    }

    /// **每一条 git 命令都要带那两个 `-c` 覆盖** —— 包括 `clone`。
    ///
    /// 实测（Windows）：
    /// - 不带 `core.longpaths` 的 `clone` **退出码 0**，却漏掉长路径文件（静默残废）
    /// - 不带 `core.longpaths` 的 `worktree add` 报
    ///   `Could not reset index file to revision 'HEAD'`（看不出跟长度有关）
    ///
    /// 所以这里钉住"两个开关都在、clone 也走同一份常量"，防止哪天改一处漂一处。
    #[test]
    fn every_git_command_carries_the_common_flags() {
        assert!(
            COMMON_FLAGS.contains(&"core.quotePath=false"),
            "非 ASCII 路径不许被转义"
        );
        assert!(
            COMMON_FLAGS.contains(&"core.longpaths=true"),
            "Windows 长路径必须打开"
        );

        // clone 那条**不经过** `make_cmd`，是单独拼的 —— 正是它最容易漏
        let src = include_str!("git.rs");
        let clone_part = src
            .split("pub fn clone_with_token")
            .nth(1)
            .expect("找不到 clone_with_token")
            .split("\n}")
            .next()
            .unwrap();
        assert!(
            clone_part.contains("COMMON_FLAGS"),
            "clone 也必须带这两个开关 —— 不带的话它会**静默**留下没检出全的仓库"
        );
    }

    /// **Windows 的 260 字符上限**：这条判定是其他人那台机器上"worktree 怎么都建不出来"
    /// 的答案 —— 仓库里有文件的完整路径太长，checkout 写不出来，
    /// 而 git 只说 `Could not reset index file to revision 'HEAD'`，看不出跟长度有关。
    #[test]
    fn detects_paths_that_break_the_windows_260_limit() {
        // 真实的肇事路径形如 `docs/x-turbo/design/后端技术方案-H5…下架.md`，
        // **相对路径 256 字符**（这里按同样的量级造一条：200 字符 + ".md"）
        let long = format!("docs/x-turbo/design/{}.md", "后端技术方案-".repeat(25));
        let prefix =
            r"D:\gws\hub\my-hub\ws\feature-20260818-ca-mobile-vault-v3\demo-app"
                .len();
        let paths = vec!["README.md".to_string(), long.clone()];

        let over = paths_over_windows_limit(&paths, prefix, 3);
        assert_eq!(over.len(), 1, "只该报出那一条：{over:?}");
        assert_eq!(over[0].0, long);
        assert!(over[0].1 > 260, "算出来的完整长度要超过 260：{}", over[0].1);

        // 前缀短一点（更浅的 hub 目录）→ 不再超限
        let short_prefix = r"D:\gws".len();
        assert!(
            paths_over_windows_limit(&paths, short_prefix, 3).is_empty(),
            "hub 放浅一点就不该报"
        );

        // 按长度倒序、受 limit 限制（诊断信息不该刷屏）
        let many: Vec<String> = (0..10)
            .map(|i| format!("{}/f{i}.md", "d".repeat(250)))
            .collect();
        let over = paths_over_windows_limit(&many, 40, 3);
        assert_eq!(over.len(), 3);
        assert!(
            over[0].1 >= over[1].1 && over[1].1 >= over[2].1,
            "要按长度倒序"
        );
    }

    /// 正常结束的命令不受影响（输出要原样拿到）。
    #[test]
    fn a_normal_command_still_returns_its_output() {
        let mut cmd = Command::new("git");
        cmd.args(["--version"]);
        let out = run_capped(&mut cmd, "git --version", NET_TIMEOUT).unwrap();
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("git version"));
    }

    // ── 提交身份（跟着 hub 走，跟 git 全局配置解耦）─────────────────────

    /// 读某个仓库 local config 里的一个键（没有就是 `None`）。
    fn local_config(repo: &Path, key: &str) -> Option<String> {
        let out = Command::new("git")
            .current_dir(repo)
            .args(["config", "--local", "--get", key])
            .output()
            .expect("git 可执行");
        if !out.status.success() {
            return None;
        }
        let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!v.is_empty()).then_some(v)
    }

    fn init_repo(dir: &Path) {
        Command::new("git")
            .current_dir(dir)
            .args(["init", "-q", "-b", "main"])
            .output()
            .expect("git 可执行");
    }

    #[test]
    fn identity_is_written_and_cleared_in_local_config() {
        let dir = tempfile::TempDir::new().unwrap();
        init_repo(dir.path());

        let id = crate::hub::GitIdentity {
            author_name: "张三".into(),
            author_email: "zhangsan@example.com".into(),
        };
        apply_identity(dir.path(), &id).unwrap();
        assert_eq!(local_config(dir.path(), "user.name").as_deref(), Some("张三"));
        assert_eq!(
            local_config(dir.path(), "user.email").as_deref(),
            Some("zhangsan@example.com")
        );

        // 清掉身份时**必须把 local config 里的旧值也删掉** ——
        // 否则之前设过的身份会继续生效，用户改了配置却看不到变化
        apply_identity(dir.path(), &crate::hub::GitIdentity::default()).unwrap();
        assert_eq!(local_config(dir.path(), "user.name"), None);
        assert_eq!(local_config(dir.path(), "user.email"), None);
    }

    #[test]
    fn half_filled_identity_is_not_written() {
        // 只填一半不算「配了身份」（HubConfig::validate 也会拦），这里确保
        // 它不会被写成一个半截的 local config
        let dir = tempfile::TempDir::new().unwrap();
        init_repo(dir.path());
        let id = crate::hub::GitIdentity {
            author_name: "张三".into(),
            author_email: String::new(),
        };
        assert!(!id.is_set());
        apply_identity(dir.path(), &id).unwrap();
        assert_eq!(local_config(dir.path(), "user.name"), None);
    }

    /// **本方案的核心假设**：worktree 与主 clone 共享同一份 local config，
    /// 所以只在主 clone 上设一次就够了。
    ///
    /// 这条一旦不成立（比如 git 改了 worktree config 的行为、或将来开了
    /// `extensions.worktreeConfig`），`Hub::sync_identity_to_repos` 就只覆盖
    /// 了主 clone，用户在 `ws/` 里手工提交会**用错身份**——所以要有测试钉住。
    #[test]
    fn worktrees_inherit_the_main_clone_identity() {
        let dir = tempfile::TempDir::new().unwrap();
        let main = dir.path().join("main");
        std::fs::create_dir_all(&main).unwrap();
        init_repo(&main);

        // 造一个初始提交，worktree add 需要 HEAD
        Command::new("git")
            .current_dir(&main)
            .args([
                "-c", "user.email=t@t", "-c", "user.name=t",
                "commit", "-q", "--allow-empty", "-m", "init",
            ])
            .output()
            .expect("git 可执行");

        let id = crate::hub::GitIdentity {
            author_name: "张三".into(),
            author_email: "zhangsan@example.com".into(),
        };
        apply_identity(&main, &id).unwrap();

        // 挂一个 worktree（模拟 hub 里的 ws/<分支>/<模块>）
        let wt = dir.path().join("wt");
        let out = Command::new("git")
            .current_dir(&main)
            .args(["worktree", "add", "-q", "-b", "feature", wt.to_str().unwrap()])
            .output()
            .expect("git 可执行");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

        assert_eq!(
            local_config(&wt, "user.name").as_deref(),
            Some("张三"),
            "worktree 应当继承主 clone 的提交身份"
        );
        assert_eq!(
            local_config(&wt, "user.email").as_deref(),
            Some("zhangsan@example.com")
        );
    }
}
