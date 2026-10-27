<script setup lang="ts">
//! 通用操作弹窗：选模块范围 → 执行 → 展示结果。
//!
//! 所有模块级操作都走这里，保证交互一致，也保证**每个操作都能选模块**
//! （设计方案 §6.4：有时候只需要更新某几个项目，不需要全部）。

import { ref, watch } from "vue";
import { NAlert, NButton, NCheckbox, NModal, NSelect, useMessage } from "naive-ui";

import ModulePicker from "./ModulePicker.vue";
import type { EnvRow, ModuleStatus } from "../types";

export interface OpPayload {
  modules: string[];
  env: string | null;
  push: boolean;
  force: boolean;
  rebase: boolean;
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
  action: (p: OpPayload) => Promise<string[]>;
}>();

const emit = defineEmits<{ (e: "close"): void; (e: "done"): void }>();
const msg = useMessage();

const selected = ref<string[]>([]);
const env = ref<string | null>(null);
const push = ref(props.pushByDefault ?? false);
const force = ref(false);
const rebase = ref(false);
const running = ref(false);
/** 执行结果（每行一条）。`null` = 还没执行。 */
const result = ref<string[] | null>(null);

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
    result.value = await props.action({
      modules: selected.value,
      env: env.value,
      push: push.value,
      force: force.value,
      rebase: rebase.value,
    });
  } catch (e) {
    // 失败也停下来展示，而不是弹个 toast 就没了——用户需要看到完整原因
    result.value = [`✗ ${String((e as { message?: string })?.message ?? e)}`];
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
      <div v-if="withPush || withForce || withRebase" class="opts">
        <NCheckbox v-if="withPush" v-model:checked="push">
          合并后直接推送到远程（远程上就能看到）
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
    </template>

    <div v-if="result" class="result">
      <div v-for="(l, i) in result" :key="i">{{ l }}</div>
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
</style>