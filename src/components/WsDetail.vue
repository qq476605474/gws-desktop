<script setup lang="ts">
import { useCopy } from "../lib/copy";
import { NO_AUTO } from "../lib/inputs";
import { computed, onMounted, ref, watch } from "vue";
import {
  NAlert,
  NButton,
  NCard,
  NInput,
  NModal,
  NSpin,
  NTabPane,
  NTabs,
  NTag,
  useDialog,
  useMessage,
} from "naive-ui";

import * as api from "../bridge";
import { openInFinder, openInTerminal, openUrl } from "../bridge";
import { useAppStore } from "../stores/app";
import { withBusy } from "../busy";
import * as fmt from "../format";
import OpDialog, { type OpDir, type OpPayload, type OpResult } from "./OpDialog.vue";
import ActionResult from "./ActionResult.vue";
import LinkEditor from "./LinkEditor.vue";
import BaseEditor from "./BaseEditor.vue";
import LinkList from "./LinkList.vue";
import MrPanel from "./MrPanel.vue";
import MergeCheck from "./MergeCheck.vue";
import QuickLinks from "./QuickLinks.vue";
import PaseoSessionButton from "./PaseoSessionButton.vue";
import EditorButton from "./EditorButton.vue";
import FileBrowser from "./FileBrowser.vue";
import RepoInfoInline from "./RepoInfoInline.vue";
import DocTree from "./DocTree.vue";
import { nodesFromFiles } from "../lib/docTree";
import DangerDialog from "./DangerDialog.vue";
import { gitlabBranchUrl, moduleRepos, outcomeKind, outcomeReason } from "../types";
import { baseChainLabel, hasBase } from "../lib/base";
import type { Link, MergeOutcome, ModuleStatus, Outcome, RemoteProbe, WsDetail as Detail, WsShell } from "../types";

/** 文档库的名字。多处用到，写一次。 */
/** 文档库在 hub 里的目录名 —— 从 hub 配置读，别写死（用户填什么就叫什么）。 */
const DOCHUB = computed(() => app.hub?.dochub_name ?? "doc-hub");

/** 表格里的一行。`pending` = 数字还没读回来（显示「…」而不是 0）。 */
type Row = ModuleStatus & { pending?: boolean };

const props = defineProps<{ branch: string }>();
/**
 * 当前分支名。
 *
 * 一般情况下就等于 `props.branch`，但**改分支名之后会变** —— 而
 * `afterOp()` 紧接着会去探测远程，用旧名字查必然查不到、提示错误（实测反馈）。
 * 所以所有后续动作都读这个 ref，而不是 props。
 */
const branch = ref(props.branch);
const emit = defineEmits<{ (e: "close"): void; (e: "changed"): void }>();

const app = useAppStore();

/** 快捷入口：hub 里配了才显示（开源版不预设任何发布系统）。 */
const quickLinks = computed(() => app.hub?.quick_links ?? []);
const msg = useMessage();
const copy = useCopy();
const dialog = useDialog();

const detail = ref<Detail | null>(null);
/**
 * 第一屏（元数据 + 模块名，**不打 git**，毫秒级）。
 *
 * 页面的骨架靠它，各模块那三列数字靠 `detail` —— 后者要起十几个 git 进程，
 * Windows 上一次一两秒（实测反馈：「每次进去需求还是会转一会儿圈」）。
 * `detail` 回来之前那几格显示「…」：**不能显示 0**，0 会被读成"真的没有改动"。
 */
const shell = ref<WsShell | null>(null);
const err = ref("");
const loading = ref(true);
const seq = ref(0);

/** 需求 #8：打开时**异步**探测远程是否有他人推送。
 *  本地状态先渲染，这个结果晚一点回来再补标记——否则每次点开都要等 fetch。 */
const probe = ref<RemoteProbe[] | null>(null);
const probing = ref(false);

/** 弹窗状态：`null` = 没开；否则是要执行的操作名。 */
const op = ref<null | "push" | "pull" | "merge" | "push-env" | "sync-main">(null);
/**
 * 预勾选的模块。空 = 全选。
 *
 * 「远程有新提交」那条提示点「去拉取」时，勾的就该是**落后了的那几个**
 * ——提示里已经点名了它们，点进去却全选上，等于提示白给（实测反馈 #2）。
 */
const opPreselect = ref<string[] | null>(null);
/** 统一入口：顺手把预勾选也带上，免得漏。 */
function openOp(which: NonNullable<typeof op.value>, preselect: string[] | null = null) {
  opPreselect.value = preselect;
  op.value = which;
}
const showAdd = ref(false);
/**
 * 正在浏览的模块 + 落哪一页。
 *
 * 「浏览文件」看真实文件（文件树），「对比代码」默认看与本地（未提交的改动）的
 * 差异 —— 需求分支常常还没推送，那时跟远程比没有基准。两个按钮，两件事（实测反馈）。
 */
const browse = ref<{ module: string; mode: "tree" | "changed" } | null>(null);

function openBrowse(module: string, mode: "tree" | "changed") {
  browse.value = { module, mode };
}
/** 合并到环境检查（合进环境后看结果）。 */
const showMergeCheck = ref(false);
/** 提 MR（模块标签页里的一个功能，不再单开一个标签页）。 */
const showMr = ref(false);
/** 快捷入口（灰度 / 正式两个入口，默认灰度）—— 发版时不用退回总览页。 */
const showQuickLinks = ref(false);
/** 打开的危险操作。 */
const danger = ref<null | "revert" | "reset" | "prune" | "rename">(null);
const renameInput = ref("");

/**
 * 改分支名弹窗的实例：填了新名字要**让它重算影响面**。
 *
 * ⚠️ 弹窗里的预览是**挂载时**算一次 —— 而打开时输入框是空的，于是永远停在
 * 「请先填入新分支名」。不重算的话，人点「继续」时根本没看到会发生什么，
 * 「先看影响面」这一层就白设了。
 */
const renameDlg = ref<{ refreshPreview: () => Promise<void> } | null>(null);
/** 影响面还没跟上最新的新分支名（正在防抖/计算）—— 这时不让点「继续」。 */
const renamePreviewStale = ref(false);
/**
 * 新分支名一改就重算影响面。
 *
 * 加 400ms 防抖：`branchRenamePlan` 要跑 git（每个模块都要算一遍），
 * 逐字符重算会把界面拖住。
 */
let renamePreviewTimer: ReturnType<typeof setTimeout> | undefined;
watch(renameInput, () => {
  clearTimeout(renamePreviewTimer);
  // 空输入不用算（预览本身会提示"请先填"），只把「陈旧」标记撤掉
  if (!renameInput.value.trim()) {
    renamePreviewStale.value = false;
    return;
  }
  renamePreviewStale.value = true;
  renamePreviewTimer = setTimeout(() => {
    void renameDlg.value?.refreshPreview().finally(() => {
      renamePreviewStale.value = false;
    });
  }, 400);
});

/**
 * 打开危险操作弹窗。
 *
 * **每次都把输入重置** —— `renameInput` 是这个组件的常驻 ref（不像弹窗本身
 * 会随 `v-if` 重新挂载），不清的话上次填的新分支名还在（实测反馈）。
 */
function openDanger(kind: "revert" | "reset" | "prune" | "rename") {
  clearTimeout(renamePreviewTimer);
  renamePreviewStale.value = false;
  renameInput.value = "";
  danger.value = kind;
}



const wsPath = computed(() => `${app.hub?.root}/ws/${branch.value}`);

