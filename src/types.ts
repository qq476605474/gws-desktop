//! 与 Rust 侧结构体一一对应的类型。
//!
//! **字段名一律 snake_case**，与 serde 的默认命名一致。加 `rename_all` 转换
//! 看着更「JS 味」，但那样每个字段都有两套名字（前端的、后端的），任何一处
//! 写错都只在运行时暴露成 `undefined`。照抄后端命名，编译器就能帮忙查。

/** 命令错误。`kind === 'precheck'` 时带结构化明细，界面要分类展示。 */
export interface CommandError {
  message: string;
  kind: 'precheck' | 'other';
  precheck?: PrecheckFailure;
}

export interface PrecheckFailure {
  missing_repos: string[];
  remote_branch_exists: string[];
  local_branch_exists: string[];
  base_missing: [string, string][];
}

// ── hub ──────────────────────────────────────────────────────────────

export interface HubInfo {
  root: string;
  name: string;
  /** `[环境标识, 显示名]` */
  envs: [string, string][];
  repo_count: number;
  workspace_count: number;
  /** 文档库在 hub 里的目录名（从配置的 URL 推导），前端别写死 `doc-hub`。 */
  dochub_name: string;
  dochub_ready: boolean;
  dochub_branch: string | null;
  /** 新建需求时自动拼接的前缀与日期位数（6/8），界面预览要用真实配置。 */
  branch_prefix: string;
  branch_date_digits: number;
  /** 打开时做过的目录迁移/清理说明（空 = 什么都没动）。 */
  migration_notes: string[];
  /** 用户自己配的快捷入口。**空数组 = 界面整块隐藏快捷入口**。 */
  quick_links: QuickLink[];
  /** 这个 hub 的提交身份（空 = 用 git 全局配置）。 */
  git_name: string;
  git_email: string;
}

/** 一个快捷入口：显示名 + 地址。用户自己配，开源版不预设。 */
export interface QuickLink {
  label: string;
  url: string;
}

export interface InitInput {
  path: string;
  name?: string | null;
  envs?: string[] | null;
  force?: boolean;
  /** GitLab 地址（**建 hub 时固定，之后不可改**）。三项要么都给、要么都不给。 */
  gitlab_url?: string | null;
  gitlab_token?: string | null;
  /** 文档库仓库地址（**建 hub 时固定，之后不可改**）。 */
  dochub_url?: string | null;
  /** 提交身份（可留空 = 用 git 全局配置）。**建 hub 时填，之后随时可改。** */
  git_name?: string | null;
  git_email?: string | null;
}

// ── 仓库 / 环境 ───────────────────────────────────────────────────────

export interface RepoRow {
  name: string;
  main_branch: string;
  /** 被哪些**需求**引用（`ws/<分支>`）。**只有这一类会拦住移除**。 */
  usages: string[];
  /** 在哪些**环境**里有检出（`envs/<标识>`）。移除时会一并清掉。 */
  env_usages: string[];
  remote_url: string;
  /** 是不是文档库这个特殊仓库（在 repos/ 下、能同步，但不能当模块加、不能移除）。 */
  is_doc: boolean;
}

export type Outcome =
  | 'Ok'
  | { Skipped: string }
  | { Failed: string };

export interface RepoOutcome {
  name: string;
  outcome: Outcome;
}

/**
 * 「添加仓库」的结果。
 *
 * 不只是仓库那一栏：加完还会**自动把环境补齐**（新增仓库在每个环境里都缺一份
 * 检出），那一步的结果也得能显示出来，否则失败了就是「悄悄没做」。
 */
export interface RepoAddOutcome {
  repos: RepoOutcome[];
  /** 补建出来的环境检出：`[环境标识, 仓库名, 分支名]`。 */
  env_created: [string, string, string][];
  /** 环境补齐失败：`[环境标识/仓库, 原因]`。**不影响仓库添加本身**。 */
  env_failures: [string, string][];
}

