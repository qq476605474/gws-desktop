<script setup lang="ts">
import { useCopy } from "../lib/copy";
import { computed, onMounted, ref, watch } from "vue";
import {
  NAlert,
  NButton,
  NCheckbox,
  NInput,
  NModal,
  NRadioButton,
  NRadioGroup,
  NSelect,
  NTabPane,
  NTabs,
  NTag,
  useDialog,
  useMessage,
} from "naive-ui";

import { NO_AUTO } from "../lib/inputs";
import { checkedOut, confirmedEmpty, type EnvDirs } from "../lib/envDirs";
import * as api from "../bridge";
import { openInFinder, openUrl } from "../bridge";
import { useAppStore } from "../stores/app";
import { withBusy } from "../busy";
import * as fmt from "../format";
import { gitlabBranchUrl } from "../types";
import { overrideRepoOptions } from "../lib/envOverride";
import type { EnvRow, GitlabProject, Link, RepoRow, SharedRepo } from "../types";
import RepoDetail from "../components/RepoDetail.vue";
import EditorButton from "../components/EditorButton.vue";
import SyncDialog from "../components/SyncDialog.vue";
import FileBrowser from "../components/FileBrowser.vue";
import LinkEditor from "../components/LinkEditor.vue";
import RepoInfoInline from "../components/RepoInfoInline.vue";

/**
 * 由**左侧菜单**指定「只显示哪一栏」。
 *
 * 实测反馈：想把「环境」从仓库页里拆出来、在菜单里独立成一项（放在「仓库」下面）。
 *
 * 这里**不把环境那部分逻辑搬去新组件** —— 它和仓库共用了不少东西
 * （同步弹窗、环境补建状态、`loadEnvDirs`），搬一次风险不小（搬崩过一次，有前科）。
 * 做法是：菜单决定只显示哪一栏，并把 tab 条藏起来 —— 使用者看到的就是两个独立页面。
 * 组件名仍叫 Repos，因为它确实同时管着仓库与环境；真要拆干净是另一件事。
 */
const props = defineProps<{ only?: "repos" | "envs" }>();

const app = useAppStore();
/** 文档库在 hub 里的目录名（从配置读，别写死）。 */
const docName = computed(() => app.hub?.dochub_name ?? "doc-hub");

/** 顶部「总的」打开目录指向所有仓库的父目录。 */
const reposRoot = computed(() => (app.hub ? `${app.hub.root}/repos` : ""));
const emit = defineEmits<{ (e: "gotoWs", branch: string): void }>();
const msg = useMessage();
const copy = useCopy();
const dialog = useDialog();

/** 当前标签。从总览跳过来时会带上目标标签。 */
const tab = ref<string>(props.only ?? "repos");
// only 给了就锁死（菜单决定页面），不再听 reposTab ——
// 「从总览跳到环境」由 App.vue 直接切菜单项，不在这里绕。
watch(
  () => props.only,
  (v) => {
    if (v) tab.value = v;
  },
  { immediate: true },
);
watch(
  () => (props.only ? null : app.reposTab),
  (v) => {
    if (!v) return;
    tab.value = v;
    app.reposTab = null; // 一次性意图，读完就清
  },
  { immediate: true },
);

const repoQuery = ref("");

/**
 * 「添加仓库」是否打开。
 *
 * ⚠️ **打开时一定要走 `openAdd()` 把表单清干净**，不要直接 `showAdd = true`：
 * 表单是常驻的 ref，不清的话上次填的地址、搜过的结果、勾过的仓库全都还在，
 * 下次打开像是"恢复了草稿"，很容易误提（实测反馈）。
 */
const showAdd = ref(false);
const addMode = ref<"manual" | "gitlab" | "paste">("gitlab");
const manualUrls = ref("");

/** 打开「添加仓库」，**表单重置为初始状态**。 */
function openAdd() {
  addMode.value = "gitlab";
  manualUrls.value = "";
  glQuery.value = "";
  glResults.value = [];
  glPicked.value = [];
  glErr.value = "";
  shareText.value = "";
  sharedRepo.value = null;
  shareErr.value = "";
  showAdd.value = true;
}
/**
 * 「粘贴配置文本」那一栏。
 *
 * 在仓库详情里点「导出配置」得到一段文本，粘到这里就能导入 ——
 * 仓库地址、各环境发布地址、自定义链接全都跟着过来（实测反馈）。
 */
const shareText = ref("");
/** 识别出来的那份仓库配置（非 null = 现在走「按这份配置导入」）。 */
const sharedRepo = ref<SharedRepo | null>(null);
const shareErr = ref("");
const sharing = ref(false);
const glQuery = ref("");
const glResults = ref<GitlabProject[]>([]);
const glPicked = ref<string[]>([]);
const glLoading = ref(false);
const glErr = ref("");
/** 正在查看详情的仓库。 */
const detailRepo = ref<RepoRow | null>(null);
/** 同步对话框：`{ env: null }` = 同步主干；`{ env: "pre" }` = 同步该环境。 */
const syncTarget = ref<{ env: string | null } | null>(null);
/** 打开同步弹窗时预勾的仓库（改完覆盖只同步受影响的那几个）。 */
const syncPreselect = ref<string[] | null>(null);

/** 环境分支覆盖的编辑草稿：`环境标识 → [仓库, 分支][]` */
const draft = ref<Record<string, [string, string][]>>({});
/** 覆盖表的校验错误：`环境标识 → 错误文案` */
const draftErr = ref<Record<string, string>>({});
/**
 * 只看哪几个环境。
 *
 * 做成**平铺复选框、默认全选**：环境本来就没几个（三五个），手动输入过滤
 * 的可能性很低，而复选框一眼看到全部、点一下就切换（实测反馈）。
 */
/**
 * 只看哪一个环境（**单选**）。
 *
 * 环境之间是「切换着看」的关系，不是「并排比较」——同时铺开几个环境，
 * 每个都带一张卡片、一套按钮，屏幕上全是重复结构（实测反馈）。
 * 默认选第一个。
 */
const envPick = ref<string>("");

// ── 添加环境弹窗 ──
const showAddEnv = ref(false);
const newEnvId = ref("");
const newEnvLabel = ref("");
const newEnvOverrides = ref<[string, string][]>([]);

/** 文档库由置顶行单独渲染，这里排除掉，免得同一个仓库出现两遍。 */
const docRow = computed(() => app.repos.find((r) => r.is_doc) ?? null);
const repos = computed(() => {
  const s = repoQuery.value.trim().toLowerCase();
  return app.repos
    .filter((r) => !r.is_doc)
    .filter(
      (r) => !s || r.name.toLowerCase().includes(s) || r.remote_url.toLowerCase().includes(s),
    );
});

onMounted(() => {
  void app.refreshAll();
  void loadRepoInfo();
});

// ══ 每个仓库的自定义信息（链接 + 备注）══════════════════════════════════
//
// 实测反馈：这块原来只藏在「仓库详情」弹窗里，仓库列表上看不到 —— 而它的用处
// 恰恰是**随手点**（dev/prod 的 Jenkins、常见模块域名）。所以两件事：
// 列表里直接显示，文本条目点了能复制。
//
// 数据存 `deployments.json`（`deploy_matrix` 读、`deploy_set_repo_info` 写），
// 与仓库的结构性配置分开 —— 它是「经常改的自定义数据」。

/** `仓库名 → 自定义信息`。没配过的仓库不在表里。 */
const repoInfo = ref<Record<string, { notes: string; links: Link[] }>>({});

/** 取某个仓库的信息，没有就给空值 —— 模板里到处判 `undefined` 太吵。 */
function infoOf(name: string): { notes: string; links: Link[] } {
  return repoInfo.value[name] ?? { notes: "", links: [] };
}

