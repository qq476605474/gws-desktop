<script setup lang="ts">
//! 模块文件浏览与对比（需求 #18）。
//!
//! 原来只能看到「这个模块有 3 处改动」，点不进去看具体改了什么。
//! 这里给出：
//! - 左侧：相对主干的**改动文件列表**，或整个目录树
//! - 右侧：文件内容 / 与主干的 diff
//!
//! 用途：review 自己或其他人改了什么、定位某段逻辑在哪个文件里。

import { computed, onMounted, ref, watch } from "vue";
import { NAlert, NButton, NEmpty, NInput, NSelect, NSpin } from "naive-ui";

import * as api from "../bridge";
import { useCopy } from "../lib/copy";
import { useAppStore } from "../stores/app";
import {
  compareBase,
  isReady,
  type BaseChoice,
  type CompareTarget,
} from "../lib/compareBase";
import { NO_AUTO } from "../lib/inputs";
import type {
  BaseResolution,
  CommitInfo,
  EntryInfo,
  FileChange,
  FileDiff,
} from "../types";

const props = defineProps<{
  /** 要浏览的工作目录（绝对路径）。**单模块模式**下必给。 */
  dir?: string;
  /** 所属仓库名，**只用来探测主干名**（各仓库 main/master 混用）。文档库传 doc-hub。 */
  repo?: string;
  /** 该目录检出的分支（用于「与远程比」）。 */
  branch?: string;
  /** 标题里显示的短名（模块名 / 仓库名 / 环境）。 */
  label?: string;
  /**
   * **整个需求**模式：给了它就按模块分组，把各模块的改动全列在一屏里。
   *
   * 实测反馈：原来「需求改动总览」点一个模块会**再开一个窗口**看那个模块的
   * 文件 —— 两层跳转。像 IDE 那样「左树分组、点文件右侧出 diff」才是要的。
   */
  wsBranch?: string;
  /**
   * 比对基准 ref，如 `origin/dev`。
   *
   * 不给就探测主干 —— 给了才能回答「本地 vs 远程环境分支」这类问题
   * （合并检查关心的正是这个，而不是跟主干比）。
   */
  base?: string | null;
  /** 打开时落在哪一页。缺省：整需求模式看「差异」，其余看「文件树」。 */
  defaultMode?: "changed" | "tree" | "history";
  /** 打开时的对比目标，缺省「与本地」（本地未提交的改动）。 */
  defaultTarget?: "worktree" | "remote" | "main" | "env" | "branch";
}>();
const copy = useCopy();
const app = useAppStore();
/** 文档库在 hub 里的目录名（从配置读，别写死）。 */
const docName = computed(() => app.hub?.dochub_name ?? "doc-hub");


type Mode = "changed" | "tree" | "history";

/** 一个可供浏览的目录（单模块模式下就一个；整个需求模式下每个模块一个）。 */
interface Source {
  /** 分组标题（模块名）。 */
  label: string;
  dir: string;
  repo: string;
  /** 比对基准，null = 探测主干。 */
  base: string | null;
  /** 该目录检出的分支（用于「与远程比」）。 */
  branch: string;
}

/**
 * 对比目标 —— **这才是「看差异」这个功能的主角**。
 *
 * 实测反馈：「浏览文件」和「看 git 差异」是两件事 ——
 * 前者是看真实的当前文件和内容，后者才是看分支之间的差异，
 * 而常用的就那么几种：本地未提交的改动（**默认**）、本地 vs 远程、
 * 本地 vs 环境、本地 vs 生产主干，以及「最近别人提交了什么」（提交历史）。
 *
 * 默认「与本地」而不是「与远程」：需求分支常常还没推送，那时 `origin/<分支>`
 * 不存在，默认「与远程」就只剩一句报错，本地明明改了东西却什么都看不到（实测反馈）。
 * 所以把目标做成显式选择，而不是由上下文猜。
 */
type Target = "worktree" | "remote" | "main" | "env" | "branch" | "local" | "baseline";

/**
 * 是否「整个需求」模式。
 *
 * `wsBranch` 现在有两个用途：①给出需求上下文（「与基线」要用）；
 * ②没给 `dir` 时切成整需求多模块视图。需求详情里打开的是**单个模块**
 * （同时给了 dir），不能因为带了需求分支就变成全模块列表。
 */
const wholeWs = computed(() => !!props.wsBranch && !props.dir);

/** 当前的对比目标（默认「与本地」：未提交的改动，不依赖分支有没有推送）。 */
const target = ref<Target>(props.defaultTarget ?? "worktree");
/** 参与对比的环境标识（选「与环境比」时用）。 */
const envForTarget = ref<string>("");
/** 「与分支」时手输的远程分支名。 */
const branchTarget = ref("");
/**
 * 「与基线」解析结果：`repo → 结果`。
 *
 * 要按模块分开：同一条基线链在不同仓库上命中的分支可能不同（某个仓库没有
 * 上游分支），甚至有的模块会退回主干 —— 一个全局 ref 会张冠李戴。
 */
const baselineRefs = ref<Record<string, BaseResolution>>({});
const baselineLoading = ref(false);
const baselineErr = ref("");
const sources = ref<Source[]>([]);
/** 每个源各自的改动文件：`源下标 → 文件`。 */
const bySource = ref<FileChange[][]>([]);
/**
 * 每个源各自的错误。
 *
 * **按模块分开报**：同一个需求的各仓库分支状态不一样（有的推过、有的没推），
 * 一个模块「远程还没有这个分支」不该让整屏都空 —— 其它模块的改动照样能看
 * （实测反馈）。
 */
const srcErr = ref<string[]>([]);
/**
 * 本地相对当前基准的 `(领先, 落后)`。
 *
 * **必须显示**：`diff` 只说"哪些文件不一样"，不说方向。而"本地落后 7500 个提交"
 * 和"本地领先 3 个提交"是完全不同的两件事 —— 前者该去拉取，后者才是要推的
 * （实测反馈：主干仓库 vs origin/dev 明明差几千个提交，界面却说没变化）。
 */
const aheadBehind = ref<[number, number] | null>(null);
/** 当前选中的文件所属的源。 */
const selSource = ref<Source | null>(null);