/** Rust 侧 `Outcome` 是带数据的枚举，serde 序列化成 `{"Skipped": "原因"}` 这种形态。 */
export function outcomeKind(o: Outcome): 'ok' | 'skipped' | 'failed' {
  if (o === 'Ok') return 'ok';
  if (typeof o === 'object' && 'Skipped' in o) return 'skipped';
  return 'failed';
}

export function outcomeReason(o: Outcome): string {
  if (o === 'Ok') return '';
  if (typeof o === 'object' && 'Skipped' in o) return o.Skipped;
  if (typeof o === 'object' && 'Failed' in o) return o.Failed;
  return '';
}

export interface EnvRow {
  id: string;
  label: string;
  overrides: [string, string][];
  effective: [string, string][];
}

export interface EnvAddInput {
  id: string;
  label?: string | null;
  overrides?: [string, string][];
}

// ── 工作区 ────────────────────────────────────────────────────────────

export interface WsRow {
  branch: string;
  title: string;
  modules: number;
  missing_modules: number;
  created: string;
  /** 基线来源链，顺序即优先级；空数组 = 未设置（即主干兜底）。 */
  base: string[];
  doc_family: string | null;
}

export interface Link {
  label: string;
  url: string;
}

export interface DocRef {
  family: string;
  period: string;
}

export interface MrRecord {
  iid: number;
  url: string;
  state: string;
}

export interface WorkspaceMeta {
  schema: number;
  branch: string;
  title: string;
  prefix: string;
  created: string;
  base: string[];
  base_resolved: Record<string, string>;
  modules: string[];
  doc: DocRef | null;
  links: Link[];
  notes: string;
  merged: Record<string, string | null>;
  mr: Record<string, MrRecord>;
}

export interface ModuleStatus {
  module: string;
  present: boolean;
  dirty: number;
  pushed: boolean;
  ahead: number;
  behind: number;
  ahead_of_main: number | null;
  error: string | null;
}

export interface StatusReport {
  branch: string;
  title: string;
  modules: ModuleStatus[];
}

/** 文档（doc-hub）那一行的数字。跟模块状态一起给，免得前端拼两个请求。 */
export interface DocRowStatus {
  dirty: number;
  ahead_of_main: number;
  pushed: boolean;
  ahead: number;
  behind: number;
}

export interface WsDetail {
  meta: WorkspaceMeta;
  status: StatusReport;
  /** 没挂文档时为 null。 */
  doc: DocRowStatus | null;
}

/**
 * 需求详情的**第一屏**：只要有元数据与模块名，不含任何数字。
 *
 * 进需求时先拿它把页面画出来（毫秒级），各模块的三列数字再等 [`WsDetail`]
 * 回来填 —— 那三列要起十几个 git 进程，Windows 上一个 4 模块的需求要 1~2 秒。
 */
export interface ShellModule {
  name: string;
  /** 目录还在不在（后端只查文件系统，不跑 git）。 */
  present: boolean;
}

export interface WsShell {
  meta: WorkspaceMeta;
  /**
   * 元数据里登记的模块 + 目录是否还在。
   *
   * 刻意**不是**"只列存在的"：那样第二屏（数字）会多出几行来，表格当场长高、
   * 把下面的东西顶下去。行集合两屏必须一致。
   */
  modules: ShellModule[];
  /** 挂了文档库没有。 */
  doc: boolean;
}

export interface NewInput {
  name?: string | null;
  branch?: string | null;
  title?: string | null;
  modules: string[];
  base?: string[];
  prefix?: string | null;
  doc_family?: string | null;
  /** 建的时候就能填的备注与自定义信息。 */
  notes?: string | null;
  links?: Link[] | null;
}

export interface NewResult {
  branch: string;
  created: string[];
  base_resolved: Record<string, string>;
  base_notes: Record<string, string>;
  doc: AttachOutcome | null;
  doc_error: string | null;
}

export interface GetInput {
  branch: string;
  title?: string | null;
  doc_family?: string | null;
  notes?: string | null;
  links?: Link[] | null;
}

export interface GetOutcome {
  branch: string;
  modules: string[];
  reused: string[];
  not_involved: string[];
}

