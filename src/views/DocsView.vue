<script setup lang="ts">
//! 文档：**按需求分类管理跟这个需求有关的文档**（实测反馈后的重做）。
//!
//! # 想要的样子
//!
//! - 先列出**有哪些需求分类**（折叠着，不铺开一堆）
//! - 点一个族**展开**，里面**一个分支一条** —— 一条 = 一个开发阶段产生的文档
//! - 每条写死**状态 + 时间**；顺序：**开发中在前，已归档在后**，各自按时间倒序
//! - 点某条 → 看**它那一份目录**下的文档，别的目录的不显示。
//!   同一期在本地可能有两份拷贝，规则是**优先取生产环境那份，取不到再退工作区那份**
//!   （详见 `lib/docPath.ts`）：
//!     - 生产环境 → `repos/doc-hub/workspaces/<族>/<分支>/`（已并主干 ⇒ 已上线）
//!     - 工作区 → `ws/<分支>/doc-hub/workspaces/<族>/<分支>/`（还在开发）
//!   界面上标出内容来自哪一份；「打开目录」「复制目录」「浏览文件」用的是**同一个**
//!   答案 —— 显示的是一份、点开跳到另一份，那种不一致比看不到更难查
//!
//! # 为什么去掉了 V1 / V2 标签
//!
//! 实测反馈：用处不大，反而管理麻烦（要人工维护，而且各人本地不同步）。
//! 状态和时间才是真正要看的。`meta.json` 里的 `label` 字段仍保留（兼容已有数据），
//! 只是界面不再显示。

import { computed, onMounted, ref, watch } from "vue";
import { NAlert, NButton, NCard, NInput, NModal, NTag, useMessage } from "naive-ui";

import * as api from "../bridge";
import { openInFinder } from "../bridge";
import { useAppStore } from "../stores/app";
import { docDirCandidates, docWhereLabel, type DocDir } from "../lib/docPath";
import { NO_AUTO } from "../lib/inputs";
import { useCopy } from "../lib/copy";
import EditorButton from "../components/EditorButton.vue";
import FileBrowser from "../components/FileBrowser.vue";
import DocTree from "../components/DocTree.vue";
import type { DocNode, Family, PeriodWithState } from "../types";
import { visiblePeriods } from "../lib/docVisibility";

const app = useAppStore();
/** 文档库在 hub 里的目录名（从配置读，别写死）。 */
const docName = computed(() => app.hub?.dochub_name ?? "doc-hub");

const msg = useMessage();
const copy = useCopy();

const families = ref<Family[]>([]);
const q = ref("");
const syncing = ref(false);
const err = ref("");
const loading = ref(true);
/** 展开的族（点标题切换）。允许同时展开多个 —— 想对照两个族时不被挡住。 */
const expanded = ref<Set<string>>(new Set());
const selected = ref<{ family: string; period: PeriodWithState } | null>(null);
/** 这一份文档目录的**树**（目录 + 文件，递归）。 */
const tree = ref<DocNode[]>([]);
const browse = ref<{ dir: string; repo: string; label: string } | null>(null);
/**
 * 这一步解析出来的那一份目录（两份拷贝里实际取到的那份）。
 *
 * 树、打开目录、复制目录、浏览文件**全用它**：各自再按 `merged` 推一遍路径的话，
 * 一旦哪份取不到就会各推各的，显示的和点开的不是同一份。
 */
const resolved = ref<DocDir | null>(null);

onMounted(() => void load());

/** Agent 在外部改了文档之后，回到前台也重读一遍（列表刷新时顺带）。 */
watch(
  () => app.refreshTick,
  () => void load(),
);

async function load() {
  loading.value = true;
  try {
    families.value = await api.docFamilies();
    err.value = "";
    // 只有一个族时默认展开，省一次点击
    if (families.value.length === 1) expanded.value = new Set([families.value[0].name]);
  } catch (e) {
    err.value = api.errText(e);
  } finally {
    loading.value = false;
  }
}

async function syncFromRemote() {
  syncing.value = true;
  try {
    await api.dochubEnsure();
    await load();
    msg.success("文档库已更新到线上最新");
  } catch (e) {
    msg.error(api.errText(e));
  } finally {
    syncing.value = false;
  }
}

async function ensureLibrary() {
  try {
    await api.dochubEnsure();
    await load();
    msg.success("文档库已就绪");
  } catch (e) {
    msg.error(api.errText(e));
  }
}