// ── 提交历史 ──
const history = ref<CommitInfo[]>([]);
/** 已加载的批数（每批 20 条）。「最近 20 条」不用那么死板，可以继续往下要。 */
const historyBatches = ref(1);
/** 选中的提交（null = 还没选，列的是全部提交）。 */
const selCommit = ref<CommitInfo | null>(null);
/** 选中提交改了哪些文件。 */
const commitFiles = ref<FileChange[]>([]);

/**
 * 默认看哪一页。
 *
 * **「浏览文件」进来就该看到文件**（真实内容），差异是另一个功能、要人主动切
 * （实测反馈：两个功能原来混在一起了）。整需求模式例外 —— 那里本来就是看
 * 「这个需求改了什么」。
 */
const mode = ref<Mode>(props.defaultMode ?? (wholeWs.value ? "changed" : "tree"));
/** 目录树模式下的当前目录（相对模块根）。 */
const cwd = ref("");
const entries = ref<EntryInfo[]>([]);
const loadingList = ref(false);
/** 目录是否**真的读过**一遍 —— 用来区分「空目录」和「还没读」。 */
const treeLoaded = ref(false);
const listErr = ref("");

const picked = ref<string | null>(null);
const content = ref("");
const diff = ref("");
const diffStat = ref("");
/** 最近一次改动这个文件的人 / 日期 / 说明。 */
const fileDiff = ref<FileDiff | null>(null);
const loadingFile = ref(false);
const fileErr = ref("");
const q = ref("");
/** 右侧看哪一页：diff / 内容。 */
const pane = ref<"diff" | "content">("diff");

/** 单模块模式：整个组件只服务一个目录。 */
function singleSource(): Source[] {
  return [
    {
      label: props.label ?? "",
      dir: props.dir ?? "",
      repo: props.repo ?? "",
      base: props.base ?? null,
      branch: props.branch ?? "",
    },
  ];
}

/**
 * 把「对比目标」翻译成真正的 git ref。
 *
 * 显式给了 `base`（比如合并检查传的 `origin/<环境分支>`）就以它为准 ——
 * 那种上下文的意图很明确，不该被这里的下拉覆盖。
 */
/**
 * 把「对比目标」翻译成真正的 git ref（规则在 `lib/compareBase.ts`，那边有单测）。
 *
 * **三种返回值含义不同**：`string` = 具体基准；`undefined` = 交给后端探测主干；
 * `null` = 目标还没选全。
 *
 * ⚠️ 实测踩过：这两种「没有基准」的意思混用过，调用方只看 `null` 判断能不能发请求，
 * 于是「与主干」被当成"没选全"、**请求根本没发出去**，界面上永远是「差异 (0)」。
 */
function baseOf(src: Source): BaseChoice {
  const row = app.envs.find((e) => e.id === envForTarget.value);
  return compareBase({
    target: target.value as CompareTarget,
    source: src,
    branchInput: branchTarget.value,
    env: envForTarget.value,
    envBranch: row?.effective.find(([m]) => m === src.repo)?.[1],
    baselineRef: baselineRefs.value[src.repo]?.reference,
    override: props.base,
  });
}

/**
 * 确保每个模块的基线都解析过（后端要 fetch，所以慢、只能异步）。
 *
 * **必须先解析再去列改动**：`baseOf` 对没解析出的基线返回 `null`（先别请求），
 * 如果这里不先跑，切到「与基线」的第一次加载会直接显示「差异 (0)」。
 */
async function ensureBaselines() {
  if (target.value !== "baseline" || !props.wsBranch) return;
  const missing = sources.value.filter((s) => !baselineRefs.value[s.repo]);
  if (!missing.length) return;
  baselineLoading.value = true;
  baselineErr.value = "";
  try {
    const results = await Promise.all(
      missing.map((src) =>
        api.wsBaseline(props.wsBranch!, src.repo).catch((e) => {
          baselineErr.value = api.errText(e);
          return null;
        }),
      ),
    );
    const next = { ...baselineRefs.value };
    missing.forEach((src, i) => {
      const r = results[i];
      if (r) next[src.repo] = r;
    });
    baselineRefs.value = next;
  } finally {
    baselineLoading.value = false;
  }
}

/** 「与基线」当前用的 ref（多模块可能不同，去重后拼起来给标题用）。 */
const baselineLabel = computed(() => {
  const refs = [
    ...new Set(
      sources.value
        .map((s) => baselineRefs.value[s.repo]?.reference)
        .filter((r): r is string => !!r),
    ),
  ];
  return refs.join(" / ");
});

/**
 * 整个需求模式：从需求详情推出「要列哪些目录」。
 *
 * 文档库（doc-hub）也算一个分组 —— 它本来就是挂在需求分支上的另一块 worktree。
 */
async function wsSources(): Promise<Source[]> {
  // **从 store 拿 hub 根路径**，不搞模块级的影子副本 —— 那种副本在某些启动
  // 路径下（比如直接恢复上次的 hub）没被设上，于是拼出 `/ws/...` 这种缺了前缀
  // 的路径，报错是「目录不在 hub 内，已拒绝」（实测反馈）。
  const root = app.hub?.root ?? "";
  if (!root) {
    throw new Error("还没有打开 hub");
  }
  const b = props.wsBranch ?? "";
  const detail = await api.wsDetail(b);
  const out: Source[] = detail.status.modules
    .filter((m) => m.present)
    .map((m) => ({
      label: m.module,
      dir: `${root}/ws/${b}/${m.module}`,
      repo: m.module,
      base: null,
      branch: props.wsBranch ?? "",
    }));
  if (detail.meta.doc) {
    out.unshift({
      label: docName.value,
      dir: `${root}/ws/${b}/${docName.value}`,
      repo: docName.value,
      base: null,
      branch: props.wsBranch ?? "",
    });
  }
  return out;
}