export interface MetaPatch {
  title?: string | null;
  base?: string[] | null;
  links?: Link[] | null;
  notes?: string | null;
  doc_family?: string | null;
}

export interface FilterInput {
  /** `null`/`undefined` = 不限制；空数组 = 一个都不选。 */
  only?: string[] | null;
  exclude?: string[] | null;
}

// ── 同步与合并 ────────────────────────────────────────────────────────

export interface PushOutcome {
  pushed: string[];
  dirty: [string, number][];
  behind: [string, number][];
  failed: [string, string][];
}

export interface PullOutcome {
  updated: [string, number][];
  skipped: [string, string][];
  dirty: [string, number][];
  conflicts: string[];
}

export interface MergeOutcome {
  env: string;
  stale: string[];
  pending: [string, number][];
  clean: string[];
  skipped: [string, string][];
  conflicts: string[];
  conflict_dirs: Record<string, string>;
  pushed: string[];
  push_failed: [string, string][];
  did_push: boolean;
  /** 模块 → **在这个环境里实际用的分支**（不是环境标识）。 */
  branches: Record<string, string>;
}

export interface SyncMainOutcome {
  from: string[];
  updated: [string, string][];
  conflicts: string[];
  restash_conflicts: string[];
  blocked: [string, string][];
  downgraded: [string, string][];
}

export interface SyncOutcome {
  repos_updated: string[];
  repo_failures: [string, string][];
  env_created: [string, string, string][];
  env_moved: [string, string, string][];
  env_failures: [string, string][];
}

export interface DoneOutcome {
  /** 已并入主干（相对主干没有多余提交）的模块。 */
  merged: string[];
  /** 尚未并入主干的模块及领先提交数（0 = 读不到差异）。 */
  unmerged: [string, number][];
}

/** 合并后检查：某模块在环境里的状态。 */
export interface EnvModuleCheck {
  module: string;
  /** 有未解决的冲突。 */
  conflict: boolean;
  conflict_files: string[];
  /** 远程还没有的本地提交数（>0 = 没推上去）。 */
  unpushed: number;
  behind: number;
  /** 合进环境的提交：`[短 sha, 说明]`。 */
  commits: [string, string][];
  /** 相对主干改了几个文件。 */
  files: number;
  /** 该模块在此环境实际检出的分支（前端拿它拼 `origin/<分支>` 做比对基准）。 */
  env_branch: string;
  error: string | null;
}

export interface RemoteProbe {
  module: string;
  ahead: number;
  behind: number;
  error: string | null;
}

// ── MR ────────────────────────────────────────────────────────────────

export interface MrRow {
  module: string;
  project: string;
  group: string;
  side: 'Backend' | 'Frontend';
  target_branch: string;
  files: number;
  reviewer: string;
  reviewer_name: string;
  reviewer_id: number;
  existing: MergeRequest | null;
  created: MergeRequest | null;
  error: string | null;
}

export interface MergeRequest {
  iid: number;
  web_url: string;
  state: string;
  source_branch: string;
  target_branch: string;
  title: string;
}

export interface Reviewers {
  frontend: string;
  backend: string;
  source: string;
}

export interface MrDefaults {
  title: string;
  description: string;
}

export interface MrPlan {
  branch: string;
  /** 实际提交的最终标题（用户可改，原样提交）。 */
  title: string;
  /** 实际提交的最终描述。 */
  description: string;
  reviewers: Reviewers;
  rows: MrRow[];
  skipped: [string, string][];
  registered: Record<string, number>;
  is_dry_run: boolean;
}

export interface MrInput {
  branch: string;
  title?: string | null;
  description?: string | null;
  reviewer?: string | null;
  frontend_reviewer?: string | null;
  backend_reviewer?: string | null;
  only?: string[] | null;
  exclude?: string[] | null;
}

export interface CloseOutcome {
  closed: [string, string][];
  skipped: [string, string][];
  failed: [string, string][];
}

export interface ReviewerInfo {
  frontend: string;
  backend: string;
  source: string;
}

// ── 文档 ──────────────────────────────────────────────────────────────

