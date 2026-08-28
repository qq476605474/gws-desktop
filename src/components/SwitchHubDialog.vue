<script setup lang="ts">
import { ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { useHubStore } from "../stores/hub";
import { useSettingsStore } from "../stores/settings";
import { hubExists } from "../lib/gws-bridge";

const emit = defineEmits<{ (e: "close"): void }>();
const hub = useHubStore();
const settings = useSettingsStore();
// 回显当前 hub 目录：既是查看处（顶栏不再占位展示长路径），也方便基于它微调
const hubPath = ref(hub.path);
const checking = ref(false);
const error = ref("");

async function pickHub() {
  try {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir === "string") hubPath.value = dir;
  } catch {
    // 选择器异常（极罕见）按取消处理
  }
}

async function openHub() {
  const p = hubPath.value.trim();
  if (!p || checking.value) return;
  // 同路径 no-op：setHub 会清空列表 store，但 path 不变则 MainView 不重挂载，
  // 页面会停在空列表——直接关窗即可
  if (p === hub.path) {
    emit("close");
    return;
  }
  checking.value = true;
  error.value = "";
  try {
    if (!(await hubExists(p))) {
      error.value = "该目录不是 gws hub（缺少 .gws-hub 标记）";
      return;
    }
    hub.setHub(p);
    settings.lastHub = p; // 下次启动自动进入新 hub
    emit("close"); // hub.path 变化令 App.vue 的 MainView :key 失效整体重挂载
  } catch (e) {
    error.value = String(e);
  } finally {
    checking.value = false;
  }
}
</script>

<template>
  <div class="mask" @click.self="emit('close')">
    <div class="dialog">
      <h3>切换 Hub 目录</h3>
      <label>目录路径 <input v-model="hubPath" autocapitalize="off" autocorrect="off" spellcheck="false" placeholder="hub 目录路径（含 .gws-hub）" /></label>
      <p class="muted">切换后页面状态（如已打开的需求详情）将重置</p>
      <p v-if="error" class="err">{{ error }}</p>
      <div class="actions">
        <button @click="emit('close')">取消</button>
        <button @click="pickHub">浏览…</button>
        <button class="primary" :disabled="!hubPath.trim() || checking" @click="openHub">打开 Hub</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mask { position: fixed; inset: 0; background: var(--mask); display: flex; align-items: center; justify-content: center; z-index: 100; }
/* hub 路径往往很长：弹窗放宽到 560px、输入框单行完整容纳 */
.dialog { background: var(--bg-soft); border: 1px solid var(--border); box-shadow: var(--shadow); border-radius: 8px; padding: 20px; width: 560px; display: flex; flex-direction: column; gap: 8px; }
label { display: grid; grid-template-columns: 5em 1fr; gap: 8px; align-items: center; font-size: 13px; }
input { max-width: none; height: var(--control-h); }
.actions { display: flex; gap: 8px; justify-content: flex-end; }
.muted { color: var(--fg-muted); font-size: 13px; margin: 0; }
.err { color: var(--danger-text); font-size: 13px; margin: 0; }
</style>
