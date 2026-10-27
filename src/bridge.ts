//! 与 Rust 命令层的桥接。
//!
//! 这里是前端**唯一**碰 `invoke` 的地方。其它模块一律通过本文件调用后端——
//! 这样命令名、参数名一旦写错，改一处即可，而不会散落在几十个组件里。

import { invoke } from "@tauri-apps/api/core";

import { busy } from "./busy";
import { notifyError } from "./lib/notify";
import type {
  AttachOutcome,
  CloseOutcome,
  QuickLink,
  RepoImportResult,
  ShareImportResult,
  SharedRepo,
  SharedRequirement,
  CommandError,
  DocFile,
  DocSyncOutcome,
  DocTypeView,
  DocView,
  DoneOutcome,
  EditorOption,
  EntryInfo,
  EnvAddInput,
  EnvModuleCheck,
  EnvRow,
  Family,
  FilterInput,
  GetInput,
  GetOutcome,
  GitlabProject,
  GitlabStatus,
  GitlabUser,
  HubInfo,
  InitInput,
  MergeOutcome,
  MetaPatch,
  MrDefaults,
  MrInput,
  MrPlan,
  NewInput,
  NewResult,
  PaseoSession,
  PullOutcome,
  PushOutcome,
  RemoteProbe,
  RepoAddOutcome,
  RepoOutcome,
  RepoRow,
  ReviewerInfo,
  SyncMainOutcome,
  SyncOutcome,
  AddressInput,
  BaseResolution,
  BranchCheck,
  CommitInfo,
  FileChange,
  FileDiff,
  Link,
  MatrixRow,
  PruneOutcome,
  PrunePlan,
  RenameOutcome,
  RenamePlan,
  ResetOutcome,
  ResetPlan,
  RevertOutcome,
  RevertPlan,
  TerminalOption,
  ToolVersions,
  ToolSyncOutcome,
  UpdateInfo,
  UpdateInstallOutcome,
  WorkspaceMeta,
  WsDetail,
  WsShell,
  WsRow,
} from "./types";

/** 把后端抛出的东西归一成可读文本。
 *
 * Rust 侧 `Err(CommandError)` 会被 Tauri 序列化成对象，不是 Error 实例——
 * 直接 `String(e)` 会得到 `[object Object]`，是最容易漏的一处。
 */
export function errText(e: unknown): string {
  if (e && typeof e === "object") {
    const ce = e as Partial<CommandError>;
    if (typeof ce.message === "string" && ce.message) return ce.message;
  }
  return String(e);
}

/** 取结构化预检明细（仅 `kind === 'precheck'` 时有）。 */
export function precheckOf(e: unknown): CommandError | null {
  if (e && typeof e === "object") {
    const ce = e as CommandError;
    if (ce.kind === "precheck") return ce;
  }
  return null;
}

/** 数据刷新类调用：在途期间全局忙碌计数 +1，界面据此挡掉重复操作。 */
function tracked<T>(p: Promise<T>): Promise<T> {
  busy.count++;
  return p.finally(() => {
    busy.count--;
  });
}

/** 不参与忙碌计数的调用（平台能力、轻量查询）。 */
function plain<T>(p: Promise<T>): Promise<T> {
  return p;
}

/**
 * 「点了没反应」的一类命令：打开目录 / 文件 / 链接 / 终端。
 *
 * 它们**本身没有可见结果** —— 成功了界面也不变，所以失败必须说话。
 * 旧代码直接把 Promise 丢在 `@click` 里，报错进了 unhandledrejection：
 * 用户点「文件管理」，什么都没发生，连个报错都看不到（实测反馈）。
 *
 * 现在统一在这里接住并弹提示。**不 rethrow**：调用点都在模板里，
 * 没有 catch 的地方，rethrow 只会再造一个 unhandledrejection。
 */
function visible<T>(p: Promise<T>): Promise<T | undefined> {
  return p.catch((e: unknown) => {
    notifyError(errText(e));
    return undefined;
  });
}

// ══ hub ═══════════════════════════════════════════════════════════════