/** 该模块在需求分支上的 GitLab 页面（点开就落在分支上，不用再切）。 */
function gitlabUrlOf(module: string): string | null {
  const remote = app.repos.find((r) => r.name === module)?.remote_url ?? "";
  if (!remote) return null;
  return gitlabBranchUrl(remote, branch.value);
}
/** 占位行：模块名与是否存在是真的，数字还不知道（`pending`）。 */
function placeholder(name: string, present = true): Row {
  return {
    module: name,
    present,
    dirty: 0,
    pushed: false,
    ahead: 0,
    behind: 0,
    ahead_of_main: null,
    error: null,
    pending: true,
  };
}

const modules = computed<Row[]>(() => {
  const d = detail.value?.status.modules;
  // 数字回来了就用真的；还没回来就把**模块名先摆出来**（从 shell 拿）
  if (!d) return (shell.value?.modules ?? []).map((m) => placeholder(m.name, m.present));
  // 文档库只出现在**置顶那一行**。万一它从哪儿混进了模块列表，这里再兜一层 ——
  // 否则界面上会出现两个 doc-hub（实测反馈）。
  return d.filter((m) => m.module !== DOCHUB.value);
});
/**
 * 操作弹窗（拉取 / 推送 / 合并…）能勾的模块 —— **这里要带上文档库**。
 *
 * `modules` 把 doc-hub 滤掉了（它在界面上只出现在置顶那一行，不能出现两次），
 * 但文档库在工作区里是一个真实的 worktree，需求文档得跟着需求一起推、一起合。
 * 实测反馈：合并 /「合并到环境并推送」时选不到 doc-hub。
 *
 * 直接用 `status.modules`（它现在含文档库那条），不再走上面那个过滤版。
 *
 * 「撤销改动 / 强制回退」也用它 —— 那两个操作同样要作用到文档库上，
 * 原来用的是不含文档库的那一份，于是**静默漏掉**（实测反馈）。
 */
const opModules = computed<Row[]>(() => {
  if (!detail.value) return modules.value; // 第一屏就能用（存在与否是查目录得出的）
  return detail.value.status.modules.filter((m) => m.present);
});
/**
 * 工作区里**已登记的全部模块**（含文档库）。
 *
 * 「增加模块」的候选要拿它过滤：`modules` 是不含文档库的那份（界面显示用），
 * 用它判断"在不在工作区里"会把已挂上的文档库又列一遍（实测反馈）。
 */
const allModules = computed<Row[]>(() => {
  const d = detail.value?.status.modules;
  if (d) return d;
  // 第一屏：shell 里只有代码模块，文档库要按 `doc` 标记补上 ——
  // 否则「增加模块」会把已经挂上的文档库又列一遍
  const out = (shell.value?.modules ?? []).map((m) => placeholder(m.name, m.present));
  if (shell.value?.doc) out.push({ ...placeholder(DOCHUB.value), present: true });
  return out;
});
const meta = computed(() => detail.value?.meta ?? shell.value?.meta ?? null);

/**
 * 基线的显示文本（没填基线时为空串，模板据此不显示）。
 *
 * 明细头部要跟列表一样能看出**基线来源**与**需求分类** —— 否则从列表点进来，
 * 「这一期属于哪个需求、基于哪一条」这些上下文就丢了。
 */
const baseText = computed(() => {
  const b = meta.value?.base ?? [];
  return hasBase(b) ? baseChainLabel(b) : "";
});

/**
 * `仓库名 → 自定义信息`（链接 + 备注），来自 `deploy_matrix`。
 *
 * 需求详情里**只显示不编辑**（编辑统一在「仓库」页）。显示它的用处：
 * 在仓库上配的 Jenkins 地址，合完代码往往马上就要去发布，信息在这里
 * 就能点着走（实测反馈）。
 */
const repoInfo = ref<Record<string, { notes: string; links: Link[] }>>({});

/** 「导出需求配置」正在生成（导出要读各模块的远程地址）。 */
const sharing = ref(false);

/**
 * 把这段配置复制到剪贴板。
 *
 * 生成放在 core 里（那边才知道每个模块的远程地址），这里只负责复制与提示 ——
 * **提示里要说清下一步**：光说「已复制」的话，人不知道该让对方怎么用。
 */
async function copyShare() {
  if (sharing.value) return;
  sharing.value = true;
  try {
    const text = await api.wsShare(branch.value);
    await copy(text, "需求配置");
    msg.success("配置已复制；对方粘到「导入需求」里即可建出同一个需求");
  } catch (e) {
    msg.error(api.errText(e));
  } finally {
    sharing.value = false;
  }
}

/**
 * 「需求相关文档」快捷弹窗。
 *
 * 名字里带「相关」是**故意的**：只叫「需求文档」容易被理解成「需求描述文档」，
 * 而这里其实是**这个需求牵扯到的全部文档**（需求文档、技术方案、提测单…）。实测反馈。
 *
 * 实测反馈：希望有个按钮**直接看本次需求的文档**（不含归档），不用绕到「文档」页
 * 再按分类找一遍。用的就是 `doc_view` —— 它读的是本需求工作区里的期次目录，
 * 天然只含这一次，不含已归档的历史。
 */
const showDoc = ref(false);
const docLoading = ref(false);
const docErr = ref("");
const doc = ref<import("../types").DocView | null>(null);
/** 工具生成的脚手架，不是用户文档，不在「需求相关文档」里列出来。 */
const DOC_SCAFFOLD = new Set(["meta.json", "CLAUDE.md", "AGENTS.md"]);
const docNodes = computed(() =>
  doc.value
    ? nodesFromFiles(
        doc.value.files.filter((f) => !DOC_SCAFFOLD.has(f.rel)),
        doc.value.empty_dirs ?? [],
        doc.value.dir,
      )
    : [],
);

async function openDoc() {
  showDoc.value = true;
  docLoading.value = true;
  docErr.value = "";
  doc.value = null;
  try {
    doc.value = await api.docView(branch.value);
  } catch (e) {
    // 没挂文档时这里会报错 —— 说清「没挂」，别给一个空白弹窗
    docErr.value = api.errText(e);
  } finally {
    docLoading.value = false;
  }
}

/**
 * 提交并上传这次需求的文档。
 *
 * **界面上原来没有这个入口**：文档页只有「刷新 / 同步最新文档 / 打开文档库」，
 * 而详情页那句提示写着「在『文档』页可以提交并上传」—— 指着一个不存在的按钮
 * （`doc_sync` 命令一直躺在后端没人调）。写文档的最后一公里只能在命令行走，
 * 所以把这个入口放回它本该在的地方：看得见「有 N 处改动没提交」的那个弹窗。
 */
const docSubmitting = ref(false);
async function submitDoc() {
  if (!doc.value || docSubmitting.value) return;
  docSubmitting.value = true;
  try {
    const title = meta.value?.title?.trim() || branch.value;
    const out = await api.docSync(branch.value, `docs(${title})`, true);
    if (!out.changed.length) msg.info("文档没有改动，无需提交");
    else if (out.pushed) msg.success(`已提交并推送 ${out.changed.length} 个文件`);
    else msg.warning(`已提交 ${out.changed.length} 个文件，但**没有推送成功**，请重试`);
    await openDoc(); // 重读这一份，pending 才会清掉
    emit("changed");
  } catch (e) {
    msg.error(api.errText(e));
  } finally {
    docSubmitting.value = false;
  }
}
/** 危险操作（撤销改动 / 强制回退）作用到哪些模块。 */
const opOnly = () => opModules.value.map((m) => m.module);