/**
 * 这一期实际读的那份目录。
 *
 * 已解析出来就用它；还没解析（刚点开、还在探测）时按优先级给第一个候选 ——
 * 只是给按钮一个落脚点，用户看到内容后 `resolved` 就定了。
 */
function periodDir(family: string, p: PeriodWithState): string {
  if (resolved.value) return resolved.value.dir;
  const root = app.hub?.root ?? "";
  return docDirCandidates(root, docName.value, family, p.branch, p.merged)[0].dir;
}

/** 过滤后还有内容的族（可见性规则见 `lib/docVisibility`）。 */
const shownFamilies = computed(() =>
  families.value
    .map((f) => ({ name: f.name, periods: visiblePeriods(f, q.value) }))
    .filter((f) => f.periods.length > 0),
);

const developing = computed(() =>
  shownFamilies.value.reduce((n, f) => n + f.periods.filter((p) => !p.merged).length, 0),
);

function toggle(name: string) {
  const next = new Set(expanded.value);
  if (next.has(name)) next.delete(name);
  else next.add(name);
  expanded.value = next;
}

/**
 * 点一条期次：列**它那一份目录**下的文档 —— **整棵树**。
 *
 * 原来只列顶层文件，`sql/` 这类目录整个看不见（实测反馈）。
 * 每个分支的文档本来就不多，所以一次把整棵树读完，界面上全展开。
 */
async function pick(family: string, p: PeriodWithState) {
  selected.value = { family, period: p };
  tree.value = [];
  resolved.value = null;

  // **按优先级逐个试**：优先生产环境那份（已上线的那份），取不到再退工作区那份。
  // 只试一个路径的话，那一份恰好没同步下来时界面就是空的 ——
  // 而另一份明明有内容（实测反馈就卡在这个形状上）。
  const root = app.hub?.root ?? "";
  for (const c of docDirCandidates(root, docName.value, family, p.branch, p.merged)) {
    const entries = await api.listEntries(c.dir).catch(() => null);
    if (entries === null) continue; // 这一份不在 → 试下一份
    resolved.value = c;
    tree.value = await walk(c.dir);
    return;
  }
  // 两份都没有：多半是还没同步（已归档那份要拉过才有），界面会给出提示
}

/**
 * 递归读一棵目录树。
 *
 * 深度与节点数都设上限：文档目录正常只有几十个文件，但**万一有人把 node_modules
 * 之类的东西放进文档目录**，递归下去会把 webview 卡住。上限到了就停下。
 */
async function walk(dir: string, depth = 0): Promise<DocNode[]> {
  if (depth > 6) return [];
  const entries = (await api.listEntries(dir).catch(() => [])).filter(
    // macOS 的目录元数据，不是文档 —— 列出来只是干扰（实测反馈里就有它）
    (e) => e.name !== ".DS_Store",
  );
  if (entries.length > 500) {
    msg.warning(`${dir} 下条目过多（${entries.length}），只列前 500 条`);
  }
  const dirs = entries.filter((e) => e.is_dir).slice(0, 200);
  const files = entries.filter((e) => !e.is_dir).slice(0, 500);
  // 目录在前、文件在后，各自按名排序 —— 跟文件管理器一致
  const children = await Promise.all(dirs.map(async (e) => ({
    name: e.name,
    abs: e.path,
    dir: true,
    children: await walk(e.path, depth + 1),
  })));
  return [
    ...children,
    ...files.map((e) => ({ name: e.name, abs: e.path, dir: false })),
  ];
}

/** 浏览这一份文档目录（内容 / 差异 / 提交历史）。 */
function browseSelected() {
  const sel = selected.value;
  if (!sel) return;
  browse.value = {
    dir: periodDir(sel.family, sel.period),
    repo: docName.value,
    label: `${sel.family} · ${sel.period.branch}`,
  };
}

function statusType(merged: boolean): "default" | "success" {
  return merged ? "default" : "success";
}
</script>

