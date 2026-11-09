<script setup lang="ts">
//! 通用操作弹窗：选模块范围 → 执行 → 展示结果。
//!
//! 所有模块级操作都走这里，保证交互一致，也保证**每个操作都能选模块**
//! （设计方案 §6.4：有时候只需要更新某几个项目，不需要全部）。

import { computed, ref, watch } from "vue";
import {
  NAlert,
  NButton,
  NCheckbox,
  NInput,
  NModal,
  NRadioButton,
  NRadioGroup,
  NSelect,
  useMessage,
} from "naive-ui";

import ModulePicker from "./ModulePicker.vue";
import { NO_AUTO } from "../lib/inputs";
import { buildCommitMessage, COMMIT_TYPES } from "../lib/conventional";
import EditorButton from "./EditorButton.vue";
import type { EnvRow, ModuleStatus } from "../types";

export interface OpPayload {
  modules: string[];
  env: string | null;
  push: boolean;
  force: boolean;
  rebase: boolean;
  /** 提交说明（「提交」操作用；留空由后端按分支名生成）。 */
  message: string;
}

/** 结果里附带的一个可打开目录（合并到环境后要 review / 解冲突）。 */
export interface OpDir {
  path: string;
  label: string;
  /** 一句状态说明，如「冲突待处理」。 */
  note?: string;
}

/**
 * 执行结果。除了文本行，还可以带一组目录 ——
 * 原来合并出冲突只打印路径，还得人自己切到「环境」页去找（实测反馈）。
 */
export interface OpResult {
  lines: string[];
  dirs?: OpDir[];
}

const props = defineProps<{
  title: string;
  /** 一句话说明这个操作会做什么。危险操作必须说清影响面。 */
  hint?: string;
  modules: ModuleStatus[];
  /** 需要选环境时传入（merge）。 */
  envs?: EnvRow[];
  /** 额外提供「合并后推送」勾选（merge）。 */
  withPush?: boolean;
  /**
   * 「合并后推送」**默认就勾上**。
   *
   * 用在「推送到环境」那个入口：它与「合并到环境」**是同一个操作**，
   * 只是期望的结果不同（一个只合到本地、一个直接推上去），所以弹窗一样、
   * 只是默认值不同 —— 免得人勾来勾去多点一次（实测反馈）。
   */
  pushByDefault?: boolean;
  /** 额外提供「强制」勾选（merge）。 */
  withForce?: boolean;
  /** 额外提供「用 rebase」勾选（pull）。 */
  withRebase?: boolean;
  /** 额外提供提交说明输入框（commit）。 */
  withCommit?: boolean;
  /** 额外提供「提交后推送」勾选（commit，默认不勾）。 */
  withCommitPush?: boolean;
  /**
   * 预勾选的模块。给了就只勾这些，没给才全选。
   *
   * 用在「已经知道只要动哪几个」的场景：比如提示里说「这 3 个模块落后远程」，
   * 点「去拉取」时就该只勾那 3 个——实测反馈 #2。全选会让人以为每次都要动全部。
   */
  preselect?: string[] | null;
  /**
   * 默认**一个都不勾**（而不是全选）。
   *
   * 用在「增加模块」这类操作上：全选的话一不小心就把所有仓库都加进来了
   * （实测反馈）。这类操作的语义是「我明确要加哪几个」，默认空更安全。
   */
  emptyByDefault?: boolean;
  /**
   * 实际执行。**抽成函数 prop 而不是 emit**：操作与它需要的参数在父组件那边
   * 是天然绑在一起的，用 emit 得把一个 promise 结果再传回来，反而绕。
   */
  action: (p: OpPayload) => Promise<string[] | OpResult>;
}>();

const emit = defineEmits<{ (e: "close"): void; (e: "done"): void }>();
const msg = useMessage();

