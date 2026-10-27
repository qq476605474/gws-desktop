<script setup lang="ts">
import { useCopy } from "../lib/copy";
import { computed, onMounted, ref, watch } from "vue";
import { NAlert, NButton, NCard, NInput, NSelect, NTag, useDialog, useMessage } from "naive-ui";

import * as api from "../bridge";
import { openUrl } from "../bridge";
import * as fmt from "../format";
import { NO_AUTO } from "../lib/inputs";
import ModulePicker from "./ModulePicker.vue";
import type { GitlabStatus, MergeRequest, ModuleStatus, MrPlan, MrRow, ReviewerInfo } from "../types";

const props = defineProps<{ branch: string; modules: ModuleStatus[] }>();
const emit = defineEmits<{ (e: "changed"): void }>();
const msg = useMessage();
const copy = useCopy();
const dialog = useDialog();

const plan = ref<MrPlan | null>(null);
const gl = ref<GitlabStatus | null>(null);
const reviewers = ref<ReviewerInfo | null>(null);
/** 评审人候选（输入即搜）。 */
const userOptions = ref<{ label: string; value: string }[]>([]);
const frontDraft = ref("");
const backDraft = ref("");
const searching = ref(false);
const creating = ref(false);
/**
 * 本次要提的模块。**默认全选**。
 *
 * 有些模块已经提过了、有些这次不想动，所以必须能选（实测反馈 #6）。
 * core 一直支持 `only`，是界面没把它传下去。
 */
const picked = ref<string[]>([]);
/**
 * 模块选择区是否展开。
 *
 * **默认展开**：提 MR 前先看清这次要提哪些模块，是这一步的主要动作；
 * 折叠着的话人容易直接点「预览」而没注意范围（实测反馈）。
 * 提完一次才收起（省得占地方）。
 */
const showPicker = ref(true);

/**
 * MR 标题/描述 —— **可以改，改了就按原样提交**（实测反馈 #11）。
 *
 * 默认值由后端给（`需求上线-需求名`），这样 CLI 和桌面端口径一致。
 * 用户没动过时，预览返回的最终标题会回填进来；动过就**不覆盖**。
 */
const title = ref("");
const description = ref("");
const titleEdited = ref(false);
const descriptionEdited = ref(false);
const loadingDefaults = ref(false);

const names = computed(() => props.modules.filter((m) => m.present).map((m) => m.module));

// 默认全选。父组件换了需求就重来一遍。
watch(
  names,
  (n) => {
    if (!n.length) return;
    // 只在"还没选过"或"选择已经全部失效"时重置，免得把人的勾选冲掉
    if (!picked.value.length || !picked.value.some((x) => n.includes(x))) {
      picked.value = [...n];
    }
  },
  { immediate: true },
);

onMounted(async () => {
  // 默认标题/描述是纯本地的，**不联网** —— 先填上，用户打开就能看到自己要提交什么
  void loadDefaults();
  try {
    gl.value = await api.gitlabStatus();
    if (!gl.value.user) return; // 没连通就不做后续，界面会提示去设置
    reviewers.value = await api.mrReviewers(props.branch);
    frontDraft.value = reviewers.value.frontend;
    backDraft.value = reviewers.value.backend;
    // **不在这里搜接口**。候选先只放「当前配置里的那两个人」，
    // 用户想换人时**输入才触发**远程检索 —— 否则一个纯本地的信息要等网络，
    // 网络一慢整个面板就转圈（实测反馈 #1）。
    seedReviewerOptions();
    // **打开面板不自动跑预览**：预览要打 GitLab（查用户、探目标分支），
    // 一进面板就弹 busy 遮罩，非常影响体验（实测反馈）。改成显式点「预览」。
    // 面板先把「不需要网络就能知道的东西」摆出来（评审人、分支约定）。
  } catch (e) {
    msg.error(api.errText(e));
  }
});

async function loadDefaults() {
  loadingDefaults.value = true;
  try {
    const d = await api.mrDefaults(props.branch);
    if (!titleEdited.value) title.value = d.title;
    if (!descriptionEdited.value) description.value = d.description;
  } catch {
    // 拿不到默认值也不挡路：留空时后端仍会用自己的默认
  } finally {
    loadingDefaults.value = false;
  }
}

/**
 * 把当前评审人放进候选项，好让下拉显示得出「已选中的是谁」。
 *
 * **不联网**：开源版没有内置默认评审人，也没有编译期就知道的中文名，
 * 所以直接显示 username。用户想按姓名找人时，输入才触发远程检索。
 */
