<script setup lang="ts">
import { toasts } from "../lib/toast";
</script>

<template>
  <!-- 成功反馈轻提示（如复制路径）。z-index 300：高于 ConfirmDialog(200)——
       confirm 确认后触发的动作反馈不应被遮罩挡住；不挡任何点击（pointer-events: none） -->
  <div class="toasts" aria-live="polite">
    <div v-for="t in toasts" :key="t.id" class="toast">{{ t.message }}</div>
  </div>
</template>

<style scoped>
.toasts {
  position: fixed; bottom: 28px; left: 50%; transform: translateX(-50%);
  display: flex; flex-direction: column; align-items: center; gap: 8px;
  z-index: 300; pointer-events: none;
}
.toast {
  background: var(--bg-soft); color: var(--fg); border: 1px solid var(--border);
  box-shadow: var(--shadow); border-radius: var(--radius);
  padding: 8px 16px; font-size: 13px;
  animation: toast-in 0.15s ease-out;
}
@keyframes toast-in {
  from { opacity: 0; transform: translateY(6px); }
  to { opacity: 1; transform: translateY(0); }
}
</style>