/**
 * 危险操作的预览与执行。
 *
 * **写在 script 里而不是模板里的内联箭头**：模板里的表达式会经过 `$setup`
 * 代理（`proxyRefs`）取值，而它会对取到的东西做 `isRef()` 探测（读 `__v_isRef`）。
 * 模块命名空间对象在单测里是 vitest 的 mock，探测不存在的键会**抛错**
 * （`No "__v_isRef" export is defined on the "…" mock`）—— 表现是预览框里
 * 一句莫名其妙的报错，排查了很久。写成函数就绕开了这条代理链，顺带更清楚。
 *
 * 「改分支名」那份不一样：它**跟着输入变**，所以还得在 `renameInput` 上挂
 * 一个 watch 主动重算（见上面），否则预览永远停在打开时算的第一次。
 */
async function renamePlan(): Promise<string[]> {
  const nb = renameInput.value.trim();
  if (!nb) return ["请先填入新分支名，然后再看影响面"];
  return fmt.renamePlanLines(await api.branchRenamePlan(branch.value, nb));
}

const prunePlan = async () => fmt.prunePlanLines(await api.branchPrunePlan(branch.value));
const pruneApply = async () => fmt.pruneResultLines(await api.branchPruneApply(branch.value));

const revertPlan = async () => fmt.revertPlanLines(await api.wsRevertPlan(branch.value, { only: opOnly() }));
const revertApply = async () => fmt.revertResultLines(await api.wsRevertApply(branch.value, { only: opOnly() }));
const resetPlan = async () => fmt.resetPlanLines(await api.wsResetPlan(branch.value, { only: opOnly() }));
const resetApply = async () => fmt.resetResultLines(await api.wsResetApply(branch.value, { only: opOnly() }));

const infoOf = (name: string) => repoInfo.value[name] ?? { notes: "", links: [] };

async function loadRepoInfo() {
  try {
    const all = await api.deployMatrix();
    repoInfo.value = Object.fromEntries(
      all.map((r) => [r.name, { notes: r.notes ?? "", links: r.links ?? [] }]),
    );
  } catch {
    // 读不到就当都没配过：这是可选数据，不该让需求详情报错
    repoInfo.value = {};
  }
}

async function refresh() {
  const n = ++seq.value;
  loading.value = true;
  try {
    const d = await api.wsDetail(branch.value);
    if (n !== seq.value) return;
    detail.value = d;
    err.value = "";
  } catch (e) {
    if (n !== seq.value) return;
    err.value = api.errText(e);
  } finally {
    if (n === seq.value) loading.value = false;
  }
}

/** 文档那一行的数字，直接来自 ws_detail（跟模块状态一起给的）。 */
const docRow = computed(() => detail.value?.doc ?? null);

/** 数字那一半还没回来（那几格显示「…」）。 */
const countsPending = computed(() => detail.value === null);

async function probeRemote() {
  probing.value = true;
  try {
    probe.value = await api.wsProbeRemote(branch.value);
    // 逐个模块都失败 → 多半不是"某个仓库的网络抖动"，而是**凭据没配**
    // （`fetch` 要认证）。这时必须说话：实测反馈「拉取失败却没有任何提示」。
    const failed = probe.value.filter((p) => p.error);
    if (failed.length && failed.length === probe.value.length) {
      msg.warning(
        `${failed.length} 个模块都探测失败：${failed[0].error}\n` +
          `若是没配 GitLab 凭据，到「设置」页配一下 token，或先在你自己的终端里 clone 一次让 git 记住凭据`,
      );
    }
  } catch (e) {
    // 整条命令失败（打不开工作区等）**也要说**，不能像没点过一样
    probe.value = [];
    msg.error(`检查远程失败：${api.errText(e)}`);
  } finally {
    probing.value = false;
  }
}

/**
 * 手动检查远程（实测反馈 #8）。
 *
 * 进详情页时已经异步探过一次，但「刚才没改动、过一会远程上推了新的」是常态，
 * 光靠进页面那一次不够。手动这一次**给反馈**：转圈 + 结果提示，
 * 否则点完没有任何反应，人不知道到底查了没有。
 */
async function checkRemoteNow() {
  try {
    await withBusy("正在检查远程是否有新提交", probeRemote);
    const n = probeHits.value.length;
    if (n) msg.warning(`有 ${n} 个模块落后远程，上面已列出`);
    else msg.success("远程没有新提交，本地都是最新的");
  } catch (e) {
    msg.error(api.errText(e));
  }
}

onMounted(async () => {
  // 三段，按"先后要紧程度"排：
  //   ① 第一屏（元数据 + 模块名，纯读文件）—— 立刻把页面画出来
  //   ② 数字（每个模块几个 git 进程）—— 回来就填进那三列
  //   ③ 检查远程（每个模块一次 fetch，**纯网络**）—— 排在最后，
  //      免得抢第一屏的磁盘/网络；它只负责"远程上有新提交"那条提示
  // 先等自定义信息（本机一个 JSON，毫秒级）：它渲染在**模块行的单元格里**，
  // 晚一步出现会把那一行撑高、整张表跟着动一下（跟备注块是同一类问题）。
  await loadRepoInfo();
  await loadShell();
  await refresh();
  void probeRemote();
});

/**
 * 回到前台后重读。
 *
 * Agent 里用 skill 改了需求（改名、加/删模块、改备注）之后，这边没有通知；
 * 列表刷新（`app.refreshTick`）时顺带把这页的元数据和 git 数字也重读一遍。
 * 需求已经不在了（被改名）就退回列表 —— 列表上显示的才是新名字。
 */
watch(
  () => app.refreshTick,
  async () => {
    if (!app.workspaces.some((w) => w.branch === branch.value)) {
      msg.warning("这个需求已不在列表里（可能被改名或删除）");
      emit("changed");
      emit("close");
      return;
    }
    await loadShell();
    await refresh();
  },
);

/** 第一屏：只要元数据与模块名，失败也不吭声（`refresh` 会把错误显示出来）。 */
async function loadShell() {
  try {
    shell.value = await api.wsShell(branch.value);
  } catch {
    shell.value = null;
  }
}

const probeHits = computed(() => (probe.value ?? []).filter((p) => p.behind > 0));

/** 完整清单（头部那行放不下就省略，鼠标停上去看这个）。 */
const probeText = computed(() =>
  probeHits.value.map((p) => `${p.module} 落后 ${p.behind}`).join("、"),
);

/**
 * 合并结果里**可以直接打开的目录**。
 *
 * 合完之后要 review 再决定推不推、出了冲突要解 —— 原来只打印路径，人还得
 * 切到「环境」页找同一个目录（实测反馈）。这里把「合过的模块」都列出来，
 * 冲突的标出来，右边直接给编辑器按钮。
 */
function mergedDirs(o: MergeOutcome): OpDir[] {
  const root = app.hub?.root;
  if (!root) return [];
  const mods = new Set<string>([
    ...o.clean,
    ...o.conflicts,
    ...o.pending.map(([m]) => m),
  ]);
  // **整个环境目录放第一个**：需要批量 review / 批量解冲突时，在 Rebased、IDEA
  // 里开这一个目录就够了，不用逐个模块开（实测反馈）。
  return [
    {
      path: `${root}/envs/${o.env}`,
      label: `${o.env}（整个环境）`,
      note: o.conflicts.length ? `${o.conflicts.length} 个模块有冲突` : undefined,
    },
    ...[...mods].map((m) => ({
      path: `${root}/envs/${o.env}/${m}`,
      label: m,
      note: o.conflicts.includes(m) ? "冲突待处理" : undefined,
    })),
  ];
}