async function loadRepoInfo() {
  try {
    const all = await api.deployMatrix();
    repoInfo.value = Object.fromEntries(
      all.map((r) => [r.name, { notes: r.notes ?? "", links: r.links ?? [] }]),
    );
  } catch {
    // 读不到就当都没配过：这是可选数据，不该让仓库页报错
    repoInfo.value = {};
  }
}

// ── 编辑弹窗 ──
const showInfo = ref(false);
const infoRepo = ref<string>("");
const editNotes = ref("");
const editLinks = ref<Link[]>([]);
const savingInfo = ref(false);

function openInfo(name: string) {
  infoRepo.value = name;
  const i = infoOf(name);
  editNotes.value = i.notes;
  editLinks.value = i.links.map((l) => ({ ...l }));
  showInfo.value = true;
}

async function saveRepoInfo() {
  savingInfo.value = true;
  try {
    await api.deploySetRepoInfo({
      repo: infoRepo.value,
      notes: editNotes.value,
      // 空行不存：LinkEditor 允许临时加一行又没填，存进去会变成列表里一个空标签
      links: editLinks.value.filter((l) => l.label.trim() || l.url.trim()),
    });
    msg.success("已保存");
    showInfo.value = false;
    await loadRepoInfo();
  } catch (e) {
    msg.error(api.errText(e));
  } finally {
    savingInfo.value = false;
  }
}

// ══ 仓库 ══════════════════════════════════════════════════════════════

async function searchGitlab() {
  glLoading.value = true;
  glErr.value = "";
  try {
    glResults.value = await api.gitlabSearchProjects(glQuery.value, 50);
  } catch (e) {
    glErr.value = api.errText(e);
    glResults.value = [];
  } finally {
    glLoading.value = false;
  }
}

/**
 * 识别粘贴进来的仓库配置。
 *
 * **不直接导入**：先把「会做什么」摆出来（clone 这个仓库 / 补上哪几项自定义信息），
 * 补的仓库要联网克隆，闷头跑会让人以为卡死。
 */
async function recognizeRepo() {
  sharing.value = true;
  shareErr.value = "";
  try {
    sharedRepo.value = await api.repoShareParse(shareText.value);
  } catch (e) {
    sharedRepo.value = null;
    shareErr.value = api.errText(e);
  } finally {
    sharing.value = false;
  }
}

/** 按对方那份配置导入（缺就克隆，自定义信息只补空、不覆盖）。 */
async function doImportRepo() {
  const s = sharedRepo.value;
  if (!s) return;
  try {
    const r = await withBusy(`导入仓库 ${s.name}`, () => api.repoShareImport(s));
    msg.success(r.added ? `已添加仓库 ${r.name}` : `仓库 ${r.name} 本地已有`);
    if (r.info_applied) msg.success(`补上 ${r.info_applied} 项自定义信息`);
    if (r.kept_local.length) {
      // **如实说**：本地已有的一律不覆盖，别让人以为「导入成功了」就什么都没漏
      msg.info(`本地已配过、未覆盖：${r.kept_local.join("、")}（要改用对方那份，在仓库详情里自己改）`);
    }
    if (r.skipped_envs.length) {
      msg.info(`本地没有这些环境，地址已跳过：${r.skipped_envs.join("、")}`);
    }
    for (const [what, why] of r.env_failures) msg.warning(`环境补齐失败 ${what}：${why}`);
    showAdd.value = false;
    shareText.value = "";
    sharedRepo.value = null;
    await app.refreshAll();
    await loadEnvDirs();
    await loadRepoInfo();
  } catch (e) {
    msg.error(api.errText(e));
  }
}

/**
 * 把某个仓库的配置复制到剪贴板（发给对方，粘一下就能导入）。
 *
 * 跟「仓库详情」里那个按钮是同一件事，只是**多一个入口** —— 那个藏在弹窗里，
 * 实测反馈是找不到。
 */
async function copyRepoShare(r: RepoRow) {
  try {
    const text = await api.repoShare(r.name);
    await copy(text, "仓库配置");
    msg.success("配置已复制；对方粘到「添加仓库 → 粘贴配置文本」里即可导入");
  } catch (e) {
    msg.error(api.errText(e));
  }
}

async function doAdd() {
  if (addMode.value === "paste") {
    await doImportRepo();
    return;
  }
  const urls =
    addMode.value === "manual"
      ? manualUrls.value.split(/[\s,，]+/).filter(Boolean)
      : glResults.value
          .filter((p) => glPicked.value.includes(p.path_with_namespace))
          .map((p) => p.http_url_to_repo || p.ssh_url_to_repo);
  if (!urls.length) {
    msg.warning("没有选中或填写任何仓库");
    return;
  }
  try {
    // 加完仓库会**自动把环境补齐**（新增仓库在每个环境里都缺一份检出），
    // 所以这一步比单纯的 clone 慢，提示里说清是在做两件事
    const r = await withBusy(`添加 ${urls.length} 个仓库并补齐环境`, () => api.repoAdd(urls));
    const okCount = r.repos.filter((x) => x.outcome === "Ok").length;
    msg.success(`已添加 ${okCount} 个仓库`);
    if (r.env_created.length) {
      msg.success(`已在环境里补建 ${r.env_created.length} 份检出`);
    }
    for (const [what, why] of r.env_failures) {
      msg.warning(`环境补齐失败 ${what}：${why}`);
    }
    showAdd.value = false;
    glPicked.value = [];
    await app.refreshAll();
    await loadEnvDirs();
    await loadRepoInfo();
  } catch (e) {
    msg.error(api.errText(e));
  }
}

/**
 * 移除仓库。
 *
 * **只有「被需求引用」才拦** —— 环境里的是镜像，删仓库时一并清掉。
 * （原来把环境也算进去，于是提示「先把 dev 环境删掉」，而环境是常驻的，
 *  等于根本删不掉。实测反馈。）
 */
/** 环境标识 → 中文显示名。提示里报中文名，`envs/dev` 这种路径使用者认不出来。 */
function envName(id: string): string {
  const hit = app.hub?.envs.find(([eid]) => eid === id);
  return hit?.[1] || id; // 没配显示名就退回标识本身
}