/**
 * 只算「要浏览哪些目录」—— **不读 git**。
 *
 * 文件树模式只用它拿 `dir`（点文件时要读哪个目录）。原来把它和"读 git"
 * 混在 `loadChanged()` 里，而文件树又不该读 git —— 结果我上次把整个
 * `loadChanged()` 跳过之后，`sources` 是空的，点文件直接早返回，
 * **转圈不停、内容永远不出来**（实测反馈）。
 */
async function loadSources() {
  sources.value = wholeWs.value ? await wsSources() : singleSource();
}

async function loadChanged() {
  loadingList.value = true;
  listErr.value = "";
  try {
    await loadSources();
    // 「与基线」要先让后端解析出每个模块的 ref（会 fetch），再列改动
    await ensureBaselines();
    // 目标没选全（与环境/与分支/基线还在解析）就先不算 —— 免得把"与主干"的
    // 结果挂在别的标题下
    if (sources.value.some((src) => !isReady(baseOf(src)))) {
      bySource.value = sources.value.map(() => []);
      return;
    }
    // 各模块并发拉：串行的话十几个模块要等很久
    const errs: string[] = [];
    bySource.value = await Promise.all(
      sources.value.map((src, i) => {
        // 目标没选全（与环境/与分支）就不请求，但也不算错误
        const b = baseOf(src);
        // `undefined`（与主干）**要发请求**，只是让后端自己探测主干
        if (!isReady(b)) return [];
        return api.browseChanges(src.dir, src.repo, b ?? null).catch((e) => {
          errs[i] = api.errText(e);
          return [] as FileChange[];
        });
      }),
    );
    srcErr.value = sources.value.map((_, i) => errs[i] ?? "");
    // 方向（只看单模块模式：整需求模式下各模块基准不同，逐个显示才不误导）
    aheadBehind.value = null;
    const only = sources.value[0];
    // 「与主干」也要算领先/落后 —— 那正是这个视图最想看的一个数（后端探测主干）
    if (!wholeWs.value && only) {
      const b = baseOf(only);
      if (isReady(b)) {
        aheadBehind.value = await api
          .browseAheadBehind(only.dir, only.repo, b ?? null)
          .catch(() => null);
      }
    }
  } catch (e) {
    listErr.value = api.errText(e);
    sources.value = [];
    bySource.value = [];
  } finally {
    loadingList.value = false;
  }
}

/** 所有分组里的改动总数。 */
const totalChanged = computed(() => bySource.value.reduce((n, g) => n + g.length, 0));

/** 拉最近 20 条提交。 */
async function loadHistory() {
  loadingList.value = true;
  listErr.value = "";
  try {
    const src = sources.value[0];
    if (!src) return;
    history.value = await api.browseCommits(src.dir, 20 * historyBatches.value);
  } catch (e) {
    listErr.value = api.errText(e);
    history.value = [];
  } finally {
    loadingList.value = false;
  }
}

/** 再加 20 条。 */
function moreHistory() {
  historyBatches.value += 1;
  void loadHistory();
}

/** 拉回来的条数正好等于已请求的条数 → 可能还有更多。 */
const maybeMoreHistory = computed(
  () => history.value.length >= 20 * historyBatches.value,
);

/** 点一条提交 → 列它的文件变动。 */
async function pickCommit(c: CommitInfo) {
  selCommit.value = c;
  picked.value = null;
  const src = sources.value[0];
  if (!src) return;
  loadingList.value = true;
  try {
    commitFiles.value = await api.browseCommitFiles(src.dir, c.sha);
  } catch (e) {
    listErr.value = api.errText(e);
    commitFiles.value = [];
  } finally {
    loadingList.value = false;
  }
}

/** 看某个文件在那个提交里的改动（内容取的是**那个提交时的版本**）。 */
async function openCommitFile(path: string) {
  const src = sources.value[0];
  const c = selCommit.value;
  if (!src || !c) return;
  picked.value = path;
  selSource.value = src;
  loadingFile.value = true;
  fileErr.value = "";
  try {
    const d = await api.browseCommitDiff(src.dir, c.sha, path);
    content.value = d.content;
    diff.value = d.diff;
    diffStat.value = d.stat;
    fileDiff.value = null; // 这里显示的是提交信息，不是"谁最后改的"
  } catch (e) {
    fileErr.value = api.errText(e);
    content.value = "";
    diff.value = "";
  } finally {
    loadingFile.value = false;
  }
}

async function loadTree() {
  loadingList.value = true;
  listErr.value = "";
  try {
    const dir = cwd.value ? `${moduleRoot.value}/${cwd.value}` : moduleRoot.value;
    entries.value = await api.listEntries(dir);
    treeLoaded.value = true;
  } catch (e) {
    listErr.value = api.errText(e);
    entries.value = [];
    treeLoaded.value = true;
  } finally {
    loadingList.value = false;
  }
}

/** 浏览根目录（由调用方给出，四种场景各不同）。 */
const moduleRoot = computed(() => treeSource.value?.dir ?? props.dir ?? "");

/** 左侧列表的条目：改动模式下带 git 状态，文件树模式下就是名字。 */
type Row = { key: string; label: string; status?: string; dir?: boolean };

/**
 * 左侧要显示的**分组**。
 *
 * 单模块模式一个分组（就是它自己）；整个需求模式每个模块一个分组，
 * 全部列在同一屏里 —— 不跳窗口（实测反馈）。
 */
const groups = computed(() => {
  const q2 = q.value.trim().toLowerCase();
  return sources.value
    .map((src, i) => ({
      src,
      files: bySource.value[i] ?? [],
    }))
    .map((g) => ({
      ...g,
      files: q2 ? g.files.filter((f) => f.path.toLowerCase().includes(q2)) : g.files,
    }))
    // 整个需求模式下，没改动的模块不占地方；单模块模式保留（要显示"没有改动"）
    .filter((g) => (wholeWs.value ? g.files.length > 0 : true));
});

/** 整需求模式下正在看哪个模块的文件树（null = 还没进，列的是模块）。 */
const treeSource = ref<Source | null>(null);

/** 进入某个模块的文件树。 */
async function openTree(src: Source) {
  treeSource.value = src;
  cwd.value = "";
  treeLoaded.value = false;
  loadingList.value = true;
  listErr.value = "";
  try {
    entries.value = await api.listEntries(src.dir);
  } catch (e) {
    listErr.value = api.errText(e);
    entries.value = [];
  } finally {
    loadingList.value = false;
  }
}

