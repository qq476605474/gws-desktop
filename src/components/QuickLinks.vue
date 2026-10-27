<script setup lang="ts">
//! 「快捷入口」：按 hub 里**用户自己配的**列表渲染。
//!
//! 它只是一种**可全局配置的快捷方式** —— 指向哪儿由用户决定（发布页、监控、
//! 文档、工单系统都行），所以叫「快捷入口」而不是「发布入口」。
//! 开源版不预设任何地址。列表为空时调用方**整块隐藏**这个组件。
//!
//! 直接跳、不做二次确认：这些是「每天要跳好几次」的页面，多一步就是白费一步。

import { NButton } from "naive-ui";

import { openUrl } from "../bridge";
import type { QuickLink } from "../types";

const props = defineProps<{ links: QuickLink[] }>();

/** 点完一个地址后通知外层：需求详情用它**关掉弹窗**。 */
const emit = defineEmits<{ (e: "pick"): void }>();

function pick(url: string) {
  openUrl(url);
  emit("pick");
}
</script>

<template>
  <div class="pub">
    <div class="pub-btns">
      <NButton
        v-for="(l, i) in props.links"
        :key="l.url"
        size="small"
        :secondary="i !== 0"
        :type="i === 0 ? 'primary' : 'default'"
        :title="l.url"
        @click="pick(l.url)"
      >
        {{ l.label }}
      </NButton>
    </div>
  </div>
</template>

<style scoped>
.pub {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.pub-btns {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
</style>