export const locateHub = (start: string) => plain(invoke<string | null>("locate_hub", { start }));
export const openHub = (path: string) => tracked(invoke<HubInfo>("open_hub", { path }));
export const initHub = (input: InitInput) => tracked(invoke<HubInfo>("init_hub", { input }));
/**
 * **页面打开时的"读一下"用 `plain`，不要 `tracked`。**
 *
 * `tracked` 会弹那个**模态进度遮罩** —— 对"加仓库 / 同步 / 合并"这类慢操作是对的，
 * 但每次切到仓库页、文档页都闪一下就很烦（实测反馈：切页面总会 loading 一下）。
 * 下面这几个命令都是**纯本地读**，几十毫秒就回来，不需要遮罩。
 */
export const hubInfo = () => plain(invoke<HubInfo>("hub_info_of"));
export const closeHub = () => plain(invoke<void>("close_hub"));
/** 改 hub 的显示名（只是标签，不影响路径）。 */
export const hubRename = (name: string) => tracked(invoke<HubInfo>("hub_rename", { name }));
/** 设置快捷入口（空数组 = 界面隐藏快捷入口）。 */
export const hubSetQuickLinks = (links: QuickLink[]) =>
  tracked(invoke<HubInfo>("hub_set_quick_links", { links }));
/** 读 git 全局身份，给「新建 hub」做预填（读不到返回 [null, null]）。 */
export const globalGitIdentity = () =>
  plain(invoke<[string | null, string | null]>("global_git_identity"));
/** 设置提交身份（两个都留空 = 清除，回到 git 全局配置）。 */
export const hubSetIdentity = (name: string, email: string) =>
  tracked(invoke<HubInfo>("hub_set_identity", { name, email }));

// ══ 仓库 / 环境 ═══════════════════════════════════════════════════════

export const repoList = () => plain(invoke<RepoRow[]>("repo_list"));
/** 慢：clone。 */
export const repoAdd = (urls: string[]) =>
  tracked(invoke<RepoAddOutcome>("repo_add", { urls }));
export const repoRemove = (names: string[]) => tracked(invoke<RepoOutcome[]>("repo_remove", { names }));

export const envList = () => plain(invoke<EnvRow[]>("env_list"));
export const envAdd = (input: EnvAddInput) => tracked(invoke<SyncOutcome>("env_add", { input }));
/** 改环境：标识（会连带改目录名、迁地址键）与显示名。 */
export const envUpdate = (oldId: string, newId: string, label: string) =>
  tracked(invoke<void>("env_update", { oldId, newId, label }));

export const envRemove = (ids: string[]) => tracked(invoke<void>("env_remove", { ids }));
export const envSetOverrides = (id: string, overrides: [string, string][]) =>
  tracked(invoke<void>("env_set_overrides", { id, overrides }));
/** 保存前校验分支是否存在（填错分支名的话，下次同步才报错，离现场太远）。 */
export const envCheckBranches = (overrides: [string, string][]) =>
  plain(invoke<BranchCheck[]>("env_check_branches", { overrides }));

// ══ 工作区 ════════════════════════════════════════════════════════════

export const wsList = () => plain(invoke<WsRow[]>("ws_list"));
/** 慢：建 worktree + 可能克隆 doc-hub。 */
export const wsNew = (input: NewInput) => tracked(invoke<NewResult>("ws_new", { input }));
/** 慢：扫描所有仓库。 */
export const wsGet = (input: GetInput) => tracked(invoke<GetOutcome>("ws_get", { input }));
/**
 * 需求详情的**第一屏**：只要元数据 + 模块名，**不打 git**（毫秒级）。
 *
 * 界面先拿它把页面画出来，数字再等 `wsDetail` 回来填 —— 否则进需求要盯着
 * 转圈等一两秒（实测反馈）。等待期间那几格显示「…」。
 */
export const wsShell = (branch: string) => plain(invoke<WsShell>("ws_shell", { branch }));
/** 需求详情的**数字那一半**（每个模块要起几个 git 进程，慢）。 */
export const wsDetail = (branch: string, filter: FilterInput = {}) =>
  plain(invoke<WsDetail>("ws_detail", { branch, filter }));