export interface PeriodMeta {
  schema: number;
  branch: string;
  family: string;
  name: string;
  label: string;
  date: string;
  owner: string;
  summary: string;
  repos: string[];
  links: Link[];
  notes: string;
}

export interface AttachOutcome {
  family: string;
  label: string;
  relative_dir: string;
  created: boolean;
  /**
   * 真的挂上了没有。
   *
   * 导入需求时是「只复用、不生成」：远程没有这一期的文档就**什么都不建**
   * （凭空生成的那几个文件会挡住之后的 `git pull`）。那种情况这里是 `false`
   * —— 是"这次不挂"，不是失败。
   */
  attached: boolean;
}

export interface DocFile {
  name: string;
  rel: string;
  abs: string;
}

export interface DocView {
  meta: PeriodMeta;
  dir: string;
  files: DocFile[];
  pending: string[];
  /** 空目录（相对期次目录，如 `sql`）—— 只有 files 的话它们在界面上看不见。 */
  empty_dirs: string[];
  /** 除 meta.json/CLAUDE.md/AGENTS.md 外有没有真正的文档。 */
  has_content: boolean;
}

export interface DocTypeView {
  key: string;
  file: string;
}

/** 期次 + 它在文档库里的状态。 */
export interface PeriodWithState extends PeriodMeta {
  /** 该期次分支**是否已并入 main**。true = 已归档的历史文档（只读）。 */
  merged: boolean;
  /** 除工具生成的脚手架外有没有真正的文档。 */
  has_content: boolean;
  /** 文档页能不能打开：主 clone 或需求工作区里有这一期目录。 */
  available: boolean;
}

export interface Family {
  name: string;
  periods: PeriodWithState[];
}

export interface DocSyncOutcome {
  changed: string[];
  committed: boolean;
  pushed: boolean;
}

// ── 仓库（发布地址矩阵）──────────────────────────────────────────────

/** 一段可转发给别人的仓库配置（clone 地址 + 发布地址矩阵 + 自定义信息）。 */
export interface SharedRepo {
  schema: number;
  name: string;
  url: string;
  gitlab: string;
  /** 环境标识 → 地址。 */
  envs: Record<string, EnvAddress>;
  notes: string;
  links: Link[];
}

export interface RepoImportResult {
  name: string;
  /** 是不是新克隆的。 */
  added: boolean;
  info_applied: number;
  /** 本地已有、没有覆盖的项。 */
  kept_local: string[];
  /** 本地没有的环境（地址跳过了）。 */
  skipped_envs: string[];
  env_created: [string, string, string][];
  env_failures: [string, string][];
}

// ── 需求配置的复制 / 粘贴 ─────────────────────────────────────────────

/** 载荷里的一个模块：名字 + 远程地址（对方没有这个仓库时靠它克隆）。 */
export interface SharedModule {
  name: string;
  url: string;
}

/** 一段可转发给别人的需求配置。 */
export interface SharedRequirement {
  schema: number;
  branch: string;
  title: string;
  base: string[];
  doc_family: string | null;
  /** 挂了文档库时是它的仓库名（`doc-hub`）。**信息性字段**，导入时不看它。 */
  doc_hub: string | null;
  notes: string;
  links: Link[];
  modules: SharedModule[];
}

/** 导入结果。 */
export interface ShareImportResult {
  branch: string;
  title: string;
  /** 这次补上的仓库（本地原本没有）。 */
  added_repos: string[];
  modules: string[];
  reused: string[];
  /** 配置里写了、但这个仓库的远程没有该分支（正常）。 */
  not_involved: string[];
  doc: AttachOutcome | null;
  doc_error: string | null;
}

// ── GitLab ────────────────────────────────────────────────────────────

export interface GitlabStatus {
  /** 当前 hub 绑定的地址（没打开 hub、或还没绑定时为 `""`）。 */
  base_url: string;
  /** 是否已绑定 GitLab（老 hub / 离线创建的可能没绑）。 */
  bound: boolean;
  /** **有没有打开 hub**。没打开时界面要提示先打开，而不是显示一份「全局配置」。 */
  has_hub: boolean;
  source: string;
  has_stored_token: boolean;
  /** 凭据文件位置（在当前 hub 目录里）。 */
  store_path: string;
  user: [string, string] | null;
  error: string | null;
}