const selected = ref<string[]>([]);
const env = ref<string | null>(null);
const push = ref(props.pushByDefault ?? false);
const force = ref(false);
const rebase = ref(false);
/** 提交说明。**留空也能提交**：后端按分支名生成默认说明。 */
const message = ref("");

/** 提交说明的填写方式：默认按 Conventional Commits 规范，可切回自由填写。 */
const commitMode = ref<"guided" | "free">("guided");
const type = ref<string>("feat");
const scope = ref("");
const summary = ref("");
const body = ref("");
const breaking = ref(false);
const breakingNote = ref("");

const typeOptions = COMMIT_TYPES.map((t) => ({
  label: t.label,
  value: t.value,
}));

const guidedMessage = computed(() =>
  buildCommitMessage({
    type: type.value,
    scope: scope.value,
    summary: summary.value,
    body: body.value,
    breaking: breaking.value,
    breakingNote: breakingNote.value,
  }),
);

/** 真正传给后端的说明：规范模式拼出来的，或自由填写的原文。 */
const commitMessage = computed(() =>
  commitMode.value === "guided" ? guidedMessage.value : message.value,
);

// 切回自由填写时带出刚拼好的消息，想改哪句就直接改，不用重打一遍。
watch(commitMode, (m) => {
  if (m === "free" && !message.value && guidedMessage.value) {
    message.value = guidedMessage.value;
  }
});

const running = ref(false);
/** 执行结果（每行一条）。`null` = 还没执行。 */
const result = ref<string[] | null>(null);
/** 结果里附带的目录：合完可以直接打开检查或解冲突。 */
const dirs = ref<OpDir[]>([]);

// 默认全选：不做「自动只选有改动的」，理由见 ModulePicker 的注释。
// 但父组件如果明确指出了「就是这几个模块」，那就只勾那几个（反馈 #2）。
watch(
  () => props.modules,
  (m) => {
    if (result.value !== null) return;
    if (props.preselect?.length) {
      selected.value = m.map((x) => x.module).filter((n) => props.preselect!.includes(n));
    } else {
      selected.value = props.emptyByDefault ? [] : m.map((x) => x.module);
    }
  },
  { immediate: true },
);

watch(
  () => props.envs,
  (e) => {
    if (e?.length && !env.value) env.value = e[0].id;
  },
  { immediate: true },
);

async function run() {
  if (running.value) return;
  if (!selected.value.length) {
    msg.warning("至少要选一个模块");
    return;
  }
  running.value = true;
  try {
    const out = await props.action({
      modules: selected.value,
      env: env.value,
      push: push.value,
      force: force.value,
      rebase: rebase.value,
      message: commitMessage.value,
    });
    // 两种返回都接受：老操作只给文本行，合并这类会额外给可打开目录
    if (Array.isArray(out)) {
      result.value = out;
      dirs.value = [];
    } else {
      result.value = out.lines;
      dirs.value = out.dirs ?? [];
    }
  } catch (e) {
    // 失败也停下来展示，而不是弹个 toast 就没了——用户需要看到完整原因
    result.value = [`✗ ${String((e as { message?: string })?.message ?? e)}`];
    dirs.value = [];
  } finally {
    running.value = false;
  }
}
</script>

