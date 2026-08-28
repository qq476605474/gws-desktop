<script setup lang="ts">
import { ref } from "vue";
import { useHubStore } from "../stores/hub";
import SwitchHubDialog from "./SwitchHubDialog.vue";

export type Tab = "ws" | "repos" | "envs" | "docs";
defineProps<{ tab: Tab }>();
const emit = defineEmits<{
  (e: "update:tab", t: Tab): void;
  (e: "open-about"): void;
  (e: "open-settings"): void;
}>();
const hub = useHubStore();
// hub 目录路径长（顶栏展示不全也占位）：收敛为「切换」按钮，路径在弹窗内
// 回显查看/修改——切换防误触由弹窗「确认后 hub.path 变化才重挂载」自然保证
const showSwitch = ref(false);
</script>

<template>
  <header class="topbar">
    <span class="brand">GwsDesk</span>
    <button class="hub" :title="hub.path || '选择 Hub'" @click="showSwitch = true">切换</button>
    <nav>
      <button :class="{ active: tab === 'ws' }" @click="emit('update:tab', 'ws')">需求</button>
      <button :class="{ active: tab === 'repos' }" @click="emit('update:tab', 'repos')">仓库</button>
      <button :class="{ active: tab === 'envs' }" @click="emit('update:tab', 'envs')">环境</button>
      <button :class="{ active: tab === 'docs' }" @click="emit('update:tab', 'docs')">文档</button>
    </nav>
    <span class="spacer" />
    <button @click="emit('open-about')">当前版本</button>
    <button @click="emit('open-settings')">设置</button>
  </header>
  <SwitchHubDialog v-if="showSwitch" @close="showSwitch = false" />
</template>

<style scoped>
.topbar { display: flex; align-items: center; gap: 12px; padding: 8px 16px; border-bottom: 1px solid var(--border); }
.brand { font-weight: 700; }
.spacer { flex: 1; }
nav { display: flex; gap: 4px; }
/* 显式 fg：不依赖 UA 按钮色（未设 color-scheme 时 dark 主题下黑字压深底仅 1.80:1）。
   light/macos 下 --mono-bg 与默认底色差仅 5-7/255，单靠底色无法区分——加粗补足 */
.active { background: var(--mono-bg); color: var(--fg); font-weight: 600; }
</style>
