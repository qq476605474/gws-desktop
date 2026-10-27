<script setup lang="ts">
//! 危险操作确认：**先看影响面，手输名称，才允许执行**。
//!
//! # 为什么不用「确定 / 取消」两按钮
//!
//! 撤销改动、强制回退、删远程分支、改分支名——这四个都能让人丢一天工作。
//! 卡片式二次确认防不住连点：用户对弹窗会形成肌肉记忆，一路点「确定」。
//!
//! 所以确认动作设计成**手敲一遍分支名**。这不是为了刁难，而是强制一次停顿
//! ——敲的时候人一定会看一眼「我要操作的到底是哪个需求」。

import { onMounted, ref } from "vue";
import { NAlert, NButton, NInput, NModal } from "naive-ui";

import { NO_AUTO } from "../lib/inputs";
import ActionResult from "./ActionResult.vue";

const props = defineProps<{
  title: string;
  /** 一句话说清这个操作会做什么、丢什么。 */
  hint: string;
  /** 需要手敲确认的内容（一般是分支名）。 */
  confirmText: string;
  /** 首行提示（如「请手输分支名 feature-x 以确认」）。 */
  confirmLabel?: string;
  /** 预览（只读，不改任何东西）。返回给人看的行。 */
  preview: () => Promise<string[]>;
  /**
   * 参数还没填好时，拦住「继续」。
   *
   * 「改分支名」要用：新分支名还是空的（或影响面正在重算）时点「继续」，
   * 看到的是一份跟当前输入对不上的预览。
   */
  disableContinue?: boolean;
  /** 真正执行。返回结果行。 */
  apply: () => Promise<string[]>;
}>();

const emit = defineEmits<{ (e: "close"): void; (e: "done"): void }>();

/** `preview` → 已看影响面，等确认 → `result` 已执行。 */
const phase = ref<"preview" | "confirm" | "result">("preview");
const previewLines = ref<string[]>([]);
const resultLines = ref<string[]>([]);
const typed = ref("");
const running = ref(false);
const loadingPreview = ref(true);

/** 重新计算预览。有些操作的参数（新分支名）会变，预览要跟着变。 */
async function refreshPreview() {
  loadingPreview.value = true;
  try {
    previewLines.value = await props.preview();
  } catch (e) {
    previewLines.value = [`✗ ${String((e as { message?: string })?.message ?? e)}`];
  } finally {
    loadingPreview.value = false;
  }
}

onMounted(refreshPreview);

defineExpose({ refreshPreview });

const matches = () => typed.value.trim() === props.confirmText;

async function apply() {
  if (!matches() || running.value) return;
  running.value = true;
  try {
    resultLines.value = await props.apply();
    phase.value = "result";
    emit("done");
  } catch (e) {
    resultLines.value = [`✗ ${String((e as { message?: string })?.message ?? e)}`];
    phase.value = "result";
  } finally {
    running.value = false;
  }
}
</script>

<template>
  <NModal
    :show="true"
    @update:show="(v: boolean) => !v && !running && emit('close')"
    preset="card"
    :title="title"
    style="width: 700px"
    :mask-closable="!running"
  >
    <NAlert type="warning" :bordered="false" class="hint">{{ hint }}</NAlert>

    <!-- 有些操作需要先填参数（如新的分支名）才有意义预览 -->
    <div v-if="$slots.extra" class="section">
      <slot name="extra" />
    </div>

    <div class="section">
      <div class="sec-title">将发生什么</div>
      <ActionResult
        v-if="!loadingPreview"
        :lines="previewLines"
        title="影响面（只读预览，还没执行）"
      />
      <p v-else class="muted small">正在计算影响面…</p>
    </div>

    <div v-if="phase === 'confirm'" class="section">
      <div class="sec-title">
        {{ confirmLabel ?? `请手输 ${confirmText} 以确认` }}
      </div>
      <NInput
        v-model:value="typed"
        :input-props="NO_AUTO"
        :placeholder="confirmText"
        @keyup.enter="apply"
      />
      <p class="muted small">
        手输是为了强制一次停顿 —— 敲的时候人会看一眼「我操作的到底是哪个需求」。
        点「确定」是防不住连点的。
      </p>
    </div>

    <ActionResult v-if="phase === 'result'" class="mt" :lines="resultLines" title="执行结果" />

    <template #footer>
      <div class="footer">
        <NButton :disabled="running" @click="emit('close')">
          {{ phase === "result" ? "关闭" : "取消" }}
        </NButton>
        <NButton
          v-if="phase === 'preview'"
          type="error"
          :disabled="loadingPreview || disableContinue"
          @click="phase = 'confirm'"
        >
          继续
        </NButton>
        <NButton
          v-else-if="phase === 'confirm'"
          type="error"
          :loading="running"
          :disabled="!matches()"
          @click="apply"
        >
          执行
        </NButton>
      </div>
    </template>
  </NModal>
</template>

<style scoped>
.hint {
  margin-bottom: 14px;
  font-size: 12px;
}
.section {
  margin-bottom: 14px;
}
.sec-title {
  font-size: 12px;
  color: #888;
  margin-bottom: 6px;
}
.mt {
  margin-top: 12px;
}
.muted {
  color: #888;
  font-size: 12px;
  margin: 6px 0 0;
}
.small {
  font-size: 12px;
}
.footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>