async function runOp(p: OpPayload): Promise<string[] | OpResult> {
  const only = p.modules;
  const filter = { only };
  // 期间显示进度遮罩：这几个操作逐个模块 fetch/merge，主线程不阻塞之后
  // 遮罩上的进度条会随 core 上报的进度走（实测反馈：原来只是卡住不动）。
  const label =
    op.value === "merge"
      ? `合并到环境 ${p.env ?? "dev"}`
      : op.value === "push-env"
        ? `合并并推送到环境 ${p.env ?? "dev"}`
        : opTitle.value || "执行中";

  return withBusy(label, async (): Promise<string[] | OpResult> => {
    switch (op.value) {
      case "push":
        return fmt.pushLines(await api.wsPush(branch.value, filter));
      // 拉取方式由弹窗里的勾选决定（反馈 #23：Pull --rebase 是需要的，
      // 但 rebase 会改写本地提交顺序，得让人显式选并看到代价说明）
      case "pull":
        return fmt.pullLines(await api.wsPull(branch.value, p.rebase, filter));
      // 合并与「合并到环境并推送」合成一个入口，推不推在弹窗里勾（反馈 #4）。
      // 默认不推——推了远程上立刻可见，该由人显式选。
      case "merge": {
        const o = await api.wsMerge(branch.value, p.env ?? "dev", p.push, p.force, filter);
        return { lines: fmt.mergeLines(o), dirs: mergedDirs(o) };
      }
      case "push-env": {
        // 与「合并到环境」**同一个操作**，区别只在弹窗里「合并后推送」默认勾上。
        // 合过去再推，正是「合并到环境并推送」的语义。
        const o = await api.wsMerge(branch.value, p.env ?? "dev", p.push, p.force, filter);
        return { lines: fmt.mergeLines(o), dirs: mergedDirs(o) };
      }
      case "sync-main":
        return fmt.syncMainLines(await api.wsSyncMain(branch.value, [], filter));
      default:
        return ["（未知操作）"];
    }
  });
}

async function afterOp() {
  await refresh();
  void probeRemote();
  emit("changed");
}

const opTitle = computed(() => {
  switch (op.value) {
    case "push":
      return "推送需求分支到远程";
    case "pull":
      return "拉取远程上同一分支的提交";
    case "merge":
      return "合并到环境";
    case "push-env":
      return "合并到环境并推送";
    case "sync-main":
      return "把基线的最新提交合进本需求";
    default:
      return "";
  }
});

const opHint = computed(() => {
  switch (op.value) {
    case "push":
      return "推送后远程上才看得到你的提交。若有模块落后远程，会先提示你先拉取。";
    case "merge":
      return "默认只合到本地 envs/<环境>。要推到远程环境分支（远程上立刻可见）请勾下面的「合并后推送」。";
    case "push-env":
      return "与「合并到环境」是同一个操作，只是**「合并后推送」已经替你勾上**了。合完直接推到远程环境分支，远程上立刻可见。";
    default:
      return undefined;
  }
});

/** 并入主干检查：**有过程弹窗**（每个模块都要 fetch，跑十几秒）。
 *  原来直接 await 然后弹一个文本对话框，中间那段什么都没有，人以为卡死了。 */
const doneLines = ref<string[] | null>(null);
const doneRunning = ref(false);

async function doDone() {
  doneRunning.value = true;
  doneLines.value = null;
  try {
    // withBusy 负责那层转圈 + 实时进度日志（ws_done 会上报逐模块进度）
    doneLines.value = fmt.doneLines(await withBusy("检查是否已并入主干", () => api.wsDone(branch.value)));
  } catch (e) {
    doneLines.value = [`✗ ${api.errText(e)}`];
  } finally {
    doneRunning.value = false;
  }
}

/** 摘掉文档（不影响 doc-hub 上已有的内容，之后能加回来）。 */
async function detachDocs() {
  try {
    await api.docDetach(branch.value);
    msg.success(`文档已从本需求移除（${DOCHUB.value} 上的内容保留）`);
    await afterOp();
  } catch (e) {
    msg.error(api.errText(e));
  }
}

async function attachDocs() {
  try {
    const a = await api.docAttach(branch.value, null);
    msg.success(`文档目录已挂上：${a.relative_dir}`);
    await afterOp();
  } catch (e) {
    msg.error(api.errText(e));
  }
}

/**
 * 执行改分支名。
 *
 * **必须在脚本里做**：模板里的 ref 是自动解包的（`branch` 就是字符串），
 * 在那里写 `branch.value = ...` 会报错；而且改完要更新后续要用的分支名。
 */
async function applyRename(): Promise<string[]> {
  const next = renameInput.value.trim();
  const out = await api.branchRenameApply(branch.value, next);
  // 目录和分支都换了名字，后续一切（探测远程、刷新）都得用新的。
  //
  // ⚠️ **但有失败时不能换**：后端在「远程改完了、本地目录没改成」这种情况
  // 下会带着 `failed` 返回（Windows 上目录被占用时很常见）。那时工作区目录还叫
  // 旧名，把 `branch` 换成新名会让接下来的刷新、推送全部指向一个不存在的目录。
  if (!out.failed.length) branch.value = next;
  return fmt.renameResultLines(out);
}

async function dropModule(m: ModuleStatus) {
  dialog.warning({
    title: `移除模块 ${m.module}`,
    content: "会删除该模块的工作目录与本地分支。" + (m.dirty > 0 ? "该模块有未提交改动，需要勾选强制。" : ""),
    positiveText: "移除",
    negativeText: "取消",
    onPositiveClick: async () => {
      try {
        const r = (await api.wsDropModule(branch.value, m.module, m.dirty > 0)) as Outcome;
        // `Outcome` 是带数据的枚举：成功序列化成字符串 `"Ok"`，另两种是对象。
        // 原来只判了 `Failed`，于是 `Skipped`（目录本来就不在）也被报成
        // 「已移出」—— 正是本项目最忌讳的「失败还报成功」。
        const kind = outcomeKind(r);
        if (kind === "failed") msg.error(outcomeReason(r));
        else if (kind === "skipped") msg.warning(`${m.module} 未移出：${outcomeReason(r)}`);
        else msg.success(`${m.module} 已移出`);
        await afterOp();
      } catch (e) {
        msg.error(api.errText(e));
      }
    },
  });
}

async function removeWs() {
  try {
    // 先不带 force 试一次：有未保存的工作时它会返回风险清单而不删除，
    // 比直接弹一个「确定要删除吗」有用得多
    const first = await api.wsRemove(branch.value, false);
    if (!first.risks.length) {
      msg.success("需求已删除");
      emit("changed");
      emit("close");
      return;
    }
    dialog.warning({
      title: "存在未保存的工作",
      content: () =>
        first.risks.map((r) => `✗ ${r.module}: ${r.reason}`).join("\n") +
        "\n\n确认丢弃这些内容并删除需求？",
      positiveText: "强制删除",
      negativeText: "取消",
      onPositiveClick: async () => {
        await api.wsRemove(branch.value, true);
        msg.success("需求已删除");
        emit("changed");
        emit("close");
      },
    });
  } catch (e) {
    msg.error(api.errText(e));
  }
}

async function saveMeta(patch: {
  title?: string;
  base?: string[];
  notes?: string;
  links?: Link[];
}) {
  try {
    const out = await api.wsPatchMeta(branch.value, patch);
    msg.success("已保存");
    // 「顺带动了什么」要说出来（改分类会挪文档目录、改中文名会同步期次元数据）
    for (const n of out.notes ?? []) msg.info(n, { duration: 8000 });
    await afterOp();
  } catch (e) {
    msg.error(api.errText(e));
  }
}

const titleDraft = ref("");
/** 基线链（有序列表，顺序即优先级）。 */
const baseList = ref<string[]>([]);
const baseEditor = ref<InstanceType<typeof BaseEditor> | null>(null);
const notesDraft = ref("");
const linksDraft = ref<Link[]>([]);
const editing = ref(false);