/** 「与基线」：解析基线链里日期最新、且该模块存在的 ref（会 fetch）。 */
export const wsBaseline = (branch: string, module: string) =>
  plain(invoke<BaseResolution>("ws_baseline", { branch, module }));
export const wsAddModules = (branch: string, modules: string[]) =>
  tracked(invoke<RepoOutcome[]>("ws_add_modules", { branch, modules }));
export const wsDropModule = (branch: string, module: string, force: boolean) =>
  tracked(invoke<unknown>("ws_drop_module", { branch, module, force }));
export const wsDropMissing = (branch: string) =>
  tracked(invoke<{ cleaned: string[]; kept: [string, string][] }>("ws_drop_missing", { branch }));
export const wsRemove = (branch: string, force: boolean) =>
  tracked(invoke<{ branch: string; removed_modules: string[]; risks: { module: string; reason: string }[]; forced: boolean }>("ws_remove", { branch, force }));
export const wsClean = (apply: boolean) =>
  tracked(invoke<{ leftovers: string[]; removed: string[]; failed: [string, string][] }>("ws_clean", { apply }));
/** 把某个需求的配置渲染成一段可粘贴的文本（发给别人）。 */
export const wsShare = (branch: string) => plain(invoke<string>("ws_share", { branch }));
/** 解析别人发来的配置（纯本地，不建任何东西）。 */
export const wsShareParse = (text: string) =>
  plain(invoke<SharedRequirement>("ws_share_parse", { text }));
/** 按别人发来的配置把需求建出来（慢：要克隆缺的仓库、各仓库要 fetch）。 */
export const wsShareImport = (shared: SharedRequirement) =>
  tracked(invoke<ShareImportResult>("ws_share_import", { shared }));

export const wsPatchMeta = (branch: string, patch: MetaPatch) =>
  tracked(
    invoke<{ meta: WorkspaceMeta; notes: string[] }>("ws_patch_meta", { branch, patch }),
  );

// ══ 同步与合并 ════════════════════════════════════════════════════════

export const wsPush = (branch: string, filter: FilterInput = {}) =>
  tracked(invoke<PushOutcome>("ws_push", { branch, filter }));
export const wsPull = (branch: string, rebase: boolean, filter: FilterInput = {}) =>
  tracked(invoke<PullOutcome>("ws_pull", { branch, rebase, filter }));
export const wsMerge = (
  branch: string,
  env: string,
  push: boolean,
  force: boolean,
  filter: FilterInput = {},
) => tracked(invoke<MergeOutcome>("ws_merge", { branch, env, push, force, filter }));
export const wsSyncMain = (branch: string, from: string[], filter: FilterInput = {}) =>
  tracked(invoke<SyncMainOutcome>("ws_sync_main", { branch, input: { from, ...filter } }));
/** 合并后检查：合进环境了什么、推没推、有没有冲突。 */
/** 把已合好但没推的环境分支补推上远程。返回 `[模块, 失败原因][]`（空原因=成功）。 */
export const wsPushToEnv = (branch: string, env: string, modules: string[]) =>
  tracked(invoke<[string, string][]>("env_push", { branch, env, modules }));

export const envMergeCheck = (branch: string, env: string) =>
  plain(invoke<EnvModuleCheck[]>("env_merge_check", { branch, env }));

export const wsDone = (branch: string, filter: FilterInput = {}) =>
  plain(invoke<DoneOutcome>("ws_done", { branch, filter }));
/** 慢：22 个仓库 fetch + reset。**界面不用它**，界面走下面两个有范围的。 */
export const syncAll = () => tracked(invoke<SyncOutcome>("sync_all"));
/** 只同步仓库主干（仓库页用）。 */
export const syncRepos = (filter: FilterInput = {}) =>
  tracked(invoke<SyncOutcome>("sync_repos", { filter }));
/** 只同步某个环境（环境页用）。 */
export const syncEnv = (env: string, filter: FilterInput = {}) =>
  tracked(invoke<SyncOutcome>("sync_env", { env, filter }));
export const envBranchOf = (env: string, module: string) =>
  plain(invoke<string>("env_branch_of", { env, module }));
/** 需求 #8：异步探测远程是否有他人推送。 */
export const wsProbeRemote = (branch: string) =>
  plain(invoke<RemoteProbe[]>("ws_probe_remote", { branch }));

