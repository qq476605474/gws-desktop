<script setup lang="ts">
import { useSettingsStore } from "../stores/settings";

const emit = defineEmits<{ (e: "close"): void }>();
const settings = useSettingsStore();
</script>

<template>
  <div class="mask" @click.self="emit('close')">
    <div class="dialog">
      <h3>设置</h3>
      <label>
        外观主题
        <select v-model="settings.theme">
          <option value="light">浅色极简（默认）</option>
          <option value="dark">深色开发者</option>
          <option value="macos">macOS 原生</option>
        </select>
      </label>
      <label>
        在终端中打开（默认跟随系统）
        <select v-model="settings.terminal">
          <option value="system">跟随系统（有 iTerm2 用 iTerm2）</option>
          <option value="iTerm2">iTerm2</option>
          <option value="Terminal.app">Terminal.app</option>
          <option value="Warp">Warp</option>
        </select>
      </label>
      <div class="actions"><button @click="emit('close')">完成</button></div>
    </div>
  </div>
</template>

<style scoped>
.mask { position: fixed; inset: 0; background: rgba(0, 0, 0, .35); display: flex; align-items: center; justify-content: center; z-index: 100; }
.dialog { background: var(--bg-soft); color: var(--fg); border: 1px solid var(--border); border-radius: 8px; padding: 24px; width: 420px; }
label { display: flex; flex-direction: column; gap: 6px; margin-bottom: 12px; }
.actions { display: flex; justify-content: flex-end; margin-top: 12px; }
</style>
