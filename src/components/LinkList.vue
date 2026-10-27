<script setup lang="ts">
//! 自定义链接的**展示**。
//!
//! 两类条目，行为**不同**（这正是它们存在的意义）：
//!
//! - **有地址** → 点了用浏览器打开（需求 #20）。地址放在 tooltip 里，
//!   需要贴给别人时鼠标一放就能看到。**不另给「复制」按钮** —— 点名称
//!   已经能打开浏览器，再放一个复制入口是重复的（实测反馈）。
//! - **没地址（只记一条信息）** → 点了**复制**。实测反馈里的典型用法是
//!   「常见的模块域名，方便在这里拷贝」—— 这种东西要的是拷走，不是打开。
//!   以前它只是个静态标签，看得见、拿不走。

import { NButton } from "naive-ui";

import { openUrl } from "../bridge";
import { useCopy } from "../lib/copy";
import type { Link } from "../types";

defineProps<{ links: Link[] }>();

const copy = useCopy();
</script>

<template>
  <div v-if="links.length" class="wrap">
    <template v-for="(l, i) in links" :key="i">
      <NButton
        v-if="l.url"
        size="tiny"
        secondary
        type="primary"
        class="item"
        :title="`打开 ${l.url}`"
        @click="openUrl(l.url)"
      >
        {{ l.label || l.url }}
      </NButton>
      <!-- 只记一条信息：点一下复制走。加 📋 是为了让人看出它可点 ——
           静态标签的样子等于告诉人「这东西只能看」 -->
      <NButton
        v-else
        size="tiny"
        secondary
        class="item"
        :title="`点击复制：${l.label}`"
        @click="copy(l.label, '内容')"
      >
        📋 {{ l.label }}
      </NButton>
    </template>
  </div>
</template>

<style scoped>
.wrap {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.item {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  max-width: 260px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>