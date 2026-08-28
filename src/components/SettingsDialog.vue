<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useSettingsStore } from "../stores/settings";
import { terminalOptions, type TerminalOption } from "../lib/gws-bridge";

const emit = defineEmits<{ (e: "close"): void }>();
const settings = useSettingsStore();
/** 终端候选由 Rust 按当前 OS + 实际安装情况给出（加载前只留 system 项防空白） */
const terms = ref<TerminalOption[]>([{ id: "system", label: "跟随系统" }]);

onMounted(async () => {
  try {
    terms.value = await terminalOptions();
    // 跨平台迁移防御：存量偏好不在当前 OS 的候选里（如 mac 上存的 iTerm2 带到 Linux）
    // 时回退 system，否则 select 显示空白且旧值永不可达
    if (!terms.value.some((t) => t.id === settings.terminal)) settings.terminal = "system";
  } catch {
    // IPC 失败（极罕见）：保留 system 项，select 仍可用
  }
});
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
        打开终端
        <select v-model="settings.terminal">
          <option v-for="t in terms" :key="t.id" :value="t.id">{{ t.label }}</option>
        </select>
      </label>
      <div class="actions"><button @click="emit('close')">完成</button></div>
    </div>
  </div>
</template>

<style scoped>
.mask { position: fixed; inset: 0; background: var(--mask); display: flex; align-items: center; justify-content: center; z-index: 100; }
.dialog { background: var(--bg-soft); color: var(--fg); border: 1px solid var(--border); box-shadow: var(--shadow); border-radius: 8px; padding: 24px; width: 420px; }
/* 表单两列对齐（表格样）：标签列固定 5em；控件去全局 320px 上限并锁定 32px 高 */
label { display: grid; grid-template-columns: 5em 1fr; gap: 8px; align-items: center; margin-bottom: 12px; font-size: 13px; }
select { max-width: none; height: var(--control-h); }
.actions { display: flex; justify-content: flex-end; margin-top: 12px; }
</style>