/** 文件树模式下的条目（只有单模块模式有）。 */
const treeRows = computed<Row[]>(() => {
  const s = q.value.trim().toLowerCase();
  return entries.value
    .filter((e) => !s || e.name.toLowerCase().includes(s))
    .map((e) => ({ key: e.name, label: e.name, dir: e.is_dir }));
});

const changed = computed(() => bySource.value[0] ?? []);

/** 状态字母 → 中文与颜色。**review 时「新增」和「删除」性质完全不同**，
 *  只给路径根本看不出改了什么（实测反馈）。 */
function statusOf(st?: string): { text: string; cls: string } {
  switch (st) {
    case "A":
      return { text: "新增", cls: "add" };
    case "M":
      return { text: "修改", cls: "mod" };
    case "D":
      return { text: "删除", cls: "del" };
    case "R":
      return { text: "重命名", cls: "ren" };
    case "C":
      return { text: "复制", cls: "ren" };
    default:
      return { text: st ?? "?", cls: "mod" };
  }
}



const contentLines = computed(() => content.value.split("\n"));

/** 「张三 · 2026-10-12 · 修复登录」——最后一次改动这个文件的人。 */
/** 右侧那页的标题：说清到底在跟谁比。 */
const diffLabel = computed(() => {
  if (props.base) return `与 ${props.base} 的差异`;
  if (target.value === "worktree") return "与本地（未提交的改动）的差异";
  if (target.value === "remote") return "与远程的差异（本地有、远程还没有）";
  if (target.value === "env") {
    return envForTarget.value ? `与环境 ${envForTarget.value} 的差异` : "与环境对比（先选环境）";
  }
  if (target.value === "branch") {
    return branchTarget.value.trim()
      ? `与远程分支 ${branchTarget.value.trim()} 的差异`
      : "与远程分支对比（先填分支名）";
  }
  if (target.value === "local") {
    return branchTarget.value.trim()
      ? `与本地分支 ${branchTarget.value.trim()} 的差异`
      : "与本地分支对比（先填分支名）";
  }
  if (target.value === "baseline") {
    return baselineLabel.value ? `与基线 ${baselineLabel.value} 的差异` : "与基线的差异";
  }
  return "与主干的差异";
});

/** 短名，给「该文件相对 X 没有改动」这类句子用（`diffLabel` 太长）。 */
const baseShort = computed(() => {
  if (props.base) return props.base;
  if (target.value === "worktree") return "本地（HEAD）";
  if (target.value === "remote") return "远程";
  if (target.value === "env") return envForTarget.value ? `环境 ${envForTarget.value}` : "环境";
  if (target.value === "branch") {
    return branchTarget.value.trim() ? `远程分支 ${branchTarget.value.trim()}` : "远程分支";
  }
  if (target.value === "local") {
    return branchTarget.value.trim() ? `本地分支 ${branchTarget.value.trim()}` : "本地分支";
  }
  if (target.value === "baseline") return baselineLabel.value || "基线";
  return "主干";
});

const authorInfo = computed(() => {
  const a = fileDiff.value?.author ?? "";
  if (!a) return "";
  const bits = [a, fileDiff.value?.date, fileDiff.value?.subject].filter(Boolean);
  return bits.join(" · ");
});

/** 把 diff 拆成带类型的行，好按 `+`/`-`/`@@` 着色。
 *
 *  **不用让 git 输出 ANSI 颜色**：那又变成前端解析转义码 —— 正是旧版的做法（§1）。
 *  git 给纯文本，前端按前缀分类。 */
/**
 * 把 diff 拆成「带行号 + 带类型」的行。
 *
 * 行号从 `@@ -a,b +c,d @@` 头里解析，之后各自递增：
 * 上下文中旧新都在走，`+` 只走新行号，`-` 只走旧行号。没有行号的话，
 * 「这处改动在文件的第几行」说不清，也没法跟 IDE 对上（实测反馈）。
 *
 * **不让 git 输出 ANSI 颜色**：那又变成前端解析转义码 —— 正是旧版的做法（§1）。
 */
const diffLines = computed(() => {
  let oldNo = 0;
  let newNo = 0;
  return diff.value.split("\n").map((text) => {
    let cls = "ctx";
    let o = "";
    let n = "";
    const hunk = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(text);
    if (hunk) {
      cls = "hunk";
      oldNo = Number(hunk[1]);
      newNo = Number(hunk[2]);
    } else if (text.startsWith("+++") || text.startsWith("---")) {
      cls = "meta";
    } else if (
      text.startsWith("diff ") ||
      text.startsWith("index ") ||
      text.startsWith("new file") ||
      text.startsWith("deleted file") ||
      text.startsWith("similarity") ||
      text.startsWith("rename ")
    ) {
      cls = "meta";
    } else if (text.startsWith("+")) {
      cls = "add";
      n = String(newNo++);
    } else if (text.startsWith("-")) {
      cls = "del";
      o = String(oldNo++);
    } else if (text.startsWith("\\")) {
      cls = "meta"; // `\ No newline at end of file`
    } else {
      o = String(oldNo++);
      n = String(newNo++);
    }
    return { text, cls, old: o, new: n };
  });
});