function seedReviewerOptions() {
  const seen = new Set<string>();
  userOptions.value = [frontDraft.value, backDraft.value]
    .filter((u) => u && !seen.has(u) && seen.add(u))
    .map((u) => ({ label: u, value: u }));
}

/**
 * 评审人下拉变化时**只做本地的事，不发任何请求**。
 *
 * 清除的意图是「回到默认那两位」，所以清空就恢复默认；纯本地操作该是瞬时的
 * （原来清除会触发 `preview()` 打 GitLab，卡一下）。
 *
 * # 但要**保住用户搜出来的人**
 *
 * 原来这里无条件 `seedReviewerOptions()`，于是刚搜出来的人被候选重置冲掉 ——
 * 表现就是「改成别人改不了」（实测反馈）。现在只在**这一项被清空**时才重置候选；
 * 选了人就把选中项并进候选，标签才显示得出来。
 */
function onReviewerChange(which: "front" | "back") {
  if (which === "front") {
    if (!frontDraft.value) {
      frontDraft.value = reviewers.value?.frontend ?? "";
      seedReviewerOptions();
    } else {
      keepOption(frontDraft.value);
    }
  } else {
    if (!backDraft.value) {
      backDraft.value = reviewers.value?.backend ?? "";
      seedReviewerOptions();
    } else {
      keepOption(backDraft.value);
    }
  }
}

/** 把人并进候选（没有就给个兜底标签），否则下拉显示不出选中的是谁。 */
function keepOption(username: string) {
  if (userOptions.value.some((o) => o.value === username)) return;
  userOptions.value = [{ label: username, value: username }, ...userOptions.value];
}

/** 没改就传 null（后端按默认算），改了原样传。 */
function metaPayload() {
  return {
    title: titleEdited.value ? title.value : null,
    description: descriptionEdited.value ? description.value : null,
  };
}

/** 预览返回的最终标题/描述回填到**没被用户改过**的输入框。 */
function applyPlanMeta(p: MrPlan) {
  if (!titleEdited.value) title.value = p.title;
  if (!descriptionEdited.value) description.value = p.description;
}

/** 预览（dry-run）。不写任何东西，可安全重跑。 */
async function preview() {
  try {
    plan.value = await api.mrPlan({
      branch: props.branch,
      ...metaPayload(),
      frontend_reviewer: frontDraft.value || null,
      backend_reviewer: backDraft.value || null,
      only: picked.value,
    });
    applyPlanMeta(plan.value);
  } catch (e) {
    msg.error(api.errText(e));
  }
}

async function searchUsers(q: string) {
  // 关键词清空 → **直接清空候选**，不要再去打一次接口。
  // 原来 `return` 会让上一次的结果留在列表里，看着像"搜出来一堆无关的人"；
  // 而且清空输入是最常见的动作，每次都发请求纯属浪费（实测反馈）。
  if (!q.trim()) {
    // 关键词清空 → 候选回到「当前这两个人」（不是清空，那会让选中的标签显示不出来）
    userOptions.value = [frontDraft.value, backDraft.value]
      .filter(Boolean)
      .map((u) => ({ label: u, value: u }));
    return;
  }
  searching.value = true;
  try {
    const users = await api.gitlabSearchUsers(q, 20);
    // 合并而不是覆盖：覆盖会把已经回显出来的评审人候选冲掉
    const merged = new Map(userOptions.value.map((o) => [o.value, o]));
    for (const u of users) merged.set(u.username, { label: `${u.name}（${u.username}）`, value: u.username });
    userOptions.value = [...merged.values()];
  } catch (e) {
    msg.error(api.errText(e));
  } finally {
    searching.value = false;
  }
}

async function create() {
  creating.value = true;
  try {
    // 评审人改动落盘到需求级配置，下次打开弹窗就是这个值
    await api.mrSaveReviewers(props.branch, frontDraft.value, backDraft.value);
    const p = await api.mrCreate({
      branch: props.branch,
      ...metaPayload(),
      frontend_reviewer: frontDraft.value || null,
      backend_reviewer: backDraft.value || null,
      only: picked.value,
    });
    plan.value = p;
    applyPlanMeta(p);
    const made = p.rows.filter((r) => r.created).length;
    const reused = p.rows.filter((r) => r.created === null && r.existing).length;
    msg.success(`已创建 ${made} 个，复用 ${reused} 个`);
    showPicker.value = false; // 提完了，收起选择区
    emit("changed");
  } catch (e) {
    msg.error(api.errText(e));
  } finally {
    creating.value = false;
  }
}