<template>
  <div class="page">
    <div class="head">
      <h2 class="title">文档</h2>
      <div class="tools">
        <NInput
          v-model:value="q"
          size="small"
          clearable
          placeholder="筛选需求分类 / 分支…"
          class="search"
          :input-props="NO_AUTO"
        />
        <NButton size="small" @click="load">刷新</NButton>
        <NButton size="small" type="primary" :loading="syncing" @click="syncFromRemote">
          同步最新文档
        </NButton>
        <NButton size="small" @click="openInFinder(`${app.hub?.root}/repos/${docName}`)">
          打开文档库
        </NButton>
      </div>
    </div>

    <NAlert v-if="err" type="warning" :bordered="false" class="mb">
      <div>{{ err }}</div>
      <div class="alert-actions">
        <NButton size="tiny" text type="primary" @click="ensureLibrary">拉取文档库</NButton>
      </div>
    </NAlert>

    <p class="muted small mb">
      只列出跟当前需求有关的文档：一个分支目录 = 一个开发阶段产生的文档。<br />
      已上线（已并入主干）的是历史记录，只作查阅；新文档写在当前需求里。
    </p>

    <div class="split">
      <!-- ── 左：需求分类（折叠 → 展开）── -->
      <div class="list">
        <div class="list-head">
          <span class="muted small">
            {{ shownFamilies.length }} 个需求分类 · {{ developing }} 个开发中
          </span>
        </div>

        <div v-for="f in shownFamilies" :key="f.name" class="family">
          <button class="fam-head" @click="toggle(f.name)">
            <span class="caret">{{ expanded.has(f.name) ? "▾" : "▸" }}</span>
            <span class="fam-name">{{ f.name }}</span>
            <span class="muted small">{{ f.periods.length }}</span>
          </button>

          <div v-if="expanded.has(f.name)" class="periods">
            <button
              v-for="p in f.periods"
              :key="p.branch"
              class="period"
              :class="{ on: selected?.period.branch === p.branch, dim: p.merged }"
              @click="pick(f.name, p)"
            >
              <!-- 状态 + 时间**写死显示**：这是判断"该不该改"的唯一依据 -->
              <!-- 判据是**内容在不在主干上**（= 上没上线），不是文档里自己写的状态 ——
                   文档里那个 `status` 是人手填的，上线之后没人会回去改它。
                   实测反馈：明明已经上线了，这里却还写着「开发中」。 -->
              <NTag size="tiny" :bordered="false" :type="statusType(p.merged)">
                {{ p.merged ? "已上线" : "开发中" }}
              </NTag>
              <!-- 需求名比分支名友好得多（实测反馈：只给分支名不好认）——
                   放在前面当主标识，分支名退成灰色附注 -->
              <span class="name">{{ p.name || p.branch }}</span>
              <span class="date">{{ p.date }}</span>
              <span class="mono branch muted">{{ p.branch }}</span>
            </button>
          </div>
        </div>

        <p v-if="!shownFamilies.length && !loading" class="muted">
          {{ q ? "没有匹配的文档" : "文档库还没有内容。新建需求时会自动建期次目录。" }}
        </p>
      </div>

      <!-- ── 右：这一条分支目录下的文档 ── -->
      <div class="detail">
        <NCard v-if="selected" size="small">
          <template #header>
            <span>{{ selected.family }}</span>
            <NTag
              size="tiny"
              :bordered="false"
              class="ml"
              :type="statusType(selected.period.merged)"
            >
              {{ selected.period.merged ? "已上线" : "开发中" }}
            </NTag>
          </template>
          <template #header-extra>
            <div class="hdr-actions">
              <!-- 直接在系统文件管理器里打开**这一期**的目录 —— 写文档时
                   用编辑器的文件和用界面看是两回事，这一步省得从 hub 根一层层点进去 -->
              <NButton size="tiny" @click="openInFinder(periodDir(selected.family, selected.period))">
                打开目录
              </NButton>
              <EditorButton size="tiny" :path="periodDir(selected.family, selected.period)" />
              <NButton size="tiny" @click="browseSelected">浏览文件</NButton>
              <NButton
                size="tiny"
                @click="copy(periodDir(selected.family, selected.period), '目录')"
              >
                复制目录
              </NButton>
            </div>
          </template>

          <div class="kv">
            <span class="k">分支</span><span class="mono">{{ selected.period.branch }}</span>
          </div>
          <div class="kv"><span class="k">需求名</span><span>{{ selected.period.name }}</span></div>
          <div class="kv"><span class="k">日期</span><span>{{ selected.period.date }}</span></div>
          <!-- 状态只看**现实**：在不在主干上（= 上没上线）。
               文档里手填的 status 已经移除——没人维护它，显示出来就是假消息。 -->
          <div class="kv">
            <span class="k">状态</span>
            <span>{{ selected.period.merged ? "已上线（已并入主干）" : "开发中（还没并主干）" }}</span>
          </div>
          <div v-if="selected.period.summary" class="kv">
            <span class="k">说明</span><span>{{ selected.period.summary }}</span>
          </div>
          <div v-if="selected.period.repos.length" class="kv">
            <span class="k">涉及模块</span>
            <span class="mono small">{{ selected.period.repos.join("、") }}</span>
          </div>

          <div class="docs">
            <div class="docs-head muted small">
              这一份文档目录（全展开）
              <!-- 标出内容到底来自哪一份：同一期本地可能有两份拷贝
                   （生产环境那份 / 工作区那份），不标就分不清在看哪一份 -->
              <span v-if="resolved" class="where">· 来自 {{ docWhereLabel(resolved.where) }}</span>
            </div>
            <div v-if="tree.length" class="files">
              <DocTree :nodes="tree" />
            </div>
            <p v-else class="muted small">
              {{
                selected.period.merged
                  ? "生产环境（main）的文档库里还没有这一期的内容 —— 多半是还没拉取，点上面的「同步最新文档」。工作区那份也没有。"
                  : "这一期的文档目录还没挂上，或内容为空（生产环境那份也还没有 —— 还没并主干）"
              }}
            </p>
          </div>
        </NCard>
        <p v-else class="muted">左侧展开一个族，再点一条分支</p>
      </div>
    </div>

    <NModal
      v-if="browse"
      :show="true"
      @update:show="(v: boolean) => !v && (browse = null)"
      preset="card"
      :title="`浏览文件 · ${browse.label}`"
      style="width: 92vw; height: 88vh"
      @close="browse = null"
    >
      <FileBrowser
        :dir="browse.dir"
        :repo="browse.repo"
        :label="browse.label"
        default-mode="tree"
      />
    </NModal>
  </div>
