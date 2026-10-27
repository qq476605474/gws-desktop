<script setup lang="ts">
import { NO_AUTO } from "../lib/inputs";
//! 模块多选框（设计方案 §6.4）。
//!
//! 所有模块级操作（push / pull / merge / 同步 / 提 MR）都用它选范围。
//!
//! **默认全选**，而不是「自动只选有改动的模块」：对 merge/pull 这类操作，
//! 「没改动」也可能是需要处理的（只落后远程要 pull），自动过滤会漏。
//! 界面负责把每个模块的状态标出来让人自己判断。

import { computed, ref } from "vue";
import { NButton, NCheckbox, NInput, NTag } from "naive-ui";

import type { ModuleStatus } from "../types";

const props = withDefaults(
  defineProps<{
    modules: ModuleStatus[];
    /** 选中集合。空数组表示「一个都不选」，不是「全选」。 */
    modelValue: string[];
    /** 记住上次选择用的 key（按操作类型区分）。 */
    rememberKey?: string;
    /**
     * 自定义每行右侧的说明。缺省按改动/推送状态算——
     * 但同步场景关心的是「主干是哪条」，那跟改动无关。
     */
    hints?: Record<string, string>;
  }>(),
  { rememberKey: "" },
);

const emit = defineEmits<{ (e: "update:modelValue", v: string[]): void }>();

const q = ref("");

/** 每行右侧的状态摘要——**这是让人能判断该不该勾的关键**。 */
function summary(m: ModuleStatus): { text: string; type: "default" | "success" | "warning" | "error" } {
  if (props.hints && props.hints[m.module]) {
    return { text: props.hints[m.module], type: "default" };
  }
  if (m.error) return { text: "读不到状态", type: "error" };
  if (!m.present) return { text: "目录缺失", type: "warning" };
  const bits: string[] = [];
  if (m.dirty > 0) bits.push(`${m.dirty} 个改动`);
  if (!m.pushed) bits.push("未推送");
  else if (m.ahead > 0 || m.behind > 0) bits.push(`↑${m.ahead} ↓${m.behind}`);
  if (m.ahead_of_main != null && m.ahead_of_main > 0) bits.push(`领先主干 ${m.ahead_of_main}`);
  if (!bits.length) return { text: "干净", type: "success" };
  return { text: bits.join(" · "), type: m.dirty > 0 || !m.pushed ? "warning" : "default" };
}

const shown = computed(() =>
  props.modules.filter((m) => !q.value.trim() || m.module.includes(q.value.trim())),
);

const selectedSet = computed(() => new Set(props.modelValue));

function toggle(name: string, on: boolean) {
  const next = new Set(props.modelValue);
  if (on) next.add(name);
  else next.delete(name);
  emit("update:modelValue", [...next]);
}

function selectAll() {
  emit("update:modelValue", props.modules.map((m) => m.module));
}

function selectNone() {
  emit("update:modelValue", []);
}

/** 反选只作用于**当前筛选出来的**行——按筛选取反才符合直觉。 */
function invert() {
  const next = new Set(props.modelValue);
  for (const m of shown.value) {
    if (next.has(m.module)) next.delete(m.module);
    else next.add(m.module);
  }
  emit("update:modelValue", [...next]);
}
</script>

<template>
  <div class="picker">
    <!-- 筛选框单独一行，「全选/全不选/反选」另起一行。
         实测反馈：挤在一行时输入框被压得很窄，而且「全选」紧挨着输入框，
         手一抖就误点——这两个是不同性质的操作，不该贴在一起。 -->
    <div class="bar">
      <NInput v-model:value="q" size="small" clearable placeholder="筛选模块…" class="search"  :input-props="NO_AUTO"/>
    </div>

    <div class="bar bar-actions">
      <NButton size="tiny" @click="selectAll">全选</NButton>
      <NButton size="tiny" @click="selectNone">全不选</NButton>
      <NButton size="tiny" @click="invert">反选</NButton>
      <span class="count">已选 {{ modelValue.length }} / {{ modules.length }}</span>
    </div>

    <div class="list">
      <label v-for="m in shown" :key="m.module" class="row">
        <NCheckbox
          :checked="selectedSet.has(m.module)"
          @update:checked="(c: boolean) => toggle(m.module, c)"
        />
        <span class="name mono">{{ m.module }}</span>
        <NTag size="tiny" :bordered="false" :type="summary(m).type">{{ summary(m).text }}</NTag>
      </label>
      <p v-if="!shown.length" class="muted">没有匹配的模块</p>
    </div>
  </div>
</template>

<style scoped>
.picker {
  border: 1px solid var(--xg-border);
  border-radius: 6px;
  overflow: hidden;
}
.bar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 8px;
}
/* 「全选 / 全不选 / 反选」自己一行，与筛选框之间有一条分隔线 */
.bar-actions {
  padding-top: 0;
  border-bottom: 1px solid var(--xg-border);
}
.search {
  flex: 1;
}
.count {
  margin-left: auto;
  color: #888;
  font-size: 12px;
  white-space: nowrap;
}
.list {
  max-height: 220px;
  overflow: auto;
  padding: 4px 0;
}
.row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 3px 10px;
  cursor: pointer;
  font-size: 13px;
}
.row:hover {
  background: var(--n-merged-color-hover, rgba(128, 128, 128, 0.06));
}
.name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.muted {
  color: #888;
  font-size: 12px;
  padding: 4px 10px;
}
</style>