/** GitLab 的 MR 状态 → 中文。空值给「—」而不是空白。 */
function mrState(mr: MergeRequest | null): { text: string; type: "default" | "success" | "warning" | "error" | "info" } {
  if (!mr) return { text: "—", type: "default" };
  switch (mr.state) {
    case "opened":
      return { text: "打开中", type: "info" };
    case "merged":
      return { text: "已合并", type: "success" };
    case "closed":
      return { text: "已关闭", type: "default" };
    case "locked":
      return { text: "已锁定", type: "warning" };
    default:
      return { text: mr.state || "—", type: "default" };
  }
}

/** 该行最终可用的 MR（新建的优先）。 */
function mrOf(r: MrRow): MergeRequest | null {
  return r.created ?? r.existing ?? null;
}

function copyUrl(r: MrRow) {
  const mr = mrOf(r);
  if (!mr?.web_url) {
    msg.warning("这一行还没有 MR 地址");
    return;
  }
  void copy(mr.web_url, `${r.module} 的 MR 地址`);
}

async function closeAll() {
  dialog.warning({
    title: "关闭本需求的 MR",
    content: "只关闭 Merge Request，**不会删除源分支**。确认关闭？",
    positiveText: "关闭",
    negativeText: "取消",
    onPositiveClick: async () => {
      try {
        const out = await api.mrClose(props.branch);
        dialog.info({
          title: "关闭结果",
          content: () => fmt.closeLines(out).join("\n"),
          positiveText: "知道了",
        });
        emit("changed");
      } catch (e) {
        msg.error(api.errText(e));
      }
    },
  });
}

const manifest = ref("");
function copyManifest() {
  if (!plan.value) return;
  const text = fmt.mrManifest(plan.value);
  void copy(text, "MR 清单");
  
  manifest.value = text;
}
</script>