// ══ MR ════════════════════════════════════════════════════════════════

export const mrDefaults = (branch: string) =>
  plain(invoke<MrDefaults>("mr_defaults", { branch }));
export const mrPlan = (input: MrInput) => tracked(invoke<MrPlan>("mr_plan", { input }));
export const mrCreate = (input: MrInput) => tracked(invoke<MrPlan>("mr_create", { input }));
export const mrClose = (branch: string, filter: FilterInput = {}) =>
  tracked(invoke<CloseOutcome>("mr_close", { branch, filter }));
export const mrReviewers = (branch?: string) =>
  plain(invoke<ReviewerInfo>("mr_reviewers", { branch: branch ?? null }));
export const mrSaveReviewers = (branch: string, front: string, back: string) =>
  tracked(invoke<void>("mr_save_reviewers", { branch, front, back }));

// ══ 文档 ══════════════════════════════════════════════════════════════

export const docView = (branch: string) => plain(invoke<DocView>("doc_view", { branch }));
export const docAttach = (branch: string, family?: string | null) =>
  tracked(invoke<AttachOutcome>("doc_attach", { branch, family: family ?? null }));
/** 把文档从需求上摘掉（简单需求不需要文档时）。不影响 doc-hub 上已有的内容。 */
export const docDetach = (branch: string) => tracked(invoke<boolean>("doc_detach", { branch }));
export const docTypes = () => plain(invoke<DocTypeView[]>("doc_types"));
export const docNew = (branch: string, kind: string) =>
  tracked(invoke<string>("doc_new", { branch, kind }));
export const docStatus = (branch: string) => plain(invoke<string[]>("doc_status", { branch }));
export const docSync = (branch: string, message: string, push: boolean) =>
  tracked(invoke<DocSyncOutcome>("doc_sync", { branch, message, push }));
/** 改期次标签（V1 / V2 / BUGFIX…）。自动编号猜不出人的意图。 */
/** 文档分支相对 doc-hub 主干的领先提交数（0 = 已并入主干）。 */
export const docAheadOfMain = (branch: string) =>
  plain(invoke<number>("doc_ahead_of_main", { branch }));

export const docSetLabel = (branch: string, label: string) =>
  plain(invoke<string>("doc_set_label", { branch, label }));

export const docFamilies = () => plain(invoke<Family[]>("doc_families"));
export const dochubEnsure = () => tracked(invoke<RepoAddOutcome>("dochub_ensure"));

// ══ 危险操作（都是 plan + apply 两步）═════════════════════════════════
//
// **必须先调 plan 把影响面摆给人看**，人确认后才 apply。plan 是纯只读的。

export const wsRevertPlan = (branch: string, filter: FilterInput = {}) =>
  plain(invoke<RevertPlan>("ws_revert_plan", { branch, filter }));
export const wsRevertApply = (branch: string, filter: FilterInput = {}) =>
  tracked(invoke<RevertOutcome>("ws_revert_apply", { branch, filter }));
export const wsResetPlan = (branch: string, filter: FilterInput = {}) =>
  plain(invoke<ResetPlan>("ws_reset_plan", { branch, filter }));
export const wsResetApply = (branch: string, filter: FilterInput = {}) =>
  tracked(invoke<ResetOutcome>("ws_reset_apply", { branch, filter }));
export const branchPrunePlan = (branch: string, filter: FilterInput = {}) =>
  plain(invoke<PrunePlan>("branch_prune_plan", { branch, filter }));
export const branchPruneApply = (branch: string, filter: FilterInput = {}) =>
  tracked(invoke<PruneOutcome>("branch_prune_apply", { branch, filter }));
export const branchRenamePlan = (branch: string, newBranch: string, filter: FilterInput = {}) =>
  plain(invoke<RenamePlan>("branch_rename_plan", { branch, newBranch, filter }));
export const branchRenameApply = (branch: string, newBranch: string, filter: FilterInput = {}) =>
  tracked(invoke<RenameOutcome>("branch_rename_apply", { branch, newBranch, filter }));

// ══ 文件浏览与对比（需求 #18）═════════════════════════════════════════