<template>
  <NModal :show="true" @update:show="(v: boolean) => !v && emit('close')"
    preset="card"
    :title="title"
    style="width: 660px"
    :mask-closable="!running"
    @close="emit('close')"
  >
    <NAlert v-if="hint" type="info" :bordered="false" class="hint">{{ hint }}</NAlert>

    <div v-if="envs" class="env-row">
      <span class="label">环境</span>
      <NSelect
        v-model:value="env"
        :options="envs.map((e) => ({ label: `${e.id}（${e.label}）`, value: e.id }))"
        class="env-select"
      />
    </div>

    <!-- 执行开始后收起模块选择：留着的话同步完还显示着，
         让人以为还能再点一次（实测反馈 #24） -->
    <template v-if="result === null">
      <div class="label block">模块</div>
      <ModulePicker v-model="selected" :modules="modules" />

      <!-- 用 NCheckbox 而不是原生 `<input type="checkbox">`。
           原生那个在 webview 里就是一个约 13px 的小方块：不跟随主题（深色下是个
           刺眼的白块）、点击区只有那个小方块本身、跟界面里其它勾选框（模块列表、
           启动页、新建需求）完全不是一个样子。实测反馈：太小了、不好点。 -->
      <div v-if="withPush || withForce || withRebase || withCommitPush" class="opts">
        <NCheckbox v-if="withPush" v-model:checked="push">
          合并后直接推送到远程（远程上就能看到）
        </NCheckbox>
        <!-- 提交与推送是两步：默认只提交，确认没问题再推送 -->
        <NCheckbox v-if="withCommitPush" v-model:checked="push">
          提交后直接推送到远程（远程上就能看到）
        </NCheckbox>
        <NCheckbox v-if="withForce" v-model:checked="force">
          丢弃本地未推送的合并成果，从远程重新合
        </NCheckbox>
        <NCheckbox v-if="withRebase" v-model:checked="rebase">
          用 rebase 而不是 merge
        </NCheckbox>
        <!-- 代价说明单独一行、退一档：它是注解而不是选项，不该混在可勾选文字里 -->
        <p v-if="withRebase" class="muted small note">
          会改写你的本地提交顺序。只在确定这些提交还没合进过远程分支时用——
          否则远程那边拉取后会重复提交。
        </p>
      </div>

      <!-- 提交说明：默认按 Conventional Commits 填，也可切回自由填写。
           规范不是强制的：描述留空仍然用后端的默认说明。 -->
      <div v-if="withCommit" class="commit-msg">
        <div class="msg-head">
          <span class="label">提交说明</span>
          <span class="spacer" />
          <NRadioGroup v-model:value="commitMode" size="small">
            <NRadioButton value="guided">按规范填写</NRadioButton>
            <NRadioButton value="free">自由填写</NRadioButton>
          </NRadioGroup>
        </div>

        <template v-if="commitMode === 'guided'">
          <div class="cc-row">
            <div class="field">
              <span class="label">类型</span>
              <NSelect v-model:value="type" :options="typeOptions" size="small" />
            </div>
            <div class="field">
              <span class="label">影响范围（可选）</span>
              <!-- 自由输入；批量提交多个仓库时一般不填 -->
              <NInput
                v-model:value="scope"
                size="small"
                :input-props="NO_AUTO"
                placeholder="如：收银台、订单；可留空"
              />
            </div>
          </div>
          <div class="field">
            <span class="label">描述</span>
            <NInput
              v-model:value="summary"
              size="small"
              :input-props="NO_AUTO"
              placeholder="一句话说明改了什么，如：支持分次支付"
            />
          </div>
          <div class="field">
            <span class="label">详细说明（可选，一行一条）</span>
            <NInput
              v-model:value="body"
              type="textarea"
              :autosize="{ minRows: 3, maxRows: 6 }"
              :input-props="NO_AUTO"
              placeholder="第一点改了什么&#10;第二点改了什么&#10;（会自动变成 - 列表）"
            />
          </div>
          <div class="cc-breaking">
            <NCheckbox v-model:checked="breaking">包含不兼容变更（BREAKING CHANGE）</NCheckbox>
            <p v-if="!breaking" class="muted small note">
              指会让已有调用方出错的变化（如接口字段改名 / 删除、参数变化）。不确定就别勾。
            </p>
            <NInput
              v-if="breaking"
              v-model:value="breakingNote"
              size="small"
              :input-props="NO_AUTO"
              placeholder="可选：具体影响（会写进 BREAKING CHANGE footer）"
            />
          </div>
          <div class="cc-preview">
            <span class="label">预览</span>
            <pre v-if="guidedMessage">{{ guidedMessage }}</pre>
            <p v-else class="muted small">
              描述留空就仍用默认说明（chore: 提交 &lt;分支&gt; 的改动）—— 规范填写不强制。
            </p>
          </div>
        </template>

        <NInput
          v-else
          v-model:value="message"
          type="textarea"
          :autosize="{ minRows: 3, maxRows: 8 }"
          :input-props="NO_AUTO"
          placeholder="留空 = chore: 提交 <分支> 的改动；可换行，第一行是标题"
        />
      </div>
    </template>

    <div v-if="result" class="result">
      <div v-for="(l, i) in result" :key="i">{{ l }}</div>
    </div>

    <!-- 结果里给目录的操作（合并到环境）：**不用再去环境页找同一个目录** -->
    <div v-if="dirs.length" class="dirs">
      <div v-for="d in dirs" :key="d.path" class="dir-row">
        <span class="mono small">{{ d.label }}</span>
        <span v-if="d.note" class="dir-note" :class="{ bad: d.note.includes('冲突') }">
          {{ d.note }}
        </span>
        <span class="spacer" />
        <EditorButton size="tiny" :path="d.path" :label="d.note?.includes('冲突') ? '用编辑器解决冲突' : '用编辑器打开'" />
      </div>
    </div>

    <template #footer>
      <div class="footer">
        <NButton :disabled="running" @click="emit('close')">{{ result ? "关闭" : "取消" }}</NButton>
        <NButton v-if="!result" type="primary" :loading="running" @click="run">执行</NButton>
        <NButton
          v-else
          type="primary"
          @click="
            () => {
              emit('done');
              emit('close');
            }
          "
        >
          完成
        </NButton>
      </div>
    </template>
  </NModal>