export interface GitlabProject {
  id: number;
  name: string;
  path_with_namespace: string;
  default_branch: string | null;
  web_url: string;
  ssh_url_to_repo: string;
  http_url_to_repo: string;
}

export interface GitlabUser {
  id: number;
  username: string;
  name: string;
}

// ── 平台 ──────────────────────────────────────────────────────────────

export interface EntryInfo {
  name: string;
  path: string;
  is_dir: boolean;
}

export interface TerminalOption {
  id: string;
  label: string;
}

/** 设置里的「用编辑器打开」候选（`installed` 是本机是否找到入口）。 */
export interface EditorOption {
  id: string;
  label: string;
  installed: boolean;
}

// ── 发布地址 ──────────────────────────────────────────────────────────

export interface EnvAddress {
  url: string;
  domain: string;
}

/** 一行 = 一个仓库 × 各环境。 */
export interface MatrixRow {
  name: string;
  gitlab: string;
  /** `[环境标识, 地址]`，顺序与 hub 配置一致。 */
  cells: [string, EnvAddress][];
  /** 自定义备注。 */
  notes: string;
  /** 自定义链接（tapd / 飞书 / 域名 / 监控 …）。 */
  links: Link[];
}

export interface AddressInput {
  repo: string;
  env: string;
  url: string;
  domain: string;
}


// ── 危险操作 ──────────────────────────────────────────────────────────

export interface RevertModule {
  module: string;
  modified: string[];
  untracked: string[];
}
export interface RevertPlan {
  branch: string;
  modules: RevertModule[];
}
export interface RevertOutcome {
  reverted: [string, number][];
  failed: [string, string][];
}

export interface ResetModule {
  module: string;
  target: string;
  local_commits: number;
  dirty_files: number;
  ahead: boolean;
  behind: boolean;
}
export interface ResetPlan {
  branch: string;
  modules: ResetModule[];
  skipped: [string, string][];
}
export interface ResetOutcome {
  reset: [string, string][];
  skipped: [string, string][];
  failed: [string, string][];
}

export interface BranchState {
  module: string;
  project: string;
  remote_branch: string;
  merged_to_main: boolean;
  local_unpushed: number;
  exists: boolean;
}
export interface PrunePlan {
  branch: string;
  safe: BranchState[];
  risky: [BranchState, string][];
  skipped: [string, string][];
}
export interface PruneOutcome {
  deleted: [string, string][];
  failed: [string, string][];
  skipped: [string, string][];
}

export interface RenamePlan {
  old_branch: string;
  new_branch: string;
  modules: BranchState[];
  problems: string[];
  notice: string[];
}
export interface RenameOutcome {
  renamed: string[];
  pushed: string[];
  deleted_remote: string[];
  failed: [string, string][];
  notice: string[];
}

// ── 文件浏览与对比 ────────────────────────────────────────────────────

/** 「与基线」解析结果：基线链里日期最新且该模块存在的 ref。 */
export interface BaseResolution {
  reference: string;
  missed: string[];
  fell_back_to_main: boolean;
}

/** 一个改动文件：路径 + git 状态字母（A/M/D/R…）。 */
export interface FileChange {
  path: string;
  /** `A` 新增 / `M` 修改 / `D` 删除 / `R` 重命名 … */
  status: string;
}

/** 一条提交。 */
export interface CommitInfo {
  sha: string;
  author: string;
  /** `YYYY-MM-DD HH:MM`。 */
  date: string;
  subject: string;
}

export interface FileDiff {
  path: string;
  stat: string;
  diff: string;
  /** 最后一次改动这个文件的人。 */
  author: string;
  /** 该提交的日期 YYYY-MM-DD。 */
  date: string;
  /** 该提交的说明。 */
  subject: string;
  /** 看历史提交时，这里给的是**那个提交时的内容**。 */
  content: string;
}