/** 列出某个工作目录相对主干的改动文件（带 A/M/D 状态）。
 *  `repo` 只用来探测主干名（各仓库 main/master 混用），文档库传 `doc-hub`。 */
export const browseChanges = (dir: string, repo: string, base?: string | null) =>
  plain(invoke<FileChange[]>("browse_changes", { dir, repo, base: base ?? null }));
/** 读工作目录里某个文件的内容。 */
export const browseRead = (dir: string, path: string) =>
  plain(invoke<string>("browse_read", { dir, path }));
/** 某文件相对主干的 diff（纯文本，前端自己着色）。 */
export const browseDiff = (dir: string, repo: string, path: string, base?: string | null) =>
  plain(invoke<FileDiff>("browse_diff", { dir, repo, path, base: base ?? null }));

/** 本地相对某基准的 `(领先, 落后)` 提交数 —— 让差异的方向看得见。 */
export const browseAheadBehind = (dir: string, repo: string, base: string | null) =>
  plain(invoke<[number, number]>("browse_ahead_behind", { dir, repo, base }));

/** 最近 N 条提交（时间 / 作者 / 说明）。 */
export const browseCommits = (dir: string, limit = 20) =>
  plain(invoke<CommitInfo[]>("browse_commits", { dir, limit }));
/** 某个提交改了哪些文件。 */
export const browseCommitFiles = (dir: string, sha: string) =>
  plain(invoke<FileChange[]>("browse_commit_files", { dir, sha }));
/** 某文件在某提交里的改动 + 该提交时的内容。 */
export const browseCommitDiff = (dir: string, sha: string, path: string) =>
  plain(invoke<FileDiff>("browse_commit_diff", { dir, sha, path }));

// ══ 发布地址 ═══════════════════════════════════════════════════════════

/** 把某仓库的配置渲染成一段可粘贴的文本（发给别人）。 */
export const repoShare = (repo: string) => plain(invoke<string>("repo_share", { repo }));
/** 解析别人发来的仓库配置（纯本地，不动任何东西）。 */
export const repoShareParse = (text: string) =>
  plain(invoke<SharedRepo>("repo_share_parse", { text }));
/** 按别人发来的配置导入仓库（慢：缺的仓库要克隆）。 */
export const repoShareImport = (shared: SharedRepo) =>
  tracked(invoke<RepoImportResult>("repo_share_import", { shared }));

export const deployMatrix = () => plain(invoke<MatrixRow[]>("deploy_matrix"));
export const deploySetAddress = (input: AddressInput) =>
  tracked(invoke<void>("deploy_set_address", { input }));
export const deploySetGitlab = (repo: string, url: string) =>
  tracked(invoke<void>("deploy_set_gitlab", { repo, url }));
/** 仓库的自定义备注 / 链接（tapd、飞书、域名这类没有固定结构的信息）。 */
export const deploySetRepoInfo = (input: { repo: string; notes?: string; links?: Link[] }) =>
  tracked(invoke<void>("deploy_set_repo_info", { input }));

export const deploySetMany = (inputs: AddressInput[]) =>
  tracked(invoke<void>("deploy_set_many", { inputs }));

// ══ 自更新 ═══════════════════════════════════════════════════════════

export const checkUpdate = () => plain(invoke<UpdateInfo>("check_update"));
/** 一键更新：下载 + 校验 + 装（macOS 替换 .app、Windows 跑安装包、Linux 交给人装）。 */
export const updateInstall = () => tracked(invoke<UpdateInstallOutcome>("update_install"));
/** 装完退出应用（重启助手会把它拉起来）。 */
export const restartApp = () => plain(invoke<void>("restart_app"));
/** 打开 Releases 页。
 *
 * 用 `visible` 而不是 `plain`：它**会失败**（`platform::open_url` 出问题时
 * 后端返回 Err），而调用点在模板里没有 catch —— 用 `plain` 的话失败就进
 * unhandledrejection，点了完全没反应（跟 `openUrl` 那类「点了没反应」同源）。 */