<template>
  <div>
    <NAlert v-if="gl && !gl.user" type="warning" :bordered="false" class="mb">
      尚未连接 GitLab：{{ gl.error }}
      <div class="muted small">到「设置」页填入 Personal Access Token（需要 api 权限）。</div>
    </NAlert>

    <template v-else>
      <div class="toolbar">
        <span class="label">后端评审人</span>
        <NSelect
          v-model:value="backDraft"
          filterable
          remote
          clearable
          :loading="searching"
          :options="userOptions"
          placeholder="输入姓名或 username 搜索"
          class="sel"
          @search="searchUsers"
          @update:value="() => onReviewerChange('back')"
        />
        <span class="label">前端评审人</span>
        <NSelect
          v-model:value="frontDraft"
          filterable
          remote
          clearable
          :loading="searching"
          :options="userOptions"
          placeholder="输入姓名或 username 搜索"
          class="sel"
          @search="searchUsers"
          @update:value="() => onReviewerChange('front')"
        />
        <div class="spacer" />
        <NButton size="small" :disabled="creating" @click="preview">
          {{ plan ? "重新预览" : "预览" }}
        </NButton>
        <NButton size="small" type="primary" :loading="creating" @click="create">创建 MR</NButton>
        <NButton size="small" @click="closeAll">关闭 MR</NButton>
      </div>

      <p v-if="reviewers" class="muted small">
        评审人来源：{{ reviewers.source }}。改动会记住到本需求（下次打开就是这两个人）。
      </p>

      <!-- 标题/描述**可编辑**（实测反馈）：默认由后端给，改动原样提交 -->
      <div class="mr-meta">
        <div class="meta-row">
          <span class="label">标题</span>
          <NInput
            v-model:value="title"
            size="small"
            :input-props="NO_AUTO"
            placeholder="留空则用默认 需求上线-需求名"
            @update:value="titleEdited = true"
          />
        </div>
        <div class="meta-row">
          <span class="label">描述</span>
          <NInput
            v-model:value="description"
            type="textarea"
            size="small"
            :autosize="{ minRows: 3, maxRows: 8 }"
            :input-props="NO_AUTO"
            placeholder="留空则用默认描述"
            @update:value="descriptionEdited = true"
          />
        </div>
        <p class="muted small meta-hint">
          标题按你写的原样提交（默认 <code>需求上线-需求名</code>）；
          描述所有模块共用，模块名在正文里逐条列。
          <span v-if="loadingDefaults">正在读取默认值…</span>
        </p>
      </div>

      <!-- 选要提哪些模块（默认全部）。有些模块已经提过、有些这次不想动。 -->
      <div class="mods">
        <div class="mods-head">
          <NButton size="tiny" secondary @click="showPicker = !showPicker">
            {{ showPicker ? "收起" : "选择模块" }}
          </NButton>
          <span class="muted small">
            本次提 {{ picked.length }} / {{ names.length }} 个模块（创建 MR 时就按这个范围）
          </span>
        </div>
        <ModulePicker
          v-if="showPicker"
          v-model="picked"
          :modules="props.modules"
          class="mt"
        />
      </div>

      <p v-if="!plan" class="muted small">
        还没有预览。点右侧「预览」去 GitLab 查一遍（要联网，可能要几秒）——
        打开这一页时不会自动去查，点「预览」才会。
      </p>

      <table v-if="plan" class="tbl">
        <thead>
          <tr>
            <th>模块</th>
            <th>目标分支</th>
            <th class="num">改动</th>
            <th>评审人</th>
            <th>创建状态</th>
            <th>MR 状态</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="r in plan.rows" :key="r.module">
            <td class="mono">{{ r.module }}</td>
            <td class="mono">{{ r.target_branch }}</td>
            <td class="num">{{ r.files }} 文件</td>
            <td>{{ r.side === "Backend" ? "后端" : "前端" }} · {{ r.reviewer_name }}</td>
            <td>
              <NTag
                size="tiny"
                :bordered="false"
                :type="r.error ? 'error' : r.created ? 'success' : r.existing ? 'info' : 'default'"
              >
                {{ r.error ? "失败" : r.created ? "已创建" : r.existing ? "已存在" : "待创建" }}
              </NTag>
              <span v-if="r.error" class="err-text small"> {{ r.error }}</span>
            </td>
            <!-- MR 状态取自 GitLab（opened / merged / closed） -->
            <td>
              <NTag size="tiny" :bordered="false" :type="mrState(mrOf(r)).type">
                {{ mrState(mrOf(r)).text }}
              </NTag>
            </td>
            <td class="acts">
              <div class="btns">
                <NButton
                  v-if="mrOf(r)"
                  size="tiny"
                  secondary
                  type="primary"
                  @click="openUrl(mrOf(r)!.web_url)"
                >
                  打开
                </NButton>
                <!-- 复制 MR 地址：要贴到群里 @ 评审人，比"打开再复制地址栏"顺手 -->
                <NButton v-if="mrOf(r)" size="tiny" secondary @click="copyUrl(r)">
                  复制地址
                </NButton>
                <span v-if="!mrOf(r)" class="muted small">—</span>
              </div>
            </td>
          </tr>
        </tbody>
      </table>

      <div v-if="plan?.skipped.length" class="skipped">
        <div class="label">跳过</div>
        <div v-for="[m, why] in plan.skipped" :key="m" class="muted small">- {{ m }}: {{ why }}</div>
      </div>

      <NCard v-if="plan && plan.rows.some((r) => r.created || r.existing)" size="small" class="mt">
        <template #header>可复制给评审人的清单</template>
        <template #header-extra>
          <NButton size="tiny" @click="copyManifest">复制清单</NButton>
        </template>
        <pre class="manifest">{{ manifest || fmt.mrManifest(plan) }}</pre>
      </NCard>
    </template>
  </div>
</template>

<style scoped>
.mb {
  margin-bottom: 12px;
}
.mt {
  margin-top: 12px;
}
.toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  margin-bottom: 8px;
}
.label {
  font-size: 12px;
  color: #888;
}
.sel {
  width: 200px;
}
.spacer {
  flex: 1;
}
.tbl {
  width: 100%;
  border-collapse: collapse;
  font-size: 13px;
  margin-top: 8px;
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
.acts {
  white-space: nowrap;
}
.btns {
  display: inline-flex;
  gap: 6px;
  align-items: center;
}
.mr-meta {
  margin: 10px 0 4px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.meta-row {
  display: flex;
  align-items: flex-start;
  gap: 8px;
}
.meta-row > .label {
  min-width: 3em;
  padding-top: 5px;
}
.meta-hint {
  margin: 0;
}
.mods {
  margin: 10px 0 4px;
}
.mods-head {
  display: flex;
  align-items: center;
  gap: 10px;
}
.skipped {
  margin-top: 12px;
}
.muted {
  color: #888;
}
.small {
  font-size: 12px;
}
.err-text {
  color: #d03050;
}
.manifest {
  margin: 0;
  font-family: var(--mono);
  font-size: 12px;
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 320px;
  overflow: auto;
}
</style>