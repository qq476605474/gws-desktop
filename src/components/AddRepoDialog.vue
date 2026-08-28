<script setup lang="ts">
import { ref } from "vue";
import { useHubStore } from "../stores/hub";
import { useCmdStore } from "../stores/cmd";

const emit = defineEmits<{ (e: "close"): void }>();
const hub = useHubStore();
const cmd = useCmdStore();
const input = ref("");
const err = ref("");
const submitting = ref(false);

async function add() {
  if (submitting.value) return; // 防双击：exec 的 IPC 往返间隙 isRunning 尚未生效
  const urls = input.value.split(/[\s,]+/).filter(Boolean);
  if (!urls.length) return;
  err.value = "";
  submitting.value = true;
  try {
    const run = await cmd.execDialog("gws repo add", ["repo", "add", ...urls], hub.path);
    // 等命令终态再收尾（waitDone 模式）：失败不关窗，输入保留便于重试
    await cmd.waitDone(run);
    if (run.state !== "done") return;
    emit("close");
  } catch (e) {
    // exec/waitDone reject（如 IPC 失败）：内联提示，弹窗不关闭
    err.value = String(e);
  } finally {
    submitting.value = false;
  }
}
</script>

<template>
  <div class="mask" @click.self="!submitting && emit('close')">
    <div class="dialog">
      <h3>添加仓库</h3>
      <label>仓库地址 <input v-model="input" autocapitalize="off" spellcheck="false" placeholder="git 地址，可多个（空格/逗号分隔）" /></label>
      <p class="muted">添加后点击「同步最新代码」补建 worktree</p>
      <p v-if="err" class="err">{{ err }}</p>
      <div class="actions">
        <button :disabled="submitting" @click="emit('close')">取消</button>
        <button class="primary" :disabled="!input.trim() || submitting" @click="add">添加</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mask { position: fixed; inset: 0; background: var(--mask); display: flex; align-items: center; justify-content: center; z-index: 100; }
.dialog { background: var(--bg-soft); border: 1px solid var(--border); box-shadow: var(--shadow); border-radius: 8px; padding: 20px; width: 460px; display: flex; flex-direction: column; gap: 8px; }
label { display: grid; grid-template-columns: 5em 1fr; gap: 8px; align-items: center; font-size: 13px; }
/* 弹窗表单控件右列统一：高恒 32px、宽撑满列（去全局 320px 上限），各行右缘对齐 */
input { max-width: none; height: var(--control-h); }
.actions { display: flex; gap: 8px; justify-content: flex-end; }
.muted { color: var(--fg-muted); font-size: 13px; margin: 0; }
.err { color: var(--danger-text); font-size: 13px; margin: 0; }
</style>