function rmRepo(name: string, usages: string[], envUsages: string[]) {
  if (usages.length) {
    dialog.warning({
      title: `无法移除 ${name}`,
      content: `它仍被这些需求使用：${usages.join("、")}。先删掉对应需求，或从需求里移除这个模块。`,
      positiveText: "知道了",
    });
    return;
  }
  const envNames = envUsages.map((u) => envName(u.replace(/^envs\//, "")));
  const envNote = envNames.length
    ? `\n各环境里的检出（${envNames.join("、")}）会一并清掉 —— 下次同步会重建。`
    : "";
  dialog.warning({
    title: `移除仓库 ${name}`,
    content: `会删掉本地这份仓库（含全部分支与提交记录）。${envNote}\n确认移除？`,
    positiveText: "移除",
    negativeText: "取消",
    onPositiveClick: async () => {
      try {
        await withBusy(`移除 ${name}`, () => api.repoRemove([name]));
        msg.success(`${name} 已移除`);
        await app.refreshAll();
        await loadRepoInfo();
      } catch (e) {
        msg.error(api.errText(e));
      }
    },
  });
}

/** 拉取/刷新文档库。首次是克隆，可能要几十秒——走 busy 有进度可见。 */
async function pullDochub() {
  try {
    const r = await withBusy("拉取文档库", () => api.dochubEnsure());
    msg.success("文档库已就绪");
    if (r.env_created.length) msg.success(`已在环境里检出 ${r.env_created.length} 份`);
    for (const [what, why] of r.env_failures) {
      msg.warning(`环境检出失败 ${what}：${why}`);
    }
    await app.refreshAll();
  } catch (e) {
    msg.error(api.errText(e));
  }
}

// ── 改环境（标识 + 显示名）──
const showEnvMeta = ref(false);
const envMetaOld = ref("");
const envMetaLabel = ref("");

function startEditEnvMeta(e: EnvRow) {
  envMetaOld.value = e.id;
  envMetaLabel.value = e.label ?? "";
  showEnvMeta.value = true;
}

async function saveEnvMeta() {
  try {
    // 标识原样传回（服务端也支持改，但界面不再给入口 —— 见上面的注释）
    await withBusy(`修改环境 ${envMetaOld.value}`, () =>
      api.envUpdate(envMetaOld.value, envMetaOld.value, envMetaLabel.value.trim()),
    );
    msg.success("环境已更新");
    showEnvMeta.value = false;
    await app.refreshAll();
  } catch (e) {
    msg.error(api.errText(e));
  }
}

/** 正在浏览文件的目录（仓库/环境/文档都用它）。 */
const browse = ref<{ dir: string; repo: string; label: string; mode: "tree" | "changed" } | null>(null);

/** 打开同步对话框。`env=null` 同步主干，否则同步该环境。 */
/** `preselect` 为空表示全选；给了就只勾这些。 */
function openSync(env: string | null = null, preselect?: string[]) {
  syncTarget.value = { env };
  syncPreselect.value = preselect ?? null;
}

/** 每个项目的说明：同步场景关心的是「会切到哪条分支」，跟改动状态无关。 */
const syncHints = computed(() => {
  const env = syncTarget.value?.env;
  const out: Record<string, string> = {};
  for (const r of app.repos) {
    if (!env) {
      out[r.name] = `主干 ${r.main_branch}`;
    } else {
      const row = app.envs.find((e) => e.id === env);
      const eff = row?.effective.find(([m]) => m === r.name)?.[1];
      out[r.name] = eff ? `环境分支 ${eff}` : "该环境无此仓库";
    }
  }
  return out;
});

/**
 * 同步列表 = `repos/` 下的全部仓库。
 *
 * 文档库现在也在 `repos/doc-hub`（实测反馈 #9），所以它就是列表里的一员，
 * 由 `sync_repos` 一起同步——不再需要之前那段「单独追加 + 单独 sync」的特例。
 */
const syncNames = computed(() => app.repos.map((r) => r.name));

const syncTitle = computed(() =>
  syncTarget.value?.env ? `同步环境 ${syncTarget.value.env}` : "同步仓库主干",
);

const syncHintText = computed(() => {
  if (syncTarget.value?.env) {
    return `把 envs/${syncTarget.value.env}/ 下的 worktree 拉到各自环境分支的最新提交。不影响主干。`;
  }
  return "把 repos/ 下的主干镜像拉到各自主干分支的最新提交。不动任何环境。";
});

async function runSync(only: string[]) {
  const env = syncTarget.value?.env ?? null;
  const filter = { only };
  const out = env ? await api.syncEnv(env, filter) : await api.syncRepos(filter);
  const lines = fmt.syncLines(out);
  await app.refreshAll();
  // 同步会**新建** envs/<环境>/<模块> 这些 worktree 目录，所以要把
  // 「检出了没有」这份清单重新读一遍，否则「未检出」的标记会一直是旧的
  await loadEnvDirs();
  return lines;
}

// ══ 环境 ══════════════════════════════════════════════════════════════

const envs = computed(() => app.envs.filter((e) => e.id === envPick.value));

/**
 * `环境标识 → envs/<标识>/ 下**实际存在**的目录名`（`null` = 没读出来）。
 *
 * 环境页列的是**配置**，而目录是**同步时**才建出来的：一个还没同步过的环境
 * 一个子目录都没有，界面上却照样给每一行配「打开目录」，点了当然没反应
 * （实测反馈）。这份清单让「还没检出」当场可见。
 *
 * 判定逻辑在 `lib/envDirs`：读不出来时按「未知」处理（按钮照旧可用），
 * 宁可让人点一下看到报错，也不能一次列目录失败就把按钮全禁掉。
 */
const envDirs = ref<EnvDirs>({});

async function loadEnvDirs() {
  const root = app.hub?.root;
  if (!root) return;
  const out: EnvDirs = {};
  for (const e of app.envs) {
    try {
      out[e.id] = await api.listDir(`${root}/envs/${e.id}`);
    } catch {
      // 读不出来就是 `null`（未知）——**不是**空数组
      out[e.id] = null;
    }
  }
  envDirs.value = out;
}

/** 这个模块在这个环境里检出了没有（未知 → 当作有）。 */
const isCheckedOut = (envId: string, mod: string) => checkedOut(envDirs.value, envId, mod);

/** 确认一个都没检出的环境（用来提示「先同步」）。 */
const emptyEnvs = computed(() => envs.value.filter((e) => confirmedEmpty(envDirs.value, e.id)));

watch(
  () => app.envs.map((e) => e.id).join(","),
  () => {
    if (!app.envs.some((e) => e.id === envPick.value)) {
      envPick.value = app.envs[0]?.id ?? "";
    }
    void loadEnvDirs();
  },
  { immediate: true },
);

function startEditEnv(e: EnvRow) {
  draft.value[e.id] = e.overrides.map((p) => [...p] as [string, string]);
  draftErr.value[e.id] = "";
}

function addOverride(id: string) {
  draft.value[id] = [...(draft.value[id] ?? []), ["", ""]];
}

function setOverride(id: string, idx: number, which: 0 | 1, v: string) {
  const list = (draft.value[id] ?? []).map((p) => [...p] as [string, string]);
  if (!list[idx]) list[idx] = ["", ""];
  list[idx][which] = v;
  draft.value[id] = list;
  draftErr.value[id] = validateOverrides(list);
}

function delOverride(id: string, idx: number) {
  draft.value[id] = (draft.value[id] ?? []).filter((_, i) => i !== idx);
  draftErr.value[id] = validateOverrides(draft.value[id] ?? []);
}

/**
 * 覆盖表的校验。
 *
 * 实测反馈 #8：同一个仓库填了两遍（`demo-web = pre-202610` 重复出现），
 * 之前照单全收——结果是两条互相矛盾的规则，哪条生效取决于顺序，界面上还看不出来。
 * **重复仓库名必须拦住**。
 */
function validateOverrides(list: [string, string][]): string {
  const seen = new Map<string, number>();
  for (const [repo] of list) {
    const r = repo.trim();
    if (!r) continue;
    seen.set(r, (seen.get(r) ?? 0) + 1);
  }
  const dup = [...seen.entries()].filter(([, n]) => n > 1).map(([r]) => r);
  if (dup.length) {
    return `仓库重复填写：${dup.join("、")}。同一个仓库只能有一条覆盖规则。`;
  }
  if (seen.size > 1 && seen.has("*")) {
    return "填了 *（全部仓库）就不能再单独指定某个仓库，否则规则会打架。";
  }
  return "";
}

async function saveEnv(id: string) {
  const clean = (draft.value[id] ?? []).filter(([r, b]) => r.trim() && b.trim());
  const verr = validateOverrides(clean);
  if (verr) {
    draftErr.value[id] = verr;
    return;
  }

  // ① 先校验分支真的存在 —— 填错分支名如果放过去，下次「同步此环境」时
  //    worktree 会切到一个不存在的分支上，那时候报错离现场很远、很难查
  //    （实测反馈 #18）
  let checks: import("../types").BranchCheck[] = [];
  try {
    checks = await withBusy(`校验 ${id} 的分支是否存在`, () => api.envCheckBranches(clean));
  } catch (e) {
    msg.error(`校验失败，未保存：${api.errText(e)}`);
    return;
  }
  const missing = checks.filter((c) => !c.exists && !c.error);
  const unknown = checks.filter((c) => c.error);
  if (missing.length) {
    draftErr.value[id] =
      "以下分支在远程不存在，未保存：\n" +
      missing.map((c) => `  ${c.repo} → ${c.branch}`).join("\n") +
      "\n（先在对应仓库建出这条分支，或改成分支名）";
    return;
  }
  if (unknown.length) {
    // 探测失败 != 不存在。这时候拦住是错的（网络问题不该阻止保存），
    // 但要让用户知道「没验成」
    msg.warning(
      `有 ${unknown.length} 条没查成（网络问题），已按「先保存」处理：` +
        unknown.map((c) => `${c.repo}→${c.branch}`).join("、"),
    );
  }

  // ② 只保存覆盖表（本地写配置，秒回）
  //
  // 之前这里顺手串了一次 syncAll，两个问题：
  // - 同步的进度事件会把忙碌提示覆盖成「同步」，用户以为保存没执行
  // - 一次「保存」里塞 22 个仓库的同步，是用户没要求的重操作
  try {
    await api.envSetOverrides(id, clean);
  } catch (e) {
    msg.error(api.errText(e));
    return;
  }
  msg.success(`${id} 的分支名覆盖已保存`);
  delete draft.value[id];
  delete draftErr.value[id];
  await app.refreshAll();

  // ② 单独问要不要立即生效（切换分支要靠同步，但它很重，由用户决定）
  const affected = clean.length || 1;
  dialog.info({
    title: "已保存",
    content:
      `覆盖表已写入配置。要让它生效，需要把 envs/${id}/ 下 ${affected} 处 worktree ` +
      `切到新分支——这一步会跑一遍全仓库同步，可能要几十秒。\n\n` +
      `现在同步，还是稍后自己点「同步此环境」？`,
    positiveText: "现在同步",
    negativeText: "稍后",
    onPositiveClick: async () => {
      // 只同步**受影响的仓库**：覆盖表里写到的那些 + 该环境实际用到的。
      // 原来是全量同步，为了改两条规则跑 22 个仓库没必要（实测反馈 #18）
      const affected = clean.map(([r]) => r).filter((r) => r !== "*");
      openSync(id, affected.length && !clean.some(([r]) => r === "*") ? affected : undefined);
    },
  });
}

function openAddEnv() {
  showAddEnv.value = true;
  newEnvId.value = "";
  newEnvLabel.value = "";
  newEnvOverrides.value = [];
}

async function doAddEnv() {
  const id = newEnvId.value.trim();
  if (!id) return;
  const overrides = newEnvOverrides.value.filter(([r, b]) => r.trim() && b.trim());
  const verr = validateOverrides(overrides);
  if (verr) {
    msg.error(verr);
    return;
  }
  try {
    // `env_add` 自己会把各仓库的 worktree 补齐（只补建、不 reset），
    // 所以这里**不再调 `syncAll`**：那会把所有环境的 worktree 都 reset 到
    // origin，把还没推送的合并成果一并抹掉 —— 建个环境不该有这种副作用。
    const out = await withBusy(`创建环境 ${id}`, () =>
      api.envAdd({ id, label: newEnvLabel.value.trim() || null, overrides }),
    );
    msg.success(`环境 ${id} 已创建`);
    if (out.env_created.length) {
      msg.success(`已在 ${id} 里检出 ${out.env_created.length} 个仓库`);
    }
    for (const [what, why] of out.env_failures) {
      msg.warning(`环境补齐失败 ${what}：${why}`);
    }
    showAddEnv.value = false;
    await app.refreshAll();
    await loadEnvDirs();
  } catch (e) {
    msg.error(api.errText(e));
  }
}

function addEnvOverride() {
  newEnvOverrides.value = [...newEnvOverrides.value, ["", ""]];
}
function setEnvOverride(idx: number, which: 0 | 1, v: string) {
  const list = newEnvOverrides.value.map((p) => [...p] as [string, string]);
  if (!list[idx]) list[idx] = ["", ""];
  list[idx][which] = v;
  newEnvOverrides.value = list;
}
function delEnvOverride(idx: number) {
  newEnvOverrides.value = newEnvOverrides.value.filter((_, i) => i !== idx);
}

function rmEnv(id: string) {
  dialog.warning({
    title: `移除环境 ${id}`,
    content: "会删掉这个环境里检出的全部代码。有未推送的合并成果会一并丢失。",
    positiveText: "移除",
    negativeText: "取消",
    onPositiveClick: async () => {
      try {
        // 走 busy：要删几十个目录，没有反馈会让人以为没点上（反馈 #9）
        await withBusy(`移除环境 ${id}`, () => api.envRemove([id]));
        msg.success(`${id} 已移除`);
        await app.refreshAll();
      } catch (e) {
        msg.error(api.errText(e));
      }
    },
  });
}

/** 文档库的 GitLab 页面地址。它同样是 git 管理的，一样要有快速访问。
 *  优先用它自己的 remote（现在它就在 repos/ 里，跟普通仓库一个来源）。 */
const gitlabUrlOfDoc = computed(() =>
  (docRow.value && gitlabUrlOf(docRow.value.remote_url)) ||
  gitlabUrlOf(`https://gitlab.example.com/demo-pub/${docName}.git`),
);

/** 仓库的 remote 地址（用来推 GitLab 页面地址）。 */
function remoteOf(name: string): string {
  return app.repos.find((r) => r.name === name)?.remote_url ?? "";
}
function remoteOfDoc(): string {
  return remoteOf(docName.value);
}

function gitlabUrlOf(remote: string): string | null {
  const m = remote.match(/^(?:https?:\/\/|git@)([^/:]+)[/:](.+?)(?:\.git)?$/);
  if (!m) return null;
  return `https://${m[1]}/${m[2]}`;
}
</script>

<template>
  <div class="page">
    <div class="head">
      <!-- 标题跟着页面走。拆成两个菜单项之后还写「仓库与环境」，
           点「环境」进来却顶着「仓库与环境」，等于没拆干净（实测反馈）。 -->
      <h2 class="title">{{ only === "envs" ? "环境" : only === "repos" ? "仓库" : "仓库与环境" }}</h2>
    </div>

    <NTabs
      v-model:value="tab"
      type="line"
      :animated="!only"
      :class="{ standalone: !!only }"
    >
      <!-- ── 仓库 ── -->
      <NTabPane name="repos" tab="仓库">
        <div class="toolbar">
          <NInput
            v-model:value="repoQuery"
            :input-props="NO_AUTO"
            size="small"
            clearable
            placeholder="筛选仓库名 / 地址…"
            class="search"
          />
          <div class="spacer" />
          <!-- 一个都还没检出时按钮要禁用：`repos/` 是同步时才建的 -->
          <NButton size="small" :disabled="!reposRoot" @click="openInFinder(reposRoot)">
            打开目录
          </NButton>
          <NButton size="small" :disabled="!reposRoot" @click="copy(reposRoot, '目录')">
            复制目录
          </NButton>
          <EditorButton :path="reposRoot" :disabled="!reposRoot" :secondary="false" />
          <NButton size="small" @click="openSync(null)">同步主干</NButton>
          <NButton size="small" type="primary" @click="openAdd">添加仓库</NButton>
        </div>

        <!-- 有普通仓库**或**有文档库就渲染这张表。
             原来只看 `repos.length`（那份**不含**文档库），于是「只挂了文档库、
             还没加代码仓库」的 hub 整张表都不渲染 —— 连置顶的文档库行也看不到
             （实测反馈：「仓库里还是没有 x-doc-hub」）。 -->
        <table v-if="repos.length || docRow" class="tbl">
          <thead>
            <tr>
              <th>仓库</th>
              <th>主干</th>
              <th class="acts">操作</th>
            </tr>
          </thead>
          <tbody>
            <!-- 文档库置顶。它跟其他仓库一样是 git 仓库（doc-hub 的 clone），
                 同样有主干分支——主干列不能空着写「—」，`HubInfo.dochub_branch`
                 一直有这个值，是这里没用它（实测反馈 #9）。 -->
            <tr v-if="docRow && !repoQuery.trim()" class="pinned">
              <td>
                <span class="mono">{{ docName }}</span>
                <NTag size="tiny" :bordered="false" type="info" class="tag">文档库</NTag>
                <NTag
                  size="tiny"
                  :bordered="false"
                  :type="app.hub?.dochub_ready ? 'success' : 'warning'"
                  class="tag"
                >
                  {{ app.hub?.dochub_ready ? "已拉到本地" : "未拉取" }}
                </NTag>
                <RepoInfoInline
                  :links="infoOf(docName).links"
                  :notes="infoOf(docName).notes"
                />
              </td>
              <td class="mono">
                <template v-if="docRow">{{ docRow.main_branch }}</template>
                <template v-else-if="app.hub?.dochub_branch">{{ app.hub.dochub_branch }}</template>
                <span v-else class="muted small">未拉取</span>
              </td>
              <td class="acts">
                <div class="btns">
                  <NButton
                    size="tiny"
                    secondary
                    :disabled="!app.hub?.dochub_ready"
                    @click="openInFinder(`${app.hub!.root}/repos/${docName}`)"
                  >
                    打开目录
                  </NButton>
                  <EditorButton
                    size="tiny"
                    :path="`${app.hub!.root}/repos/${docName}`"
                    :disabled="!app.hub?.dochub_ready"
                  />
                  <NButton
                    size="tiny"
                    secondary
                    :disabled="!app.hub?.dochub_ready"
                    @click="copy(`${app.hub!.root}/repos/${docName}`, '目录')"
                  >
                    复制目录
                  </NButton>
                  <NButton
                    size="tiny"
                    secondary
                    type="primary"
                    :disabled="!gitlabUrlOfDoc"
                    @click="openUrl(gitlabUrlOfDoc!)"
                  >
                    GitLab
                  </NButton>
                  <NButton
                    size="tiny"
                    secondary
                    @click="browse = { dir: `${app.hub!.root}/repos/${docName}`, repo: docName, label: docName, mode: 'tree' }"
                  >
                    浏览文件
                  </NButton>
                  <NButton
                    size="tiny"
                    secondary
                    @click="browse = { dir: `${app.hub!.root}/repos/${docName}`, repo: docName, label: docName, mode: 'changed' }"
                  >
                    对比代码
                  </NButton>
                  <NButton size="tiny" secondary @click="openInfo(docName)">自定义信息</NButton>
                  <NButton size="tiny" secondary type="primary" @click="pullDochub">
                    {{ app.hub?.dochub_ready ? "拉取最新" : "拉取文档库" }}
                  </NButton>
                </div>
              </td>
            </tr>

            <tr v-for="r in repos" :key="r.name">
              <td class="mono">
                <div>{{ r.name }}</div>
                <RepoInfoInline :links="infoOf(r.name).links" :notes="infoOf(r.name).notes" />
              </td>
              <td class="mono">{{ r.main_branch }}</td>
              <td class="acts">
                <div class="btns">
                  <NButton size="tiny" secondary @click="openInFinder(`${app.hub!.root}/repos/${r.name}`)">
                    打开目录
                  </NButton>
                  <EditorButton size="tiny" :path="`${app.hub!.root}/repos/${r.name}`" />
                  <NButton size="tiny" secondary @click="copy(`${app.hub!.root}/repos/${r.name}`, '目录')">
                    复制目录
                  </NButton>
                  <!-- 「复制仓库地址」已去掉：旁边就是 GitLab 按钮，点了直接开仓库页，
                       复制一段 URL 再自己粘到浏览器是绕远路（实测反馈）。 -->
                  <NButton
                    v-if="gitlabUrlOf(r.remote_url)"
                    size="tiny"
                    secondary
                    type="primary"
                    @click="openUrl(gitlabUrlOf(r.remote_url)!)"
                  >
                    GitLab
                  </NButton>
                  <NButton
                    size="tiny"
                    secondary
                    @click="browse = { dir: `${app.hub!.root}/repos/${r.name}`, repo: r.name, label: r.name, mode: 'tree' }"
                  >
                    浏览文件
                  </NButton>
                  <NButton
                    size="tiny"
                    secondary
                    @click="browse = { dir: `${app.hub!.root}/repos/${r.name}`, repo: r.name, label: r.name, mode: 'changed' }"
                  >
                    对比代码
                  </NButton>
                  <NButton size="tiny" secondary @click="openInfo(r.name)">自定义信息</NButton>
                  <!-- 「详情」：各环境的**访问地址**只在这里能编辑（仓库 × 环境那张表），
                       另外还能看这个仓库被哪些需求/环境用着。实测反馈：这一格原来没有
                       任何入口 —— 组件在、`detailRepo` 从没被赋过值，等于整个详情弹窗
                       不可达，环境地址只能手改 deployments.json。 -->
                  <NButton size="tiny" secondary @click="detailRepo = r">详情</NButton>
                  <!-- **导出**入口放行上，别只藏在「仓库详情」里 ——
                       实测反馈：详情弹窗里那个按钮人根本找不到（会直接在这一排里找）。
                       把配置导出去是仓库页的常用动作，值得占一个位置。 -->
                  <NButton size="tiny" secondary @click="copyRepoShare(r)">导出配置</NButton>
                  <NButton size="tiny" secondary type="error" @click="rmRepo(r.name, r.usages, r.env_usages)">
                    移除
                  </NButton>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
        <p v-else class="muted">
          {{ repoQuery ? "没有匹配的仓库" : "还没有仓库。点「添加仓库」从 GitLab 检索，或手工粘贴地址。" }}
        </p>
      </NTabPane>

      <!-- ── 环境 ── -->
      <NTabPane name="envs" tab="环境">
        <div class="toolbar">
          <!-- **单选 radio**：环境是切换着看的，不是并排比较的。
               名字在前、标识在后 —— 人认的是「测试环境1」 -->
          <NRadioGroup v-model:value="envPick" size="small">
            <NRadioButton v-for="e in app.envs" :key="e.id" :value="e.id">
              {{ e.label && e.label !== e.id ? `${e.label}（${e.id}）` : e.id }}
            </NRadioButton>
          </NRadioGroup>
          <div class="spacer" />
          <NButton size="small" type="primary" @click="openAddEnv">添加环境</NButton>
        </div>

        <NAlert type="info" :bordered="false" class="mb">
          环境标识与分支名解耦：目录名是环境标识，实际检出哪个分支可以按仓库单独覆盖。
        </NAlert>

        <!-- 一个都还没检出：说清目录是同步时才建的，别让人对着「打开目录」点半天 -->
        <NAlert
          v-for="e in emptyEnvs"
          :key="`empty-${e.id}`"
          type="warning"
          :bordered="false"
          class="mb"
        >
          这个环境还没有检出任何仓库。
          <span class="small">
            下面的表是「该检出什么」的配置，目录要等「同步此环境」才会建出来 ——
            在那之前「打开目录 / 浏览文件」都无内容可开。
          </span>
        </NAlert>

        <template v-for="e in envs" :key="e.id">
          <!-- 选中环境的操作：跟「仓库」那一栏同一套按钮（不再套一层卡片） -->
          <div v-if="!draft[e.id]" class="toolbar">
              <NButton size="small" type="primary" @click="openSync(e.id)">
                同步此环境
              </NButton>
              <NButton size="small" @click="startEditEnv(e)">改分支覆盖</NButton>
              <NButton size="small" @click="startEditEnvMeta(e)">改环境</NButton>
              <NButton size="small" @click="openInFinder(`${app.hub!.root}/envs/${e.id}`)">
                打开目录
              </NButton>
              <EditorButton :path="`${app.hub!.root}/envs/${e.id}`" :secondary="false" />
              <NButton size="small" @click="copy(`${app.hub!.root}/envs/${e.id}`, '目录')">
                复制目录
              </NButton>
              <NButton size="small" type="error" @click="rmEnv(e.id)">移除</NButton>
          </div>

          <!-- 查看态：**跟「仓库」那一栏同一张表的样子**（不再套卡片、也不用紧凑样式） -->
          <table v-if="!draft[e.id]" class="tbl">
            <thead>
              <tr><th>仓库</th><th>实际检出的分支</th><th class="acts">操作</th></tr>
            </thead>
            <tbody>
              <!-- 文档库置顶 + 特殊标识：它在环境里也是 doc-hub，
                   跟仓库页一样的待遇（实测反馈 #5） -->
              <tr v-if="e.effective.some(([m2]) => m2 === docName)" class="pinned">
                <td>
                  <span class="mono">{{ docName }}</span>
                  <NTag size="tiny" :bordered="false" type="info" class="tag">文档</NTag>
                  <RepoInfoInline
                    :links="infoOf(docName).links"
                    :notes="infoOf(docName).notes"
                  />
                  <NTag
                    v-if="!isCheckedOut(e.id, docName)"
                    size="tiny"
                    :bordered="false"
                    class="tag"
                  >
                    未检出
                  </NTag>
                </td>
                <td class="mono">{{ e.effective.find(([m2]) => m2 === docName)![1] }}</td>
                <td class="acts">
                  <div class="btns">
                    <NButton
                      size="tiny"
                      secondary
                      :disabled="!isCheckedOut(e.id, docName)"
                      @click="openInFinder(`${app.hub!.root}/envs/${e.id}/${docName}`)"
                    >
                      打开目录
                    </NButton>
                    <EditorButton
                      size="tiny"
                      :path="`${app.hub!.root}/envs/${e.id}/${docName}`"
                      :disabled="!isCheckedOut(e.id, docName)"
                    />
                    <NButton
                      size="tiny"
                      secondary
                      :disabled="!isCheckedOut(e.id, docName)"
                      @click="copy(`${app.hub!.root}/envs/${e.id}/${docName}`, '目录')"
                    >
                      复制目录
                    </NButton>
                    <NButton
                      size="tiny"
                      secondary
                      :disabled="!isCheckedOut(e.id, docName)"
                      @click="browse = { dir: `${app.hub!.root}/envs/${e.id}/${docName}`, repo: docName, label: docName, mode: 'tree' }"
                    >
                      浏览文件
                    </NButton>
                  <NButton
                    size="tiny"
                    secondary
                    :disabled="!isCheckedOut(e.id, docName)"
                    @click="browse = { dir: `${app.hub!.root}/envs/${e.id}/${docName}`, repo: docName, label: docName, mode: 'changed' }"
                  >
                    对比代码
                  </NButton>
                    <NButton
                      v-if="gitlabUrlOf(remoteOfDoc())"
                      size="tiny"
                      secondary
                      type="primary"
                      @click="openUrl(gitlabBranchUrl(remoteOfDoc(), e.effective.find(([m2]) => m2 === docName)?.[1] ?? '')!)"
                    >
                      GitLab
                    </NButton>
                  </div>
                </td>
              </tr>
              <tr v-for="[mod, br] in e.effective.filter(([m2]) => m2 !== docName)" :key="mod">
                <td class="mono">
                  <div>{{ mod }}</div>
                  <!-- 自定义信息（**只显示、不编辑** —— 编辑统一在「仓库」页）。
                       在仓库上配的 Jenkins 地址，合完代码马上就要去发布；
                       信息就在这里就能点着走（实测反馈）。 -->
                  <RepoInfoInline :links="infoOf(mod).links" :notes="infoOf(mod).notes" />
                </td>
                <td class="mono">
                  {{ br }}
                  <NTag
                    v-if="e.overrides.some(([r2]) => r2 === mod || r2 === '*')"
                    size="tiny"
                    :bordered="false"
                    type="info"
                    class="tag"
                  >
                    覆盖
                  </NTag>
                  <NTag v-if="!isCheckedOut(e.id, mod)" size="tiny" :bordered="false" class="tag">
                    未检出
                  </NTag>
                </td>
                <!-- 操作与「仓库」那一行**对齐**（原来只有浏览文件，不一致） -->
                <td class="acts">
                  <div class="btns">
                    <NButton
                      size="tiny"
                      secondary
                      :disabled="!isCheckedOut(e.id, mod)"
                      @click="openInFinder(`${app.hub!.root}/envs/${e.id}/${mod}`)"
                    >
                      打开目录
                    </NButton>
                    <EditorButton
                      size="tiny"
                      :path="`${app.hub!.root}/envs/${e.id}/${mod}`"
                      :disabled="!isCheckedOut(e.id, mod)"
                    />
                    <NButton
                      size="tiny"
                      secondary
                      :disabled="!isCheckedOut(e.id, mod)"
                      @click="copy(`${app.hub!.root}/envs/${e.id}/${mod}`, '目录')"
                    >
                      复制目录
                    </NButton>
                    <NButton
                      size="tiny"
                      secondary
                      :disabled="!isCheckedOut(e.id, mod)"
                      @click="browse = { dir: `${app.hub!.root}/envs/${e.id}/${mod}`, repo: mod, label: mod, mode: 'tree' }"
                    >
                      浏览文件
                    </NButton>
                  <NButton
                    size="tiny"
                    secondary
                    :disabled="!isCheckedOut(e.id, mod)"
                    @click="browse = { dir: `${app.hub!.root}/envs/${e.id}/${mod}`, repo: mod, label: mod, mode: 'changed' }"
                  >
                    对比代码
                  </NButton>
                    <!-- 点开**直接落在该环境实际检出的那个分支上**（不是仓库首页），
                         省掉在网页里再切一次分支 -->
                    <NButton
                      v-if="gitlabUrlOf(remoteOf(mod))"
                      size="tiny"
                      secondary
                      type="primary"
                      @click="openUrl(gitlabBranchUrl(remoteOf(mod), br)!)"
                    >
                      GitLab
                    </NButton>
                  </div>
                </td>
              </tr>
              <tr v-if="!e.effective.length">
                <td colspan="3" class="muted">还没有仓库</td>
              </tr>
            </tbody>
          </table>

          <!-- 编辑态 -->
          <div v-else class="edit">
            <table class="tbl compact">
              <thead>
                <tr><th>仓库</th><th>分支名</th><th class="acts"></th></tr>
              </thead>
              <tbody>
                <tr v-for="([repo, br], i) in draft[e.id]" :key="i">
                  <td>
                    <NSelect
                      :value="repo || null"
                      size="small"
                      filterable
                      tag
                      placeholder="选一个仓库"
                      :options="overrideRepoOptions(app.repos, repo)"
                      @update:value="(v: string) => setOverride(e.id, i, 0, v ?? '')"
                    />
                  </td>
                  <td>
                    <NInput
                      :value="br"
                      :input-props="NO_AUTO"
                      size="small"
                      placeholder="分支名，如 pre-202609"
                      @update:value="(v: string) => setOverride(e.id, i, 1, v)"
                    />
                  </td>
                  <td class="acts">
                    <NButton size="tiny" secondary @click="delOverride(e.id, i)">删</NButton>
                  </td>
                </tr>
                <tr v-if="!draft[e.id].length">
                  <td colspan="3" class="muted">没有覆盖规则 —— 所有仓库都用同名分支 {{ e.id }}</td>
                </tr>
              </tbody>
            </table>

            <p v-if="draftErr[e.id]" class="err">{{ draftErr[e.id] }}</p>

            <div class="edit-actions">
              <NButton size="tiny" @click="addOverride(e.id)">加一条</NButton>
              <div class="spacer" />
              <NButton size="tiny" @click="delete draft[e.id]">取消</NButton>
              <NButton size="tiny" type="primary" :disabled="!!draftErr[e.id]" @click="saveEnv(e.id)">
                保存
              </NButton>
            </div>
          </div>
        </template>
        <p v-if="!envs.length" class="muted">还没有环境</p>
      </NTabPane>
    </NTabs>

    <!-- 浏览文件（仓库页与环境页共用同一个组件） -->
    <NModal
      v-if="browse"
      :show="true"
      @update:show="(v: boolean) => !v && (browse = null)"
      preset="card"
            :title="browse.mode === 'tree' ? `浏览文件 · ${browse.label}` : `对比代码 · ${browse.label}`"
      style="width: 92vw; height: 88vh"
      @close="browse = null"
    >
      <FileBrowser
        :dir="browse.dir"
        :repo="browse.repo"
        :label="browse.label"
        :default-mode="browse.mode"
      />
    </NModal>

    <!-- 改环境标识与显示名 -->
    <NModal
      v-if="showEnvMeta"
      :show="true"
      @update:show="(v: boolean) => !v && (showEnvMeta = false)"
      preset="card"
      title="修改环境"
      style="width: 560px"
      @close="showEnvMeta = false"
    >
      <div class="form">
        <label>环境标识</label>
        <!-- **标识不允许改**：它同时是目录名 `envs/<标识>/`，改了所有仓库都得
             重新切分支、重新同步一遍代码，代价远大于收益。要换标识就删掉重建
             （那是显式操作，不会悄悄发生）。 -->
        <code class="mono">{{ envMetaOld }}</code>
        <label>显示名</label>
        <NInput
          v-model:value="envMetaLabel"
          placeholder="如 测试环境1（留空则显示标识）"
          :input-props="NO_AUTO"
        />
      </div>
      <template #footer>
        <div class="footer">
          <NButton @click="showEnvMeta = false">取消</NButton>
          <NButton type="primary" @click="saveEnvMeta">保存</NButton>
        </div>
      </template>
    </NModal>

    <SyncDialog
      v-if="syncTarget"
      :title="syncTitle"
      :hint="syncHintText"
      :names="syncNames"
      :hints="syncHints"
      :action="runSync"
      :preselect="syncPreselect"
      @close="syncTarget = null; syncPreselect = null"
    />

    <RepoDetail
      v-if="detailRepo"
      :repo="detailRepo"
      @close="detailRepo = null"
      @goto-ws="(b: string) => emit('gotoWs', b)"
    />

    <!-- 添加仓库 -->
    <NModal v-model:show="showAdd" preset="card" title="添加仓库" style="width: 720px" @close="showAdd = false">
      <div class="modes">
        <NCheckbox :checked="addMode === 'gitlab'" @update:checked="() => (addMode = 'gitlab')">
          从 GitLab 检索
        </NCheckbox>
        <NCheckbox :checked="addMode === 'manual'" @update:checked="() => (addMode = 'manual')">
          手工填地址
        </NCheckbox>
        <NCheckbox :checked="addMode === 'paste'" @update:checked="() => (addMode = 'paste')">
          粘贴配置文本
        </NCheckbox>
      </div>

      <template v-if="addMode === 'gitlab'">
        <div class="gl-bar">
          <NInput
            v-model:value="glQuery"
            :input-props="NO_AUTO"
            size="small"
            placeholder="搜索仓库名（留空列出最近活跃的）"
            @keyup.enter="searchGitlab"
          />
          <NButton size="small" :loading="glLoading" @click="searchGitlab">检索</NButton>
          <span class="muted small">已选 {{ glPicked.length }}</span>
        </div>
        <NAlert v-if="glErr" type="warning" :bordered="false" class="mb">
          {{ glErr }}
          <div class="small">到「设置」页确认 GitLab token 是否配置正确。</div>
        </NAlert>
        <div class="gl-list">
          <NCheckbox
            v-for="p in glResults"
            :key="p.path_with_namespace"
            :checked="glPicked.includes(p.path_with_namespace)"
            @update:checked="
              (c: boolean) =>
                (glPicked = c
                  ? [...glPicked, p.path_with_namespace]
                  : glPicked.filter((x) => x !== p.path_with_namespace))
            "
          >
            <span class="mono">{{ p.path_with_namespace }}</span>
            <span class="muted small"> 主干 {{ p.default_branch ?? "?" }}</span>
          </NCheckbox>
          <p v-if="!glResults.length && !glLoading" class="muted small">
            点「检索」列出你有权限的仓库
          </p>
        </div>
      </template>

      <template v-else-if="addMode === 'manual'">
        <NInput
          v-model:value="manualUrls"
          :input-props="NO_AUTO"
          type="textarea"
          :rows="5"
          placeholder="每行一个 git 地址，或用空格/逗号分隔"
        />
      </template>

      <template v-else>
        <NInput
          v-model:value="shareText"
          :input-props="NO_AUTO"
          type="textarea"
          :rows="5"
          placeholder="把对方发来的那段「gws 仓库配置」整段粘到这里"
        />
        <div class="share-actions">
          <NButton size="small" :loading="sharing" :disabled="!shareText.trim()" @click="recognizeRepo">
            识别配置
          </NButton>
          <span class="muted small">识别后下面会列出要导入什么，确认后再点「添加」</span>
        </div>
        <NAlert v-if="shareErr" type="error" :bordered="false" class="mt">{{ shareErr }}</NAlert>
        <!-- 已识别：要把「会做什么」说清楚（克隆要联网，本地的配置不会被覆盖） -->
        <div v-if="sharedRepo" class="share-preview">
          <div class="kv"><span class="k">仓库名</span><span class="mono">{{ sharedRepo.name }}</span></div>
          <div class="kv"><span class="k">clone 地址</span><span class="mono small">{{ sharedRepo.url || "（没带地址，本地没有就补不了）" }}</span></div>
          <div v-if="sharedRepo.gitlab" class="kv">
            <span class="k">GitLab</span><span class="mono small">{{ sharedRepo.gitlab }}</span>
          </div>
          <div v-if="sharedRepo.notes" class="kv"><span class="k">备注</span><span>{{ sharedRepo.notes }}</span></div>
          <div v-if="sharedRepo.links.length" class="kv">
            <span class="k">自定义链接</span>
            <span class="small">{{ sharedRepo.links.map((l: Link) => l.label).join("、") }}</span>
          </div>
          <div v-if="Object.keys(sharedRepo.envs).length" class="kv">
            <span class="k">环境地址</span>
            <span class="small">{{ Object.keys(sharedRepo.envs).join("、") }}</span>
          </div>
          <p class="muted small">
            本地已经配过的项**不会被覆盖**，只补没有的；缺的仓库会联网克隆（可能要一会儿）。
          </p>
        </div>
      </template>

      <template #footer>
        <div class="footer">
          <NButton @click="showAdd = false">取消</NButton>
          <NButton type="primary" @click="doAdd">添加</NButton>
        </div>
      </template>
    </NModal>

    <!-- 添加环境（弹窗填写，含中文说明）-->
    <NModal v-model:show="showAddEnv" preset="card" title="添加环境" style="width: 660px" @close="showAddEnv = false">
      <NAlert type="info" :bordered="false" class="mb">
        创建后会自动把各仓库检出到这个环境里（大仓库要等一会儿，
        上面有进度）。这一步不会动已经存在的环境内容。
      </NAlert>
      <div class="form">
        <label>环境标识</label>
        <NInput v-model:value="newEnvId" :input-props="NO_AUTO" placeholder="如 pre、dev2、pre_tmp" />
        <label>中文说明</label>
        <NInput
          v-model:value="newEnvLabel"
          :input-props="NO_AUTO"
          placeholder="如 预发环境（留空则显示为标识本身）"
        />
        <label></label>
        <p class="muted small">
          环境标识同时是 <code>envs/&lt;标识&gt;/</code> 的目录名，建完后不要改。
        </p>
      </div>

      <div class="ov-head">
        <span>分支名覆盖</span>
        <span class="muted small">
          不写就用**环境标识本身**当分支名；要换分支就**逐个仓库指定** ——
          每个仓库实际有哪些分支只有它自己知道，一条通配盖上去很容易让
          大多数仓库"没这个分支"而被静默跳过。
        </span>
      </div>
      <table class="tbl compact">
        <thead>
          <tr><th>仓库</th><th>分支名</th><th class="acts"></th></tr>
        </thead>
        <tbody>
          <tr v-for="([repo, br], i) in newEnvOverrides" :key="i">
            <td>
              <NSelect
                :value="repo || null"
                size="small"
                filterable
                tag
                placeholder="选一个仓库"
                :options="overrideRepoOptions(app.repos, repo)"
                @update:value="(v: string) => setEnvOverride(i, 0, v ?? '')"
              />
            </td>
            <td>
              <NInput
                :value="br"
                :input-props="NO_AUTO"
                size="small"
                placeholder="分支名"
                @update:value="(v: string) => setEnvOverride(i, 1, v)"
              />
            </td>
            <td class="acts">
              <NButton size="tiny" secondary @click="delEnvOverride(i)">删</NButton>
            </td>
          </tr>
          <tr v-if="!newEnvOverrides.length">
            <td colspan="3" class="muted">（无覆盖，全部同名）</td>
          </tr>
        </tbody>
      </table>
      <NButton size="tiny" class="mt" @click="addEnvOverride">加一条覆盖</NButton>

      <template #footer>
        <div class="footer">
          <NButton @click="showAddEnv = false">取消</NButton>
          <NButton type="primary" :disabled="!newEnvId.trim()" @click="doAddEnv">创建</NButton>
        </div>
      </template>
    </NModal>

    <!-- 仓库的自定义信息：链接（点了打开）+ 纯文本（点了复制） -->
    <NModal
      v-model:show="showInfo"
      preset="card"
      :title="`自定义信息 · ${infoRepo}`"
      style="width: 660px"
      @close="showInfo = false"
    >
      <div class="form">
        <label>备注</label>
        <NInput
          v-model:value="editNotes"
          type="textarea"
          size="small"
          :rows="3"
          :input-props="NO_AUTO"
          placeholder="纯文本。会显示在仓库列表里，旁边有「复制」"
        />
        <label>链接 / 文本</label>
        <LinkEditor
          v-model="editLinks"
          :placeholders="['如 dev Jenkins / 模块域名', 'https://…（留空 = 纯文本，点了复制而不是打开）']"
        />
      </div>
      <p class="muted small note">
        有地址的条目在列表里点一下就用浏览器打开；<strong>地址留空</strong>的条目点一下是
        <strong>复制</strong> —— 域名这类东西要的是拷走，不是打开。
      </p>
      <template #footer>
        <div class="footer">
          <NButton @click="showInfo = false">取消</NButton>
          <NButton type="primary" :loading="savingInfo" @click="saveRepoInfo">保存</NButton>
        </div>
      </template>
    </NModal>
  </div>
</template>

<style scoped>
/* 由菜单进来时（only 有值）藏掉 tab 条：它就是那个页面本身，
   再顶着一排「仓库 / 环境」的标签会让人以为还能切。 */
:deep(.standalone .n-tabs-nav) {
  display: none;
}
.page {
  padding: 20px 24px 40px;
}
.head {
  margin-bottom: 12px;
}
.title {
  margin: 0;
  font-size: 18px;
  font-weight: 600;
}
.toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
  flex-wrap: wrap;
}
.search {
  width: 200px;
}
.spacer {
  flex: 1;
}
.tbl {
  width: 100%;
  border-collapse: separate;
  border-spacing: 0;
  font-size: 13px;
  border: 1px solid var(--xg-border);
  border-radius: 6px;
  overflow: hidden;
}
.tbl th,
.tbl td {
  text-align: left;
  padding: 6px 10px;
  border-bottom: 1px solid var(--xg-border);
  vertical-align: middle;
}
.tbl tbody tr:last-child td {
  border-bottom: none;
}
.tbl th {
  color: #888;
  font-weight: 500;
  font-size: 12px;
  background: rgba(128, 128, 128, 0.06);
}
.compact th,
.compact td {
  padding: 4px 8px;
}
/* 操作列**按内容自然撑开**。
   原来写了 width: 1%（想让它尽量窄），结果列被压到最小宽度、按钮被迫换行，
   行高翻倍而左侧空一片——反馈里说的「每个项目很高但左边又很空」就是这个。
   仓库名没那么长，宽度够用；真不够时浏览器会把列稍微挤一挤，不会换行。 */
.acts {
  white-space: nowrap;
}
/* 仓库名与主干分支名**硬保证不换行**。
   之前只靠「列宽够用」这个默认行为，项目名一长就把行撑成两行，
   而那一列本来空着——反馈里说的「每个项目很高但左边又很空」。 */
.tbl td.mono {
  white-space: nowrap;
}
/* 操作按钮成组，看起来是按钮而不是散落的文本（反馈 #6）。
   **不换行**：换行会让行高翻倍，而这一列本来就放得下。 */
.btns {
  display: inline-flex;
  gap: 6px;
  align-items: center;
}
.tag {
  margin-right: 4px;
}
.muted {
  color: #888;
  font-size: 12px;
}
.small {
  font-size: 12px;
}
.mb {
  margin-bottom: 10px;
}
.mt {
  margin-top: 8px;
}
/* 改环境弹窗的表单 */
.form {
  display: grid;
  grid-template-columns: 6em 1fr;
  gap: 10px 12px;
  align-items: center;
  font-size: 13px;
}
.footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
/* 置顶行给个淡底色，让人一眼看出这是"文档库"而不是普通仓库 */
.pinned td {
  background: rgba(37, 99, 235, 0.05);
}
.env-card {
  margin-bottom: 10px;
}
.env-id {
  font-weight: 600;
}
.edit-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 8px;
}
.share-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 8px;
}
.share-preview {
  margin-top: 10px;
  padding: 10px 12px;
  border-radius: 6px;
  background: rgba(128, 128, 128, 0.06);
}
.modes {
  display: flex;
  gap: 16px;
  margin-bottom: 12px;
}
.gl-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}
.gl-list {
  max-height: 320px;
  overflow: auto;
  border: 1px solid var(--xg-border);
  border-radius: 6px;
  padding: 8px 12px;
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.form {
  display: grid;
  grid-template-columns: 6em 1fr;
  gap: 10px 12px;
  align-items: center;
  font-size: 13px;
}
.ov-head {
  display: flex;
  align-items: baseline;
  gap: 10px;
  margin: 16px 0 6px;
  font-size: 13px;
}
.err {
  color: #d03050;
  font-size: 12px;
  margin: 8px 0 0;
}
.footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>