function startEdit() {
  titleDraft.value = meta.value?.title ?? "";
  baseList.value = [...(meta.value?.base ?? [])];
  notesDraft.value = meta.value?.notes ?? "";
  linksDraft.value = (meta.value?.links ?? []).map((l) => ({ ...l }));
  editing.value = true;
}

async function commitEdit() {
  // 前端先校验一遍（core 也会再校验，那边是权威）—— 当场告诉人比等往返好
  const bad = baseEditor.value?.firstError ?? "";
  if (bad) {
    msg.error(`基线来源有问题：${bad}`);
    return;
  }
  const base = baseList.value.map((s) => s.trim()).filter(Boolean);
  await saveMeta({
    title: titleDraft.value,
    base,
    notes: notesDraft.value,
    links: linksDraft.value.filter((l) => l.label.trim() || l.url.trim()),
  });
  editing.value = false;
}
</script>

<template>
  <div>
    <header class="head">
      <NButton size="small" @click="emit('close')">← 返回</NButton>
      <h2 class="title">{{ meta?.title ?? branch }}</h2>
      <!-- 需求分类：列表里就是按它分组的，明细里也得能一眼看到自己属于哪个需求
           （否则从列表点进来，这个上下文就丢了） -->
      <NTag v-if="meta?.doc" size="tiny" :bordered="false" type="info" class="fam">
        {{ meta.doc.family }}
      </NTag>
      <code class="branch">{{ branch }}</code>
      <!-- 基线来源，与列表同一套显示规则（主干兜底挂在链尾） -->
      <span
        v-if="baseText"
        class="base"
        :title="`按这个顺序找上游：第一个有该模块分支的用它，都不命中就兜底主干`"
      >
        基线 {{ baseText }}
      </span>
      <!-- 远程探测的状态**放在头部这一行里**，不做成表格上方的独立一块。
           探测是网络操作（每个模块一次 fetch），要几秒才回来；那一块的出现/消失
           会把下面的表格**挤上挤下**（实测反馈：「表格会突然往下面挤」）。
           所以：只占一行、超出省略、完整清单放 tooltip —— 点「去拉取」时弹窗里
           也会把那几个模块列出来。 -->
      <span
        v-if="probing"
        class="probe muted small"
      >
        <NSpin size="small" />
        检查远程中…
      </span>
      <span
        v-else-if="probeHits.length"
        class="probe warn-note"
        :title="probeText"
      >
        远程有新提交：{{ probeHits.length }} 个模块落后
        <NButton
          size="tiny"
          text
          type="primary"
          @click="openOp('pull', probeHits.map((p) => p.module))"
        >
          去拉取
        </NButton>
      </span>
      <div class="spacer" />
      <!-- 派发Paseo任务：需求级动作，放在头部（模块行留给 git 操作）。
           工作目录固定 hub 根，标签页标题用需求名称；首次会自动把 hub 注册成 Paseo 项目 -->
      <PaseoSessionButton
        :branch="branch"
        label="派发Paseo任务"
        size="tiny"
        type="primary"
        hint="在 hub 根目录派发一个 Paseo 任务，标题用需求名称"
      />
      <NButton size="tiny" secondary type="primary" @click="openDoc">需求相关文档</NButton>
      <!-- 把需求配置导成一段文本：对方粘进「导入需求」就建出跟我这边
           一样的需求（分支、模块、基线、分类、自定义信息全都带过去）。
           提测时也用这个 —— 不用再把分支名念一遍。 -->
      <NButton size="tiny" secondary :loading="sharing" @click="copyShare">导出需求配置</NButton>
      <NButton size="tiny" secondary @click="openInFinder(wsPath)">打开目录</NButton>
      <EditorButton size="tiny" :path="wsPath" />
      <NButton size="tiny" secondary @click="openInTerminal(wsPath, app.terminal)">终端</NButton>
      <NButton size="tiny" secondary @click="copy(wsPath, '目录')">复制目录</NButton>
      <NButton size="tiny" secondary @click="refresh">刷新</NButton>
    </header>

    <NAlert v-if="err" type="error" :bordered="false" class="err">
      <span>{{ err }}</span>
      <NButton size="tiny" text class="retry" @click="refresh">重试</NButton>
    </NAlert>

    <!-- 自定义信息放在最上面：飞书/tapd 这类东西就是"要经常点进去"的，
         藏在"需求设置"里等于没有（实测反馈 #5）。

         ⚠️ 条件里**不能带 `detail`**：备注与自定义信息就在 `meta` 里（本地那几个
         字节，第一屏随手就有），而 `detail` 要等各模块的 git 读数。带上它的后果
         是这块**晚一步才冒出来**，把下面的表格**整个往下挤**（实测反馈：
         「备注和自定义信息也有延迟，表格会突然往下面挤」）。 -->
    <div v-if="meta?.links?.length || meta?.notes" class="ws-links">
      <LinkList v-if="meta?.links?.length" :links="meta.links" />
      <!-- 备注**单独一行**：跟链接挤在一行时看着像同一种东西（实测反馈） -->
      <p v-if="meta?.notes" class="ws-note muted small">{{ meta.notes }}</p>
    </div>

    <!-- 有第一屏（元数据）就把界面画出来；数字还在路上时那几格显示「…」。
         原来这里是 `v-if="detail"` —— 于是进需求要整页转圈等到数字读完
         （Windows 上 1~2 秒，实测反馈）。 -->
    <NTabs v-if="meta" type="line" animated>
      <!-- ── 模块 ── -->
      <NTabPane name="mods" tab="模块">
        <div class="ops">
          <!-- 「检查远程」= fetch，「拉取」= pull。放在一起、按这个顺序，
               语义就对上了（实测反馈）。 -->
          <NButton size="small" :loading="probing" @click="checkRemoteNow">检查远程</NButton>
          <NButton size="small" :disabled="!opModules.length" @click="openOp('pull')">
            拉取
          </NButton>
          <NButton size="small" :disabled="!opModules.length" @click="openOp('push')">
            推送
          </NButton>
          <NButton size="small" :disabled="!opModules.length" @click="openOp('merge')">
            合并到环境
          </NButton>
          <!-- 与「合并到环境」是同一个操作，只是「合并后推送」已经替你勾好。
               名字里带「合并到环境」—— 它紧挨着「推送」，只叫「合并并推送」的话
               一眼看去像是「推送」的另一种说法（实测反馈） -->
          <NButton size="small" :disabled="!opModules.length" @click="openOp('push-env')">
            合并到环境并推送
          </NButton>
          <!-- 紧挨着「合并到环境」：它是那一步之后的检查，放一起才顺 -->
          <NButton size="small" @click="showMergeCheck = true">合并到环境检查</NButton>
          <!-- 发版入口紧跟检查：代码进环境之后就是发布（实测反馈）。hub 没配快捷入口就不显示 -->
          <NButton v-if="quickLinks.length" size="small" @click="showQuickLinks = true">快捷入口</NButton>
          <NButton size="small" :disabled="!opModules.length" @click="openOp('sync-main')">
            同步基线
          </NButton>
          <!-- 提 MR 是需求生命周期里的一环，放在模块这一排里，
               不再单独占一个标签页（反馈 #12） -->
          <NButton size="small" type="primary" @click="showMr = true">提 MR</NButton>
          <NButton size="small" @click="doDone">并入主干检查</NButton>
          <div class="spacer" />
          <!-- 用 `opModules`（**含文档库**）而不是 `presentModules`：
               文档库在工作区里就是一块真实的 worktree，撤销改动 / 强制回退
               同样要作用到它身上。实测反馈：这两个操作会把 doc-hub 漏掉。 -->
          <NButton size="small" :disabled="!opModules.length" @click="openDanger('revert')">
            撤销改动
          </NButton>
          <NButton size="small" type="error" :disabled="!opModules.length" @click="openDanger('reset')">
            强制回退
          </NButton>
          <NButton size="small" type="error" @click="openDanger('prune')">清理远程分支</NButton>
          <NButton size="small" type="error" @click="openDanger('rename')">改分支名</NButton>
          <div class="spacer" />
          <NButton size="small" @click="showAdd = true">增加模块</NButton>
        </div>

        <table class="tbl">
          <thead>
            <tr>
              <th>模块</th>
              <th class="num">改动</th>
              <th class="num">vs远程</th>
              <th class="num">vs主干</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            <!-- 文档**也是这一行里的一个模块**，排在最前（反馈 #3）。
                 它本身就是个 git 仓库（doc-hub 在该分支上的 worktree），
                 所以照样有「改动」数；只是它不参与代码的推/合，故后两列是 —。 -->
            <tr class="pinned">
              <td>
                <span class="mono">{{ DOCHUB }}</span>
                <NTag
                  size="tiny"
                  type="info"
                  :bordered="false"
                  :title="`需求相关文档目录：${DOCHUB} 在这个分支上的 worktree，与其它模块平级`"
                >
                  文档
                </NTag>
                <!-- 文档库这个仓库的自定义信息（只显示、不编辑） -->
                <RepoInfoInline
                  :links="infoOf(DOCHUB).links"
                  :notes="infoOf(DOCHUB).notes"
                />
              </td>
              <td class="num" :class="{ warn: (docRow?.dirty ?? 0) > 0 }">
                <span v-if="countsPending && meta?.doc" class="muted">…</span>
                <span v-else-if="!docRow" class="muted">—</span>
                <span v-else>{{ docRow.dirty }}</span>
              </td>
              <td class="num">
                <span v-if="countsPending && meta?.doc" class="muted">…</span>
                <span v-else-if="!docRow" class="muted">—</span>
                <span v-else-if="!docRow.pushed">未推送</span>
                <span v-else>↑{{ docRow.ahead }} ↓{{ docRow.behind }}</span>
              </td>
              <td class="num">
                <span v-if="countsPending && meta?.doc" class="muted">…</span>
                <span v-else-if="!docRow" class="muted">—</span>
                <span v-else>+{{ docRow.ahead_of_main }}</span>
              </td>
              <td class="actions">
                <div class="btns">
                  <template v-if="meta?.doc">
                    <NButton size="tiny" secondary @click="openInFinder(`${wsPath}/${DOCHUB}`)">
                      打开目录
                    </NButton>
                    <EditorButton size="tiny" :path="`${wsPath}/${DOCHUB}`" />
                    <NButton size="tiny" secondary @click="copy(`${wsPath}/${DOCHUB}`, '目录')">
                      复制目录
                    </NButton>
                    <NButton size="tiny" secondary @click="openBrowse(DOCHUB, 'tree')">
                      浏览文件
                    </NButton>
                    <NButton size="tiny" secondary @click="openBrowse(DOCHUB, 'changed')">
                      对比代码
                    </NButton>
                    <!-- 位置与其他仓库统一：**终端左边** -->
                    <NButton
                      v-if="gitlabUrlOf(DOCHUB)"
                      size="tiny"
                      secondary
                      type="primary"
                      @click="openUrl(gitlabUrlOf(DOCHUB)!)"
                    >
                      GitLab
                    </NButton>
                    <NButton
                      size="tiny"
                      secondary
                      @click="openInTerminal(`${wsPath}/${DOCHUB}`, app.terminal)"
                    >
                      终端
                    </NButton>
                    <NButton size="tiny" secondary type="error" @click="detachDocs">移除</NButton>
                  </template>
                  <template v-else>
                    <span class="muted small">未挂载</span>
                    <NButton size="tiny" secondary type="primary" @click="attachDocs">
                      挂上文档
                    </NButton>
                  </template>
                </div>
              </td>
            </tr>

            <tr v-for="m in modules" :key="m.module">
              <td>
                <span class="mono">{{ m.module }}</span>
                <NTag v-if="!m.present" size="tiny" type="warning" :bordered="false">目录缺失</NTag>
                <!-- 该仓库的自定义信息（只显示、不编辑）——合完码直接点着去发布 -->
                <RepoInfoInline :links="infoOf(m.module).links" :notes="infoOf(m.module).notes" />
              </td>
              <!-- 数字还没读回来时显示「…」。**不能显示 0** —— 0 会被读成
                   "这个模块真的没有改动"（实测反馈的另一半：等得久，但别骗人） -->
              <td class="num" :class="{ warn: !m.pending && m.dirty > 0 }">
                <span v-if="m.pending" class="muted">…</span>
                <span v-else>{{ m.dirty }}</span>
              </td>
              <td class="num">
                <span v-if="m.pending" class="muted">…</span>
                <span v-else-if="m.error" class="err-text">{{ m.error }}</span>
                <span v-else-if="!m.pushed">未推送</span>
                <span v-else>↑{{ m.ahead }} ↓{{ m.behind }}</span>
              </td>
              <td class="num">
                <span v-if="m.pending" class="muted">…</span>
                <span v-else>+{{ m.ahead_of_main ?? "?" }}</span>
              </td>
              <td class="actions">
                <div class="btns">
                  <NButton size="tiny" secondary @click="openInFinder(`${wsPath}/${m.module}`)">
                    打开目录
                  </NButton>
                  <EditorButton size="tiny" :path="`${wsPath}/${m.module}`" />
                  <NButton size="tiny" secondary @click="copy(`${wsPath}/${m.module}`, '目录')">
                    复制目录
                  </NButton>
                  <NButton size="tiny" secondary @click="openBrowse(m.module, 'tree')">浏览文件</NButton>
                  <!-- 「对比代码」= 直接看与远程的差异（最常用的那个） -->
                  <NButton size="tiny" secondary @click="openBrowse(m.module, 'changed')">
                    对比代码
                  </NButton>
                  <!-- 需求里也是一样：点开直接到该仓库的**需求分支**页面 -->
                  <NButton
                    v-if="gitlabUrlOf(m.module)"
                    size="tiny"
                    secondary
                    type="primary"
                    @click="openUrl(gitlabUrlOf(m.module)!)"
                  >
                    GitLab
                  </NButton>
                  <NButton size="tiny" secondary @click="openInTerminal(`${wsPath}/${m.module}`, app.terminal)">
                    终端
                  </NButton>
                  <NButton size="tiny" secondary type="error" @click="dropModule(m)">移除</NButton>
                </div>
              </td>
            </tr>
          </tbody>
        </table>

        <p v-if="!modules.length" class="muted">该需求还没有代码模块</p>
      </NTabPane>


      <!-- ── 设置 ── -->
      <NTabPane name="meta" tab="需求设置">
        <NCard size="small" class="meta">
          <div v-if="!editing" class="meta-view">
            <div class="kv"><span class="k">需求名称</span><span>{{ meta?.title }}</span></div>
            <!-- 留空就是主干。以前这里显示成「主干」，看着像真填了个值，
                 改的时候反而不知道该不该删（实测反馈 #7）。 -->
            <div class="kv">
              <span class="k">基线来源</span>
              <span v-if="meta?.base?.length" class="mono">{{ meta.base.join(" → ") }}</span>
              <span v-else class="muted">未设置（即主干）</span>
            </div>
            <div class="kv"><span class="k">创建日期</span><span>{{ meta?.created }}</span></div>
            <div v-if="meta?.doc" class="kv">
              <span class="k">需求分类</span>
              <!-- **只显示分类名**。period 存的就是分支名，跟在后面等于把分支
                   又抄一遍（而且分支在头部/需求名称那边已经有了）——实测反馈。 -->
              <span>{{ meta.doc.family }}</span>
            </div>
            <div v-if="meta?.notes" class="kv"><span class="k">备注</span><span>{{ meta.notes }}</span></div>
            <div class="kv">
              <span class="k">自定义信息</span>
              <span>
                <LinkList v-if="meta?.links?.length" :links="meta.links" />
                <span v-else class="muted">未设置</span>
              </span>
            </div>
            <div class="actions">
              <NButton size="small" @click="startEdit">修改</NButton>
            </div>
          </div>

          <div v-else class="meta-edit">
            <label>需求名称</label>
            <NInput v-model:value="titleDraft" size="small"  :input-props="NO_AUTO"/>
            <label>基线来源</label>
            <BaseEditor ref="baseEditor" v-model="baseList" />
            <label></label>
            <p class="muted small">
              拉基线和「同步基线」都**按这个顺序找**：第一个有该模块分支的用它，
              没有就退到下一个，都没有才用主干。
            </p>
            <label>备注</label>
            <NInput v-model:value="notesDraft" type="textarea" size="small" :rows="3"  :input-props="NO_AUTO"/>
            <label>自定义信息</label>
            <LinkEditor
              v-model="linksDraft"
              :placeholders="['如 tapd / 飞书文档 / 监控', 'https://…']"
            />
            <div class="actions">
              <NButton size="small" @click="editing = false">取消</NButton>
              <NButton size="small" type="primary" @click="commitEdit">保存</NButton>
            </div>
          </div>
        </NCard>

        <NCard size="small" class="meta danger">
          <div class="danger-row">
            <div>
              <div class="danger-title">删除需求</div>
              <div class="muted small">
                会删掉本地工作目录与本地分支。有未保存的工作时会先列出风险项让你确认。
              </div>
            </div>
            <NButton size="small" type="error" @click="removeWs">删除</NButton>
          </div>
        </NCard>
      </NTabPane>
    </NTabs>

    <NSpin v-else-if="loading" class="center" />
    <p v-else class="muted">加载中…</p>

    <!-- 操作弹窗 -->
    <OpDialog
      v-if="op"
      :key="op"
      :title="opTitle"
      :hint="opHint"
      :modules="opModules"
      :envs="op === 'merge' || op === 'push-env' ? app.envs : undefined"
      :preselect="opPreselect"
      :with-push="op === 'merge' || op === 'push-env'"
      :push-by-default="op === 'push-env'"
      :with-force="op === 'merge'"
      :with-rebase="op === 'pull'"
      :action="runOp"
      @close="op = null; opPreselect = null"
      @done="afterOp"
    />

    <MergeCheck
      v-if="showMergeCheck"
      :branch="branch"
      :title="meta?.title"
      @close="showMergeCheck = false"
    />

    <!-- 快捷入口：与总览页同一组入口（地址来自 hub 配置），点哪个跳哪个 -->
    <NModal
      v-if="showQuickLinks"
      :show="true"
      @update:show="(v: boolean) => !v && (showQuickLinks = false)"
      preset="card"
      title="快捷入口"
      style="width: 560px"
      @close="showQuickLinks = false"
    >
      <QuickLinks :links="quickLinks" @pick="showQuickLinks = false" />
    </NModal>

    <!-- 提 MR（模块标签页里的一个功能） -->
    <NModal
      v-if="showMr"
      :show="true"
      @update:show="(v: boolean) => !v && (showMr = false)"
      preset="card"
      title="提 Merge Request 到主干"
      style="width: 1100px"
      @close="showMr = false"
    >
      <!-- 给的是 `opModules`（**含文档库**），不是上面那个滤掉文档库的 `modules`。
           文档库在工作区里是一个真实的 worktree，需求文档也得跟着提 MR；
           实测反馈：提 MR 时列表里根本没有 doc-hub，于是少提了一个
           （而且后端那一层也会把它静默丢掉，见 core 的 `mr::selected_modules`）。 -->
      <MrPanel :branch="branch" :modules="opModules" @changed="emit('changed')" />
    </NModal>

    <!-- 并入主干检查的结果。原来塞进一个 dialog.info 拼长文本，又难看又看不出
         哪些查过哪些没查（反馈 #5）。 -->
    <NModal
      v-if="doneLines || doneRunning"
      :show="true"
      @update:show="(v: boolean) => !v && (doneLines = null)"
      preset="card"
      title="并入主干检查"
      style="width: 660px"
      @close="doneLines = null"
    >
      <p v-if="doneRunning" class="muted small">正在逐个模块跟主干比对（每个都要 fetch，稍等）…</p>
      <ActionResult v-else class="mt" :lines="doneLines!" title="检查结果" />
      <template #footer>
        <div class="footer">
          <NButton v-if="doneRunning" :disabled="true">检查中…</NButton>
          <NButton v-else type="primary" @click="doneLines = null">知道了</NButton>
        </div>
      </template>
    </NModal>

    <!-- 文件浏览与对比（需求 #18） -->
    <NModal
      v-if="browse"
      :show="true"
      @update:show="(v: boolean) => !v && (browse = null)"
      preset="card"
      :title="browse.mode === 'tree' ? `浏览文件 · ${browse.module}` : `对比代码 · ${browse.module}`"
      style="width: 92vw; height: 88vh"
      @close="browse = null"
    >
      <FileBrowser
        :dir="`${wsPath}/${browse.module}`"
        :repo="browse.module"
        :branch="branch"
        :label="browse.module"
        :ws-branch="branch"
        :default-mode="browse.mode"
      />
    </NModal>
    <!-- 「需求相关文档」：本次需求的文档（不含归档）——点开就能看，不用绕到「文档」页 -->
    <NModal
      v-model:show="showDoc"
      preset="card"
      :title="`需求相关文档 · ${meta?.title ?? branch}`"
      style="width: 680px"
      @close="showDoc = false"
    >
      <NSpin v-if="docLoading" size="small" />
      <NAlert v-else-if="docErr" type="warning" :bordered="false">
        这次需求还没有可看的文档目录。
        <div class="small">{{ docErr }}</div>
      </NAlert>
      <template v-else-if="doc">
        <div class="docmeta">
          <div class="kv"><span class="k">分支</span><span class="mono">{{ doc.meta.branch }}</span></div>
          <div class="kv"><span class="k">需求名</span><span>{{ doc.meta.name }}</span></div>
          <div class="kv"><span class="k">日期</span><span>{{ doc.meta.date }}</span></div>
          <div class="kv"><span class="k">涉及模块</span><span>{{ doc.meta.repos.join("、") || "—" }}</span></div>
        </div>
        <div class="docpanel">
          <template v-if="doc.has_content">
            <div class="muted small mb">这一份文档目录（全展开）</div>
            <DocTree v-if="docNodes.length" :nodes="docNodes" />
            <p v-else class="muted small">这个期次目录还是空的</p>
          </template>
          <p v-else class="muted small">
            该需求还没有需求文档，只有工具生成的说明文件（meta.json / AGENTS.md / CLAUDE.md）。
            写好的文档放进这个目录后，这里就会出现。
          </p>
        </div>
        <div v-if="doc.pending.length" class="pending">
          <span class="muted small">
            有 {{ doc.pending.length }} 处改动还没提交：{{ doc.pending.join("、") }}
          </span>
          <NButton
            size="tiny"
            type="primary"
            :loading="docSubmitting"
            @click="submitDoc"
          >
            提交并上传
          </NButton>
        </div>
      </template>
      <template #footer>
        <div class="footer">
          <NButton @click="openInFinder(doc?.dir ?? wsPath)">打开目录</NButton>
          <EditorButton size="medium" :path="doc?.dir ?? wsPath" :secondary="false" />
          <NButton type="primary" @click="showDoc = false">关闭</NButton>
        </div>
      </template>
    </NModal>


    <!-- 危险操作：先预览、手输分支名、才执行 -->
    <DangerDialog
      v-if="danger === 'revert'"
      title="撤销工作区改动"
      hint="丢弃本需求各模块里**未提交**的改动。分支、提交都不动。"
      :confirm-text="branch"
      :preview="revertPlan"
      :apply="revertApply"
      @close="danger = null"
      @done="afterOp"
    />

    <DangerDialog
      v-else-if="danger === 'reset'"
      title="强制回退到远程"
      hint="把本需求拉回远程分支的样子，**连本地提交一起丢**。用于远程分支被重建/回退后跟上。"
      :confirm-text="branch"
      :preview="resetPlan"
      :apply="resetApply"
      @close="danger = null"
      @done="afterOp"
    />

    <DangerDialog
      v-else-if="danger === 'prune'"
      title="清理远程需求分支"
      hint="删除远程上已合并的需求分支。**只删「已并入主干且本地无未推送提交」的**，其余会列出来但不删。"
      :confirm-text="branch"
      :preview="prunePlan"
      :apply="pruneApply"
      @close="danger = null"
      @done="afterOp"
    />

    <DangerDialog
      v-else-if="danger === 'rename'"
      ref="renameDlg"
      title="修改需求分支名"
      hint="会改本地分支名、推新分支、删旧远程分支，并重命名工作区目录。改完别处的同名工作区就对不上了，需要重拉。"
      :confirm-text="branch"
      :disable-continue="!renameInput.trim() || renamePreviewStale"
      :preview="renamePlan"
      :apply="applyRename"
      @close="danger = null"
      @done="afterOp"
    >
      <template #extra>
        <div class="sec-title">新分支名</div>
        <NInput
          v-model:value="renameInput"
          :input-props="NO_AUTO"
          placeholder="如 feature-20261012-新需求名"
        />
        <p class="muted small">
          改名后<strong>别处的同名工作区就对不上了</strong>：旧目录名得删掉重拉。
          工作区目录名就是分支名，所以目录也会一起改。
        </p>
      </template>
    </DangerDialog>

    <!-- 增加模块。候选要**用含文档库那份（allModules）过滤**：实测反馈，文档库已经
         拉下来了这里却还列着它 —— `modules` 把 doc-hub 滤掉了（它只出现在
         置顶那一行），拿它判断"在不在工作区里"必然漏判。
         （备注写在这里而不是表达式里：多行属性里的 `//` 会把后面整段吞掉。） -->
    <OpDialog
      v-if="showAdd"
      title="增加模块"
      hint="新模块会从需求基线拉一个同名分支出来。**不会默认勾选**任何模块——选好再点执行。"
      empty-by-default
      :modules="
        moduleRepos(app.repos)
          .filter((r) => !allModules.some((m) => m.module === r.name))
          .map((r) => ({
        module: r.name,
        present: true,
        dirty: 0,
        pushed: false,
        ahead: 0,
        behind: 0,
        ahead_of_main: null,
        error: null,
      }))"
      :action="async (p) => {
        const rs = await api.wsAddModules(branch, p.modules);
        return fmt.repoLines(rs);
      }"
      @close="showAdd = false"
      @done="afterOp"
    />
  </div>