export const openReleases = () => visible(invoke<void>("open_releases"));
/** 桌面端 / 命令行 / Skills 三个版本（只读探测，不联网）。 */
export const toolVersions = () => plain(invoke<ToolVersions>("tool_versions"));
/** 把桌面端自带的同版本 CLI 与 skills 覆盖到本机（设置页手动触发）。 */
export const syncTools = () => tracked(invoke<ToolSyncOutcome>("sync_tools"));

// ══ Paseo ═════════════════════════════════════════════════════════════

/** 可选的 Paseo provider（`paseo provider ls` 里可用且启用的，保持原顺序）。 */
export const paseoProviders = () => plain(invoke<string[]>("paseo_providers"));
/** 在 hub 根目录开一个 AI 会话（cwd 是根目录，AI 才读得到约定与主干代码）。 */
export const paseoOpenSession = (branch: string, provider: string) =>
  tracked(invoke<PaseoSession>("paseo_open_session", { branch, provider }));
/** 开一个不绑定需求的会话：建需求前聊需求、直接查线上日志。 */
export const paseoOpenFreeSession = (title: string, provider: string) =>
  tracked(invoke<PaseoSession>("paseo_open_free_session", { title, provider }));

// ══ GitLab ════════════════════════════════════════════════════════════

export const gitlabStatus = () => plain(invoke<GitlabStatus>("gitlab_status"));
/**
 * 用给定的地址 + token 试连一次（建 hub 前先验证）。
 *
 * 传了 `dochubUrl` 就一并校验文档库（同域名 + 存在 + 有权限）——
 * 否则会出现「测试连接说成功，创建时却因文档库失败」，而文档库地址建完不可改。
 */
export const gitlabProbe = (baseUrl: string, token: string, dochubUrl?: string) =>
  plain(invoke<[string, string]>("gitlab_probe", { baseUrl, token, dochubUrl }));
/** 给已存在但还没绑定的 hub 补绑 GitLab（已绑定过的会被拒绝）。 */
export const hubBindGitlab = (url: string, doc: string, token: string) =>
  tracked(invoke<void>("hub_bind_gitlab", { url, doc, token }));
export const gitlabLogin = (value: string) => tracked(invoke<GitlabStatus>("gitlab_login", { value }));
/** 只保存，不验证（验证要打网络，可能卡很久）。 */
export const gitlabSaveToken = (value: string) => plain(invoke<void>("gitlab_save_token", { value }));
export const gitlabLogout = () => tracked(invoke<GitlabStatus>("gitlab_logout"));
export const gitlabSearchProjects = (query: string, limit?: number) =>
  plain(invoke<GitlabProject[]>("gitlab_search_projects", { query, limit: limit ?? null }));
export const gitlabSearchUsers = (query: string, limit?: number) =>
  plain(invoke<GitlabUser[]>("gitlab_search_users", { query, limit: limit ?? null }));
export const gitlabCloneProjects = (urls: string[]) =>
  tracked(invoke<RepoAddOutcome>("gitlab_clone_projects", { urls }));

// ══ 平台 ══════════════════════════════════════════════════════════════

export const openInFinder = (path: string) =>
  visible(invoke<void>("open_in_finder", { path }));
export const openPath = (path: string) => visible(invoke<void>("open_path", { path }));
export const openUrl = (url: string) => visible(invoke<void>("open_url", { url }));
export const copyText = (text: string) => plain(invoke<void>("copy_text", { text }));
export const openInTerminal = (path: string, terminal: string | null) =>
  visible(invoke<void>("open_in_terminal", { path, terminal }));
export const terminalOptions = () => plain(invoke<TerminalOption[]>("terminal_options"));
/** 用设置里的编辑器打开目录（默认 IDEA；已开着的实例会被切到前台）。 */
export const openInEditor = (path: string, editor: string | null, customCommand: string | null) =>
  visible(invoke<void>("open_in_editor", { path, editor, customCommand }));
export const editorOptions = () => plain(invoke<EditorOption[]>("editor_options"));
export const readTextFile = (path: string) => plain(invoke<string>("read_text_file", { path }));
export const listDir = (path: string) => plain(invoke<string[]>("list_dir", { path }));
export const listEntries = (path: string) => plain(invoke<EntryInfo[]>("list_entries", { path }));

export type { DocFile };