<script setup lang="ts">
import { onMounted, ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { navigate } from "../router";
import { useSettingsStore } from "../stores/settings";
import { useHubStore } from "../stores/hub";
import { checkGwsInstalled, hubExists } from "../lib/gws-bridge";

const settings = useSettingsStore();
const hub = useHubStore();
const installed = ref(true);
const hubPath = ref("");
const error = ref("");

async function pickHub() {
  const dir = await open({ directory: true, multiple: false });
  if (typeof dir === "string") hubPath.value = dir;
}
async function openHub() {
  if (!hubPath.value) return;
  if (!(await hubExists(hubPath.value))) {
    error.value = "该目录不是 gws hub（缺少 .gws-hub 标记）";
    return;
  }
  hub.setHub(hubPath.value);
  settings.lastHub = hubPath.value;
  navigate("main");
}
onMounted(async () => {
  installed.value = await checkGwsInstalled();
  if (installed.value && settings.lastHub) {
    hubPath.value = settings.lastHub;
    await openHub(); // 上次 hub 存在且 gws 可用则直接进
  }
});
</script>

<template>
  <main class="startup">
    <h1>GWS Desk</h1>
    <p v-if="!installed" class="warn">
      未检测到 gws 命令。安装：
      <code>curl -fsSL https://raw.githubusercontent.com/qq476605474/gws/main/gws -o ~/.local/bin/gws && chmod +x ~/.local/bin/gws</code>
    </p>
    <div class="pick">
      <input v-model="hubPath" placeholder="hub 目录路径（含 .gws-hub）" />
      <button @click="pickHub">浏览…</button>
      <button :disabled="!hubPath" @click="openHub">打开 Hub</button>
    </div>
    <p v-if="error" class="warn">{{ error }}</p>
  </main>
</template>

<style scoped>
.startup { display: flex; flex-direction: column; align-items: center; justify-content: center; height: 100vh; gap: 16px; }
.warn { color: var(--danger-text); max-width: 480px; }
.pick { display: flex; gap: 8px; }
</style>
