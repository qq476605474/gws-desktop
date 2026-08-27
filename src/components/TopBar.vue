<script setup lang="ts">
import { confirm } from "@tauri-apps/plugin-dialog";
import { useHubStore } from "../stores/hub";
import { useSettingsStore } from "../stores/settings";
import { navigate } from "../router";

export type Tab = "ws" | "repos" | "envs" | "docs";
defineProps<{ tab: Tab }>();
const emit = defineEmits<{
  (e: "update:tab", t: Tab): void;
  (e: "open-about"): void;
  (e: "open-settings"): void;
}>();
const hub = useHubStore();
const settings = useSettingsStore();

// 防误触：hub 按钮点击不是切 tab 而是离开整个 main，
// WorkspacesTab 将重挂载、已开详情与滚动全丢——先原生确认，确认后才跳转
async function switchHub() {
  let ok = false;
  try {
    ok = await confirm("要切换/重新选择 hub 目录吗？当前页面状态（如已打开的工作区详情）将丢失。");
  } catch {
    return; // 理论上不 reject；万一异常按取消处理
  }
  if (!ok) return;
  // 清 lastHub：否则 StartupView 挂载即读 lastHub 自动回跳 main，用户根本没机会选新 hub
  settings.lastHub = "";
  navigate("startup");
}
</script>

<template>
  <header class="topbar">
    <span class="brand">GwsDesk</span>
    <button @click="switchHub">{{ hub.path || "选择 Hub" }} ▾</button>
    <nav>
      <button :class="{ active: tab === 'ws' }" @click="emit('update:tab', 'ws')">工作区</button>
      <button :class="{ active: tab === 'repos' }" @click="emit('update:tab', 'repos')">仓库</button>
      <button :class="{ active: tab === 'envs' }" @click="emit('update:tab', 'envs')">环境</button>
      <button :class="{ active: tab === 'docs' }" @click="emit('update:tab', 'docs')">文档</button>
    </nav>
    <span class="spacer" />
    <button @click="emit('open-about')">当前版本</button>
    <button @click="emit('open-settings')">设置</button>
  </header>
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