// ── 自更新 ────────────────────────────────────────────────────────────

/** 一键更新的结果。 */
export interface UpdateInstallOutcome {
  version: string;
  /** `restart`（已装好，退出即可重启） | `manual`（Linux 的 deb 要自己 sudo 装）。 */
  next: "restart" | "manual";
  /** 下好的安装包落在哪。 */
  package: string;
  message: string;
  /** 需要人工执行时要敲的命令（Linux 的 deb 要 sudo）；界面单独显示 + 一键复制。 */
  command: string | null;
}

export interface UpdateInfo {
  current: string;
  /** `up_to_date` | `available` | `ahead` */
  state: string;
  latest: string | null;
  release_url: string;
  /** Releases 页地址（检查结果无论哪种都有，可以直接跳）。 */
  releases_url: string;
  /** 当前平台对应的资产名。 */
  asset: string;
}

/** 一个已安装的 AI CLI 的 skills 状态。 */
export interface SkillTargetStatus {
  label: string;
  dir: string;
  version: string | null;
  installed: boolean;
}

/** 桌面端 / 命令行 / Skills 的版本状态（#7：设置页一眼看出谁落后了）。 */
export interface ToolVersions {
  desktop: string;
  cli: string | null;
  cli_path: string | null;
  /** 只列本机真实存在的 CLI skills 目录。 */
  skill_targets: SkillTargetStatus[];
}

/**
 * 一次 CLI / Skills 同步的结果（桌面端自带同版本，安装/启动时自动覆盖）。
 * 两条都是后端拼好的**人话**，界面直接显示。
 */
export interface ToolSyncOutcome {
  changed: boolean;
  cli: string;
  skills: string;
}

/**
 * 在 hub 根目录开一个 Paseo AI 会话的结果（#6）。
 * cwd 是 hub 根而不是需求目录，AI 才读得到 `AGENTS.md` 与 `repos/` 里的主干代码。
 */
export interface PaseoSession {
  branch: string;
  cwd: string;
  agent_id: string | null;
  opened: boolean;
  message: string;
}


// ── 环境覆盖校验 ──────────────────────────────────────────────────────

export interface BranchCheck {
  repo: string;
  branch: string;
  exists: boolean;
  /** 探测失败的原因。有它说明是「没查到」而不是「确定没有」。 */
  error: string | null;
}

/**
 * 选模块时的列表：**文档库置顶，其余按原顺序**。
 *
 * 实测反馈：文档库（doc-hub）要能选 —— 一般需求都需要文档，所以**默认选中**；
 * 但有些需求确实不需要，所以要能取消。置顶是为了「一眼看到选没选它」。
 *
 * 它跟普通仓库的区别只在**挂载方式**（走 `doc attach`：建期次目录、写 meta.json、
 * 认需求分类），core 侧据此分流，界面不必另设一套。
 */
export const moduleRepos = (repos: RepoRow[]) => [
  ...repos.filter((r) => r.is_doc),
  ...repos.filter((r) => !r.is_doc),
];

/**
 * 某仓库在某分支上的 GitLab 页面地址。
 *
 * `https://host/group/repo` + `/-/tree/<分支>` —— 点开**直接落在那个分支上**，
 * 而不是仓库首页，省掉在网页里再切一次分支（实测反馈：审代码时这一步很烦）。
 *
 * 分支名要做 URL 编码：`feature/xxx` 里的斜杠不编码会被 GitLab 当成路径分隔。
 */
export function gitlabBranchUrl(remote: string, branch: string): string | null {
  const m = remote.match(/^(?:https?:\/\/|git@)([^/:]+)[/:](.+?)(?:\.git)?$/);
  if (!m) return null;
  const base = `https://${m[1]}/${m[2]}`;
  if (!branch.trim()) return base;
  return `${base}/-/tree/${encodeURIComponent(branch.trim())}`;
}

/** 文档目录树的一个节点（目录或文件）。 */
export interface DocNode {
  name: string;
  abs: string;
  dir: boolean;
  children?: DocNode[];
}
