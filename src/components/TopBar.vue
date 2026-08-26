<script setup lang="ts">
import { useHubStore } from "../stores/hub";
import { navigate } from "../router";

export type Tab = "ws" | "repos" | "envs" | "docs";
defineProps<{ tab: Tab }>();
const emit = defineEmits<{
  (e: "update:tab", t: Tab): void;
  (e: "open-about"): void;
  (e: "open-settings"): void;
}>();
const hub = useHubStore();

function switchHub() {
  navigate("startup");
}
</script>

<template>
  <header class="topbar">
    <span class="brand">GWS Desk</span>
    <button @click="switchHub">{{ hub.path || "选择 Hub" }} ▾</button>
    <nav>
      <button :class="{ active: tab === 'ws' }" @click="emit('update:tab', 'ws')">工作区</button>
      <button :class="{ active: tab === 'repos' }" @click="emit('update:tab', 'repos')">仓库</button>
      <button :class="{ active: tab === 'envs' }" @click="emit('update:tab', 'envs')">环境</button>
      <button :class="{ active: tab === 'docs' }" @click="emit('update:tab', 'docs')">文档</button>
    </nav>
    <span class="spacer" />
    <button @click="emit('open-about')">About gws CLI</button>
    <button @click="emit('open-settings')">设置</button>
  </header>
</template>

<style scoped>
.topbar { display: flex; align-items: center; gap: 12px; padding: 8px 16px; border-bottom: 1px solid var(--border); }
.brand { font-weight: 700; }
.spacer { flex: 1; }
nav { display: flex; gap: 4px; }
.active { background: var(--mono-bg); }
</style>
