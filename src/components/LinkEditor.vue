<script setup lang="ts">
//! 自定义链接的**编辑**（需求 #20 / 实测反馈 #5）。
//!
//! 需求级与仓库级都用它，保证两处行为一致。
//!
//! 每行「名称 + 地址」：名称是给人看的（`tapd`、`飞书文档`、`监控`），
//! 地址为空表示**只记一条信息**（比如「负责人：某某」），此时不渲染成链接。

import { NButton, NInput } from "naive-ui";

import { NO_AUTO } from "../lib/inputs";
import type { Link } from "../types";

const props = defineProps<{
  modelValue: Link[];
  /** 名称的示例，让人知道该填什么。 */
  placeholders?: string[];
}>();

const emit = defineEmits<{ (e: "update:modelValue", v: Link[]): void }>();

function update(next: Link[]) {
  emit("update:modelValue", next);
}

function setField(i: number, key: keyof Link, value: string) {
  const next = props.modelValue.map((l, idx) => (idx === i ? { ...l, [key]: value } : l));
  update(next);
}

function add() {
  update([...props.modelValue, { label: "", url: "" }]);
}

function remove(i: number) {
  update(props.modelValue.filter((_, idx) => idx !== i));
}

function move(i: number, delta: number) {
  const next = [...props.modelValue];
  const j = i + delta;
  if (j < 0 || j >= next.length) return;
  [next[i], next[j]] = [next[j], next[i]];
  update(next);
}
</script>

<template>
  <div class="links">
    <div v-for="(l, i) in modelValue" :key="i" class="row">
      <NInput
        :value="l.label"
        size="small"
        :placeholder="placeholders?.[0] ?? '名称，如 tapd'"
        class="lab"
        :input-props="NO_AUTO"
        @update:value="(v: string) => setField(i, 'label', v)"
      />
      <NInput
        :value="l.url"
        size="small"
        :placeholder="placeholders?.[1] ?? '地址（可留空，只记一条信息）'"
        :input-props="NO_AUTO"
        @update:value="(v: string) => setField(i, 'url', v)"
      />
      <NButton size="tiny" :disabled="i === 0" @click="move(i, -1)">↑</NButton>
      <NButton size="tiny" :disabled="i === modelValue.length - 1" @click="move(i, 1)">↓</NButton>
      <NButton size="tiny" type="error" @click="remove(i)">删</NButton>
    </div>
    <div class="row">
      <NButton size="tiny" @click="add">＋ 加一条</NButton>
      <span class="muted small">
        顺序就是显示顺序。地址留空 = 只记一条信息（不渲染成可点链接）。
      </span>
    </div>
  </div>
</template>

<style scoped>
.links {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.row {
  display: flex;
  align-items: center;
  gap: 6px;
}
.lab {
  width: 150px;
  flex: none;
}
.muted {
  color: #888;
}
.small {
  font-size: 12px;
}
</style>