</template>

<style scoped>
.hint {
  margin-bottom: 12px;
  font-size: 12px;
}
.label {
  font-size: 12px;
  color: #888;
}
.block {
  margin: 12px 0 6px;
}
.env-row {
  display: flex;
  align-items: center;
  gap: 10px;
}
.env-select {
  width: 220px;
}
.opts {
  margin-top: 12px;
  display: flex;
  flex-direction: column;
  /* NCheckbox 的行高比原生 input 高，间距要留够，否则几项挤在一起 */
  gap: 10px;
  font-size: 13px;
}
.opts .note {
  margin: 0;
  /* 缩进到跟勾选文字对齐，一眼看出它是解释而不是另一个选项 */
  padding-left: 26px;
}
.commit-msg {
  margin-top: 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.msg-head {
  display: flex;
  align-items: center;
  gap: 8px;
}
.cc-row {
  display: flex;
  gap: 10px;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  flex: 1;
  min-width: 0;
}
.cc-breaking {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.cc-breaking .note {
  margin: 0;
  padding-left: 26px;
}
.cc-preview pre {
  margin: 4px 0 0;
  padding: 8px 10px;
  border: 1px solid var(--xg-border);
  border-radius: 6px;
  background: var(--xg-fill-soft);
  font-family: var(--mono);
  font-size: 12px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
}
.cc-preview p {
  margin: 4px 0 0;
}
.muted {
  color: #888;
}
.small {
  font-size: 12px;
}
.result {
  margin-top: 4px;
  padding: 10px 12px;
  border-radius: 6px;
  background: rgba(128, 128, 128, 0.08);
  font-family: var(--mono);
  font-size: 12px;
  max-height: 260px;
  overflow: auto;
  white-space: pre-wrap;
  word-break: break-all;
}
.footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
.dirs {
  margin-top: 10px;
  border: 1px solid var(--xg-border);
  border-radius: 6px;
  padding: 6px 10px;
  max-height: 180px;
  overflow: auto;
}
.dir-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 0;
}
.dir-row + .dir-row {
  border-top: 1px solid var(--xg-border);
}
.dir-note {
  font-size: 12px;
  color: #888;
}
.dir-note.bad {
  color: #d03050;
}
.spacer {
  flex: 1;
}
</style>