async function openFile(path: string, src?: Source) {
  picked.value = path;
  selSource.value = src ?? sources.value[0] ?? null;
  loadingFile.value = true;
  fileErr.value = "";
  const use = selSource.value;
  if (!use) return;
  try {
    // **文件树只读内容**：它要的是"这个文件现在长什么样"，跟 git、跟分支状态
    // 都没关系。原来顺手把 diff 也一起拉了，于是「本需求的分支还没推送过」
    // 这种跟"看文件"毫不相干的错误也会冒出来（实测反馈）。
    // diff 是「差异」页的事，那里有对比目标可选。
    const onlyContent = mode.value === "tree";
    // 「与基线」的 ref 可能还没解析（比如直接从树里切过来）
    if (!onlyContent) await ensureBaselines();
    const c = await api.browseRead(use.dir, path);
    content.value = c;
    if (onlyContent) {
      diff.value = "";
      diffStat.value = "";
      fileDiff.value = null;
    } else {
      const d = await api.browseDiff(use.dir, use.repo, path, baseOf(use) ?? null);
      diff.value = d.diff;
      diffStat.value = d.stat;
      fileDiff.value = d;
    }
  } catch (e) {
    // ② 文件在本地不存在（比如 diff 里列出的"别人删掉的"文件）不是错误，
    //    给一句说明，**diff 该显示还得显示**
    fileErr.value = api.errText(e);
    content.value = "";
    if (mode.value !== "tree") {
      try {
        const d = await api.browseDiff(use.dir, use.repo, path, baseOf(use) ?? null);
        diff.value = d.diff;
        diffStat.value = d.stat;
        fileDiff.value = d;
      } catch {
        diff.value = "";
      }
    }
  } finally {
    loadingFile.value = false;
  }
}

function enterDir(name: string) {
  cwd.value = cwd.value ? `${cwd.value}/${name}` : name;
  void loadTree();
}

function upDir() {
  const parts = cwd.value.split("/").filter(Boolean);
  parts.pop();
  cwd.value = parts.join("/");
  void loadTree();
}

// 换了需求分支，旧的基线结果就作废（同一个仓库名在不同需求上可能命中不同基线）
watch(
  () => props.wsBranch,
  () => {
    baselineRefs.value = {};
    baselineErr.value = "";
  },
);

watch(
  () => [props.dir, props.repo, props.base, props.wsBranch, target.value, envForTarget.value, branchTarget.value],
  () => {
    picked.value = null;
    cwd.value = "";
    void loadChanged();
    if (!wholeWs.value) {
      void loadTree();
      void loadHistory();
    }
  },
);

/**
 * 切到哪一页，就把那一页的数据补上。
 *
 * 原来只在**挂载时**按"初始页"加载一次 —— 于是「对比代码」（以「差异」页打开）
 * 切到「文件树」时，`entries` 从没被读过，界面显示成「（空目录）」
 * （实测反馈：对比代码切文件树完全不显示内容）。而「浏览文件」正好以树页挂载，
 * 所以它是好的 —— 这个差异正是线索。
 */
watch(mode, (m) => {
  if (m === "tree") {
    // 整需求模式下"树"先列模块，不需要读目录
    if (!wholeWs.value) void loadTree();
  } else if (m === "history") {
    if (!history.value.length) void loadHistory();
  } else if (!sources.value.length) {
    void loadChanged();
  }
});

onMounted(async () => {
  // **只在自己那一页才去读 git**：文件树纯粹是列目录、读文件，跟 git 无关。
  if (mode.value === "tree") {
    // 文件树：只要「目录信息」（不读 git），再列目录
    await loadSources();
    if (!wholeWs.value) void loadTree();
  } else {
    void loadChanged();
    if (!wholeWs.value) void loadHistory();
  }
});
</script>

