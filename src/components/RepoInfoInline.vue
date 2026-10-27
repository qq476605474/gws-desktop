<script setup lang="ts">
//! 把某个仓库的**自定义信息**（链接 + 备注）直接显示在行内。
//!
//! 只显示，**不编辑** —— 编辑入口统一在「仓库」页那一行（实测反馈的明确要求）。
//!
//! 为什么要在需求/环境里也显示：在仓库上配的 Jenkins 地址，合完代码往往马上就要
//! 去发布；信息就在这里的话，点一下就能跳到对应页面，不用回仓库页找。
//!
//! 抽成组件是因为**三处在用**（仓库列表、环境表、需求详情的模块表），
//! 各写一遍迟早会各自漂。

import { NButton } from "naive-ui";

import { useCopy } from "../lib/copy";
import LinkList from "./LinkList.vue";
import type { Link } from "../types";

defineProps<{
  links: Link[];
  notes: string;
}>();

const copy = useCopy();
</script>

<template>
  <div v-if="links.length || notes" class="rinfo">
    <LinkList :links="links" />
    <div v-if="notes" class="rinfo-notes">
      <span class="txt" :title="notes">{{ notes }}</span>
      <NButton size="tiny" quaternary @click="copy(notes, '备注')">复制</NButton>
    </div>
  </div>
</template>

<style scoped>
/* 挂在仓库名下面，不占额外一列 —— 它是补充说明，不该把表格撑宽 */
.rinfo {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin-top: 4px;
  font-weight: 400;
}
.rinfo-notes {
  display: flex;
  align-items: baseline;
  gap: 4px;
}
.rinfo-notes .txt {
  color: #888;
  font-size: 12px;
  white-space: pre-wrap;
  /* 长备注最多两行，别把行高撑爆；全文在 title 里 */
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
</style>