</template>

<style scoped>
.page {
  padding: 20px 24px 40px;
}
.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 12px;
  flex-wrap: wrap;
}
.title {
  margin: 0;
  font-size: 18px;
  font-weight: 600;
}
.tools {
  display: flex;
  align-items: center;
  gap: 8px;
}
.search {
  width: 220px;
}
.mb {
  margin-bottom: 10px;
}
.ml {
  margin-left: 6px;
}
.muted {
  color: #888;
  font-size: 12px;
}
.small {
  font-size: 12px;
}
.split {
  display: grid;
  grid-template-columns: minmax(360px, 38%) 1fr;
  gap: 16px;
  align-items: start;
}
.list {
  border: 1px solid var(--xg-border);
  border-radius: 6px;
  max-height: calc(88vh - 260px);
  overflow: auto;
}
.list-head {
  padding: 6px 10px;
  border-bottom: 1px solid var(--xg-border);
  position: sticky;
  top: 0;
  background: var(--sider-bg, #fff);
}
.family + .family {
  border-top: 1px solid var(--xg-border);
}
/* 族标题：整行可点，箭头表示展开状态 */
.fam-head {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  border: none;
  background: none;
  padding: 8px 10px;
  cursor: pointer;
  color: inherit;
  text-align: left;
  font-size: 13px;
}
.fam-head:hover {
  background: rgba(128, 128, 128, 0.07);
}
.caret {
  width: 12px;
  color: #888;
}
.fam-name {
  font-weight: 600;
  flex: 1;
}
.periods {
  padding: 0 0 6px;
}
/* 一条 = 一个分支目录 = 一个开发阶段 */
.period {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  border: none;
  background: none;
  padding: 5px 10px 5px 30px;
  cursor: pointer;
  color: inherit;
  font-size: 12px;
  text-align: left;
}
.period:hover {
  background: rgba(128, 128, 128, 0.07);
}
.period.on {
  background: rgba(37, 99, 235, 0.1);
}
/* 已归档是背景资料，视觉上退一档（但不隐藏） */
.period.dim {
  opacity: 0.75;
}
.date {
  color: #888;
  flex: none;
}
.branch {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.hdr-actions {
  display: flex;
  gap: 6px;
}
.kv {
  display: grid;
  grid-template-columns: 5em 1fr;
  gap: 8px;
  padding: 3px 0;
  font-size: 13px;
}
.k {
  color: #888;
}
.docs {
  margin-top: 12px;
  padding-top: 10px;
  border-top: 1px solid var(--xg-border);
}
.docs-head {
  margin-bottom: 6px;
}
/* 「来自生产环境 / 工作区」那半句弱化一档，别跟主体说明抢注意力 */
.docs-head .where {
  color: var(--n-text-color-3, #999);
}
.files {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.file {
  display: flex;
  flex: none;
  border: none;
  background: none;
  padding: 4px 6px;
  border-radius: 4px;
  cursor: pointer;
  color: #2563eb;
  font-size: 13px;
  text-align: left;
}
.file:hover {
  background: rgba(128, 128, 128, 0.1);
}
</style>