<template>
  <div class="fb">
    <div class="left">
      <!-- 标签栏**一直显示**：整需求模式原来把它藏了，于是切不到「文件树」，
           只能看到「差异」+ 对比目标那一行（实测反馈：文件树不需要对比） -->
      <div class="tabs">
        <button :class="{ on: mode === 'changed' }" @click="mode = 'changed'; picked = null">
          差异 ({{ changed.length }})
        </button>
        <button
          :class="{ on: mode === 'history' }"
          @click="mode = 'history'; picked = null; selCommit = null"
        >
          提交历史（已载入 {{ history.length }}）
        </button>
        <button :class="{ on: mode === 'tree' }" @click="mode = 'tree'; picked = null">
          文件树
        </button>
      </div>

      <!-- 对比目标：**单独一行**。原来跟「差异/提交历史/文件树」挤在一排，
           四个按钮加两个输入直接换行、错位（实测反馈）。 -->
      <div v-if="mode === 'changed' && !props.base" class="target-box">
        <div class="target">
          <span class="muted small">对比</span>
          <!-- 「与本地」= 工作区 vs 本地 HEAD（未提交的改动）。放在最前面并做默认：
               需求分支还没推送时，`origin/<分支>` 不存在，「与远程」只会报错，
               本地明明改了东西却什么都看不到（实测反馈）。 -->
          <button
            :class="{ on: target === 'worktree' }"
            title="工作区相对本地 HEAD 的改动（未提交的）；分支没推送也能看"
            @click="target = 'worktree'"
          >
            与本地
          </button>
          <button :class="{ on: target === 'remote' }" @click="target = 'remote'">与远程</button>
          <button :class="{ on: target === 'main' }" @click="target = 'main'">与主干</button>
          <button :class="{ on: target === 'env' }" @click="target = 'env'">与环境</button>
          <button :class="{ on: target === 'branch' }" @click="target = 'branch'">
            与远程分支
          </button>
          <button :class="{ on: target === 'local' }" @click="target = 'local'">
            与其它本地分支
          </button>
          <!-- 基线来自需求元数据（`meta.base` 链），**没有需求上下文时不能选** -->
          <button
            v-if="props.wsBranch"
            :class="{ on: target === 'baseline' }"
            @click="target = 'baseline'"
          >
            与基线
          </button>
        </div>
        <!-- **参数单独一行**：跟在按钮同一排会挤成一团、还会被压变形（实测反馈）。
             与环境：必须选；与远程分支：必须填。 -->
        <div v-if="target === 'env'" class="target-arg">
          <span class="muted small">选一个环境</span>
          <NSelect
            v-model:value="envForTarget"
            size="small"
            placeholder="必须选一个环境才能对比"
            :options="app.envs.map((e) => ({ label: e.label && e.label !== e.id ? `${e.label}（${e.id}）` : e.id, value: e.id }))"
            class="arg-input"
          />
        </div>
        <div v-if="target === 'branch' || target === 'local'" class="target-arg">
          <span class="muted small">{{ target === "branch" ? "远程分支名" : "本地分支名" }}</span>
          <NInput
            v-model:value="branchTarget"
            size="small"
            placeholder="如 feature-20261001-someone-else"
            class="arg-input"
            :input-props="NO_AUTO"
          />
          <span class="muted small">
            按{{ target === "branch" ? "远程分支 origin/" : "本地分支 " }}&lt;你填的&gt; 比
          </span>
        </div>
        <!-- 「与基线」解析结果：每个模块一行（基线链在不同仓库上可能命中不同分支） -->
        <div v-if="target === 'baseline'" class="target-arg baseline-note">
          <NSpin v-if="baselineLoading" size="small" />
          <span v-else-if="baselineErr" class="err small">{{ baselineErr }}</span>
          <template v-else>
            <span
              v-for="src in sources"
              :key="'bl' + src.dir"
              class="muted small baseline-item"
            >
              {{ sources.length > 1 ? src.label + "：" : "基线：" }}
              {{ baselineRefs[src.repo]?.reference ?? "…" }}
              <span v-if="baselineRefs[src.repo]?.fell_back_to_main" class="warn">
                （上游没有此分支，已退回主干）
              </span>
            </span>
          </template>
        </div>
      </div>

      <NInput v-model:value="q" :input-props="NO_AUTO" size="tiny" clearable placeholder="筛选文件…" />

      <!-- 改动汇总：几增几改几删，一眼看出这次改动的性质 -->
      <div v-if="totalChanged" class="summary">
        <span class="muted small">共 {{ totalChanged }} 个文件改动</span>
      </div>

      <!-- `min-height: 0` 不能少：flex 子项默认不会缩到内容以下，
           于是 overflow:auto 不生效、列表直接顶出可视区（实测反馈）。 -->
      <div class="list">
        <NSpin v-if="loadingList" size="small" />
        <p v-else-if="listErr" class="err small">{{ listErr }}</p>

        <template v-else>
          <!-- 文件树 -->
          <template v-if="mode === 'tree'">
            <!-- 整需求模式：先列模块（一个模块一个"根"），点进去再看文件 -->
            <template v-if="wholeWs && !treeSource">
              <button
                v-for="src in sources"
                :key="src.dir"
                class="file dir"
                @click="openTree(src)"
              >
                📁 {{ src.label }}
                <span v-if="src.label === docName" class="tag-mini">文档</span>
              </button>
            </template>
            <div v-if="!wholeWs || treeSource" class="crumbs">
              <NButton
                size="tiny"
                text
                :disabled="!cwd"
                @click="upDir"
              >
                ↑ 上一层
              </NButton>
              <NButton
                v-if="wholeWs"
                size="tiny"
                text
                @click="treeSource = null; entries = []"
              >
                ← 回到模块
              </NButton>
              <span class="mono small muted">
                {{ treeSource?.label ?? label }}{{ cwd ? "/" + cwd : "" }}
              </span>
            </div>
            <button
              v-for="row in treeRows.filter((r) => r.dir)"
              :key="'d' + row.key"
              class="file dir"
              @click="enterDir(row.key)"
            >
              📁 {{ row.label }}
            </button>
            <button
              v-for="row in treeRows.filter((r) => !r.dir)"
              :key="'f' + row.key"
              class="file"
              :class="{ on: picked === (cwd ? cwd + '/' + row.key : row.key) }"
              @click="openFile(cwd ? cwd + '/' + row.key : row.key, treeSource ?? undefined)"
            >
              <span class="path">{{ row.label }}</span>
            </button>
            <p v-if="!entries.length && treeLoaded" class="muted small">（空目录）</p>
          </template>

          <!-- 提交历史：先列提交，点一条再列它的文件变动 -->
          <template v-if="mode === 'history'">
            <template v-if="!selCommit">
              <button
                v-for="c in history"
                :key="c.sha"
                class="commit"
                :title="`${c.sha}\n${c.date} · ${c.author}\n${c.subject}`"
                @click="pickCommit(c)"
              >
                <div class="c-line">
                  <span class="sha mono">{{ c.sha }}</span>
                  <span class="subject">{{ c.subject }}</span>
                </div>
                <!-- 「什么时候、谁」按你说的显示出来 -->
                <div class="c-meta muted small">{{ c.date }} · {{ c.author }}</div>
              </button>
              <NButton
                v-if="maybeMoreHistory"
                size="tiny"
                class="more"
                :loading="loadingList"
                @click="moreHistory"
              >
                再看 20 条
              </NButton>
              <p v-if="!history.length" class="muted small">没有提交记录</p>
            </template>
            <template v-else>
              <div class="crumbs">
                <NButton size="tiny" text @click="selCommit = null; picked = null">
                  ← 回到提交列表
                </NButton>
                <span class="mono small muted">{{ selCommit.sha }}</span>
              </div>
              <p class="muted small c-subject">{{ selCommit.subject }}</p>
              <button
                v-for="f in commitFiles"
                :key="f.path"
                class="file"
                :class="{ on: picked === f.path }"
                :title="f.path"
                @click="openCommitFile(f.path)"
              >
                <span class="badge" :class="statusOf(f.status).cls">{{ statusOf(f.status).text }}</span>
                <span class="path">{{ f.path }}</span>
              </button>
              <p v-if="!commitFiles.length" class="muted small">这个提交没有文件变动</p>
            </template>
          </template>

          <!-- 改动：**按模块分组，全列在一屏里**（像 IDE 的变更树） -->
          <template v-else-if="mode === 'changed'">
            <p v-if="!wholeWs && srcErr[0]" class="muted small note">{{ srcErr[0] }}</p>
            <p v-else-if="!groups.length" class="muted small">
              <!-- 「与本地」只比未提交的改动 —— 空的时候要说清"确实没有未提交的改动"
                   以及去哪儿看已经提交的内容，免得又以为识别不出来（实测反馈） -->
              <template v-if="target === 'worktree' && !props.base">
                本地没有未提交的改动。已经提交到分支的内容用「与主干」看；推送之后「与远程」也能看。
              </template>
              <template v-else>相对{{ diffLabel }}没有文件改动</template>
            </p>
            <div v-for="(g, gi) in groups" :key="g.src.dir" class="group">
              <div v-if="wholeWs" class="group-head">
                <span class="mono">{{ g.src.label }}</span>
                <span class="muted small">{{ g.files.length }}</span>
              </div>
              <!-- 这个模块自己的情况，**友好提示**（通常是"这个仓库还没推过
                   这条分支"，属于正常状态，不是错误），不影响其它模块 -->
              <p v-if="srcErr[gi]" class="muted small note">{{ srcErr[gi] }}</p>
              <button
                v-for="f in g.files"
                :key="g.src.dir + f.path"
                class="file"
                :class="{ on: picked === f.path && selSource?.dir === g.src.dir }"
                :title="f.path"
                @click="openFile(f.path, g.src)"
              >
                <span class="badge" :class="statusOf(f.status).cls">{{ statusOf(f.status).text }}</span>
                <span class="path">{{ f.path }}</span>
              </button>
            </div>
          </template>
        </template>
      </div>
    </div>

    <div class="right">
      <template v-if="picked">
        <div class="file-head">
          <span class="mono">{{ selSource ? selSource.label + " / " : "" }}{{ picked }}</span>
          <!-- 谁改的：review 时「谁改的」和「改了什么」一样重要。
               看历史提交时显示的是那条提交的信息。 -->
          <span v-if="selCommit && mode === 'history'" class="muted small">
            {{ selCommit.date }} · {{ selCommit.author }} · {{ selCommit.subject }}
          </span>
          <span v-else-if="authorInfo" class="muted small">{{ authorInfo }}</span>
          <NButton size="tiny" @click="copy(picked!, '文件路径')">复制路径</NButton>
        </div>
        <NAlert v-if="fileErr" type="error" :bordered="false" class="small">{{ fileErr }}</NAlert>
        <template v-else>
          <!-- 文件树模式下**只显示内容**：那里就是"看这个文件长什么样"，
               不该出现"与远程的差异"这种跟分支状态有关的东西（实测反馈）。 -->
          <div v-if="mode !== 'tree'" class="rtabs">
            <button :class="{ on: pane === 'diff' }" @click="pane = 'diff'">
              {{ diffLabel }}
            </button>
            <button :class="{ on: pane === 'content' }" @click="pane = 'content'">内容</button>
          </div>

          <div class="pane">
            <NSpin v-if="loadingFile" size="small" />
            <!-- 文件树：直接给内容，没有第二层标签 -->
            <div v-else-if="mode === 'tree'" class="code">
              <p v-if="!content" class="muted small">（这个文件读不出来，可能已被删除）</p>
              <div v-for="(l, i) in contentLines" :key="i" class="line ctx">
                <span class="no">{{ i + 1 }}</span><span class="no"></span><span class="txt">{{ l || " " }}</span>
              </div>
            </div>
            <template v-else-if="pane === 'diff'">
              <pre v-if="diffStat" class="stat">{{ diffStat }}</pre>
              <div v-if="diff" class="code">
                <div
                  v-for="(l, i) in diffLines"
                  :key="i"
                  class="line"
                  :class="l.cls"
                ><span class="no">{{ l.old }}</span><span class="no">{{ l.new }}</span><span class="txt">{{ l.text || " " }}</span></div>
              </div>
              <p v-else class="muted small">
                该文件相对{{ baseShort }}没有改动
              </p>
            </template>
            <div v-else class="code">
              <div v-for="(l, i) in contentLines" :key="i" class="line ctx"><span class="no">{{ i + 1 }}</span><span class="no"></span><span class="txt">{{ l || " " }}</span></div>
            </div>
          </div>
        </template>
      </template>
      <NEmpty v-else description="左侧选一个文件" class="mt" />
    </div>
  </div>