</template>

<style scoped>
/* 未提交的文档改动：一句说明 + 提交按钮，同一行 */
.pending {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  margin-top: 10px;
}
/* 「需求相关文档」弹窗：期次信息 + 目录树 */
.docmeta {
  margin-bottom: 12px;
}
.docpanel {
  border: 1px solid var(--xg-border);
  border-radius: 6px;
  padding: 10px 12px;
  max-height: 46vh;
  overflow: auto;
}
.mb {
  margin-bottom: 6px;
}
/* 头部：需求分类与基线都比标题弱一档，它们是补充信息 */
.fam {
  flex: none;
}
.base {
  font-size: 12px;
  color: #888;
  flex: none;
}
.head {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 12px;
  flex-wrap: wrap;
}
.title {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
}
.branch {
  color: #888;
}
.spacer {
  flex: 1;
}
.ws-note {
  margin: 0;
  white-space: pre-wrap;
}
.ws-links {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 8px;
  margin-bottom: 10px;
  padding: 8px 12px;
  border: 1px solid var(--xg-border);
  border-radius: 6px;
  background: var(--xg-fill-soft);
}
.err {
  margin-bottom: 12px;
  white-space: pre-wrap;
  font-size: 12px;
}
.ops {
  display: flex;
  gap: 8px;
  align-items: center;
  margin-bottom: 12px;
  flex-wrap: wrap;
}
.tbl {
  width: 100%;
  border-collapse: collapse;
  font-size: 13px;
}
.tbl th,
.tbl td {
  text-align: left;
  padding: 6px 8px;
  border-bottom: 1px solid var(--xg-border);
}
.tbl th {
  color: #888;
  font-weight: 500;
  font-size: 12px;
}
.num {
  text-align: right;
  font-variant-numeric: tabular-nums;
}
.warn {
  color: #f0a020;
  font-weight: 600;
}
.err-text {
  color: #d03050;
}
/* 操作列按内容自然撑开。写 width:1% 会把它压到最小宽度、按钮被迫换行，
   而其它列也不会自动铺满 —— 实测反馈「宽度很奇怪、操作被换行、表格没铺满」 */