</template>

<style scoped>
.fb {
  display: grid;
  /* 左列**拉宽**：300px 时文件路径和状态徽标挤在一起要换行（实测反馈）。
     路径普遍不短（`specs/模块A-被回复悬浮气泡/verify/x.verify.cjs`），
     窄了就只能狂省略号，看不出是哪个文件。 */
  grid-template-columns: minmax(360px, 26%) 1fr;
  gap: 12px;
  /* **高度跟着弹窗走，不再写死 520px**。
     弹窗是 88vh，而这里写死 520 —— 窗口一大，下面就是一大片空白（实测反馈）。
     用 calc 减掉卡片的头/脚/内边距（约 170px）；估偏了最多是多一条滚动条，
     总比空着强。min-height 兜住小窗口。 */
  height: calc(88vh - 170px);
  min-height: 320px;
}
.left {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
  /* flex 子项默认 min-height:auto → 不会缩到内容以下，overflow:auto 就失效、
     列表会顶出可视区。两个都要显式给 0。 */
  min-height: 0;
}
.right {
  min-width: 0;
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
}
.rtabs {
  display: flex;
  gap: 6px;
  flex: none;
  margin-bottom: 6px;
}
.rtabs button {
  border: 1px solid var(--xg-border);
  background: none;
  border-radius: 5px;
  padding: 3px 12px;
  cursor: pointer;
  font-size: 12px;
  color: inherit;
}
.rtabs button.on {
  background: rgba(37, 99, 235, 0.1);
  border-color: #2563eb;
}
/* 右侧的滚动容器：**吃掉剩余高度**并自己滚。
   它必须真的能缩（min-height: 0），否则 flex 子项的下限是内容高度、
   overflow 形同虚设 —— 内容被裁掉，滚不动。 */