.actions {
  white-space: nowrap;
}
.tbl {
  table-layout: auto;
}
/* 行内操作按钮成组，看起来是按钮而不是散落的文本（反馈 #6） */
.btns {
  display: inline-flex;
  gap: 6px;
  flex-wrap: wrap;
  align-items: center;
}
/* 头部的远程探测状态：**只占一行、超出省略** —— 它绝不能把下面的表格挤来挤去 */
.probe {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  max-width: 34ch;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.warn-note {
  color: #d97706;
}

/* 置顶行（文档）。跟仓储页的 doc-hub 行一样，用底色区分出「它特殊但仍在表内」 */
.tbl tr.pinned td {
  background: rgba(37, 99, 235, 0.05);
}
.tbl td {
  white-space: nowrap;
}
.footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
.muted {
  color: #888;
  font-size: 12px;
}
.small {
  font-size: 12px;
}
.center {
  display: block;
  margin: 40px auto;
}
.meta {
  margin-bottom: 12px;
}
.kv {
  display: grid;
  grid-template-columns: 6em 1fr;
  gap: 8px;
  padding: 4px 0;
  font-size: 13px;
}
.k {
  color: #888;
}
.meta-edit {
  display: grid;
  grid-template-columns: 6em 1fr;
  gap: 8px 10px;
  align-items: center;
  font-size: 13px;
}
/* ⚠️ **必须限定作用域**：表格的操作单元格也叫 `.actions`（`<td class="actions">`）。
   写成裸 `.actions` 的话，这条 `display: flex` 会作用到那个 `<td>` 上 ——
   而 `display:flex` 的 `<td>` **不再是表格单元格**，它的高度自成一套，
   于是操作列的横向分割线跟其它列**错位**（实测反馈：就这一列的线对不上）。
   条件只该给「需求设置」面板里那行按钮。 */
.meta .actions,
.meta-edit .actions {
  display: flex;
  gap: 8px;
  justify-content: flex-end;
}
.meta-edit .actions {
  grid-column: 1 / -1;
}
.meta-edit .muted {
  grid-column: 2;
}
.danger-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 16px;
}
.danger-title {
  font-weight: 500;
}
</style>