.pane {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.tabs {
  display: flex;
  gap: 6px;
}
.tabs button {
  flex: 1;
  border: 1px solid var(--xg-border);
  background: none;
  border-radius: 5px;
  padding: 4px 0;
  cursor: pointer;
  font-size: 12px;
  color: inherit;
}
.tabs button.on {
  background: rgba(37, 99, 235, 0.1);
  border-color: #2563eb;
}
.tag-mini {
  margin-left: 6px;
  font-size: 11px;
  color: #2563eb;
}
.crumbs {
  display: flex;
  align-items: center;
  gap: 8px;
  /* **粘在顶部**：文件一多，"回到提交列表"会滚出可视区，就"回不去了"
     （实测反馈）。分组标题也用了同样的手法。 */
  position: sticky;
  top: 0;
  z-index: 1;
  background: var(--n-color, #fff);
  padding: 2px 0;
}
.list {
  flex: 1;
  min-height: 0;
  overflow: auto;
  border: 1px solid var(--xg-border);
  border-radius: 6px;
  padding: 4px;
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.file {
  display: flex;
  align-items: center;
  gap: 6px;
  /* **不能被挤压**：`.list` 是 flex 列容器，子项默认 shrink，文件一多就全挤成一坨 */
  flex: none;
  text-align: left;
  border: none;
  background: none;
  padding: 3px 6px;
  border-radius: 4px;
  cursor: pointer;
  font-family: var(--mono);
  font-size: 12px;
  color: inherit;
  white-space: nowrap;
  overflow: hidden;
  width: 100%;
}
.file .path {
  overflow: hidden;
  text-overflow: ellipsis;
}
/* 改动状态徽标：新增/修改/删除用不同底色 —— review 时这三者性质完全不同 */
.badge {
  flex: none;
  border-radius: 3px;
  padding: 0 4px;
  font-size: 11px;
  font-family: system-ui, sans-serif;
}
.summary {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
  align-items: center;
}
.dir-hint {
  font-size: 12px;
  color: #888;
}
.dir-hint.warn {
  color: #f0a020;
  font-weight: 600;
}
.target-box {
  flex: none;
  padding: 4px 0 6px;
  border-bottom: 1px solid var(--xg-border);
  margin-bottom: 4px;
}
.target {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}
.target-arg {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 6px;
  flex-wrap: wrap;
}
.arg-input {
  width: 260px;
}
.baseline-item {
  white-space: nowrap;
}
.baseline-item + .baseline-item {
  margin-left: 12px;
}
.warn {
  color: #f0a020;
  font-weight: 600;
}
.target button {
  white-space: nowrap;
}
.target button {
  border: 1px solid var(--xg-border);
  background: none;
  border-radius: 4px;
  padding: 2px 8px;
  cursor: pointer;
  font-size: 12px;
  color: inherit;
}
.target button.on {
  background: rgba(37, 99, 235, 0.12);
  border-color: #2563eb;
}
.env-pick {
  width: 120px;
}
.branch-input {
  width: 160px;
}
.more {
  align-self: flex-start;
  margin: 6px;
}
.commit {
  display: flex;
  flex-direction: column;
  gap: 2px;
  flex: none;
  text-align: left;
  border: none;
  background: none;
  padding: 5px 6px;
  border-radius: 4px;
  cursor: pointer;
  color: inherit;
  border-bottom: 1px solid var(--xg-border);
}
.commit:hover {
  background: rgba(128, 128, 128, 0.1);
}
.c-line {
  display: flex;
  align-items: baseline;
  gap: 6px;
}
.c-line .sha {
  flex: none;
  color: #888;
  font-size: 11px;
}
.c-line .subject {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
}
.c-meta {
  padding-left: 0;
}
.c-subject {
  margin: 2px 0 6px;
  white-space: normal;
}
.group + .group {
  margin-top: 8px;
}
/* 分组标题：模块名，粘在组上方（像 IDE 的变更树） */
.group-head {
  display: flex;
  align-items: baseline;
  gap: 6px;
  padding: 3px 6px;
  font-size: 12px;
  font-weight: 600;
  color: #888;
  border-bottom: 1px solid var(--xg-border);
  margin-bottom: 2px;
  position: sticky;
  top: 0;
  background: var(--n-color, transparent);
}
.badge.add {
  background: rgba(24, 160, 88, 0.18);
  color: #18a058;
}
.badge.mod {
  background: rgba(240, 160, 32, 0.2);
  color: #c47f0a;
}
.badge.del {
  background: rgba(208, 48, 80, 0.16);
  color: #d03050;
}
.badge.ren {
  background: rgba(37, 99, 235, 0.16);
  color: #2563eb;
}
.file:hover {
  background: rgba(128, 128, 128, 0.1);
}
.file.on {
  background: rgba(37, 99, 235, 0.15);
}
.file.dir {
  color: #2563eb;
}
.file-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
  margin-bottom: 6px;
  font-size: 12px;
}
/* diff / 内容区：定高 + 滚动，**不换行**（保留缩进，代码才看得懂） */
.code {
  flex: 1;
  min-height: 0;
  overflow: auto;
  border: 1px solid var(--xg-border);
  border-radius: 6px;
  font-family: var(--mono);
  font-size: 12px;
  line-height: 1.6;
  background: rgba(128, 128, 128, 0.04);
}
.line {
  display: flex;
  white-space: pre;
}
/* 行号槽：定宽、右对齐、不参与选中（复制代码时不该把行号带进去） */
.line .no {
  flex: none;
  width: 3.2em;
  padding-right: 6px;
  text-align: right;
  color: #888;
  user-select: none;
  opacity: 0.65;
}
.line .txt {
  flex: 1;
  padding-right: 10px;
}
/* 逐行着色：git 给的是纯文本，分类由前端按前缀做（不让 git 出 ANSI 颜色） */
.line.add {
  background: rgba(24, 160, 88, 0.14);
  color: #18a058;
}
.line.del {
  background: rgba(208, 48, 80, 0.13);
  color: #d03050;
}
.line.hunk {
  background: rgba(37, 99, 235, 0.12);
  color: #2563eb;
}
.line.meta {
  color: #888;
}
.stat {
  margin: 0 0 6px;
  padding: 6px 10px;
  border: 1px solid var(--xg-border);
  border-radius: 6px;
  font-family: var(--mono);
  font-size: 12px;
  white-space: pre-wrap;
  flex: none;
}
.muted {
  color: #888;
}
.small {
  font-size: 12px;
}
.err {
  color: #d03050;
}
/* 缺分支这类是**正常状态**，不是错误 —— 用中性色 + 缩进，别做成红字报警 */
.note {
  margin: 4px 6px;
  padding: 6px 8px;
  border-left: 2px solid var(--xg-border);
  white-space: pre-wrap;
  line-height: 1.5;
}
.mt {
  margin-top: 60px;
}
</style>