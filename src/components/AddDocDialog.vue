<script setup lang="ts">
import { ref } from "vue";
import { useHubStore } from "../stores/hub";
import { useCmdStore } from "../stores/cmd";

const emit = defineEmits<{ (e: "close"): void }>();
const hub = useHubStore();
const cmd = useCmdStore();
// props 不走 defineProps：DocsTab 用 v-if 挂载并经 provide/inject 传 ws 代价高，
// 直接以 props 传数据归属工作区
const props = defineProps<{ ws: string }>();
const input = ref("");
const err = ref("");
const submitting = ref(false);

async function create() {
  if (submitting.value) return; // 防双击：exec 的 IPC 往返间隙 isRunning 尚未生效
  const name = input.value.trim(); // 纯空格输入视同空
  if (!name) return;
  // gws doc new 不禁止空格名，但含空格的文件名会“已创建却列不出来”——入口直接拒绝
  if (/\s/.test(name)) {
    err.value = "文档名不能包含空格";
    return;
  }
  err.value = "";
  submitting.value = true;
  try {
    const run = await cmd.execDialog(`gws doc new ${name}`, ["doc", "new", name], `${hub.path}/ws/${props.ws}`);
    // 等命令终态再收尾：失败不关窗，输入保留便于重试
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
      <h3>新建文档</h3>
      <label>文档名 <input v-model="input" autocapitalize="off" spellcheck="false" placeholder="如 技术方案.md（不能包含空格）" @keydown.enter="create" /></label>
      <p class="muted">在当前需求（{{ ws }}）的文档目录下创建</p>
      <p v-if="err" class="err">{{ err }}</p>
      <div class="actions">
        <button :disabled="submitting" @click="emit('close')">取消</button>
        <button class="primary" :disabled="!input.trim() || submitting" @click="create">创建</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mask { position: fixed; inset: 0; background: var(--mask); display: flex; align-items: center; justify-content: center; z-index: 100; }
.dialog { background: var(--bg-soft); border: 1px solid var(--border); box-shadow: var(--shadow); border-radius: 8px; padding: 20px; width: 460px; display: flex; flex-direction: column; gap: 8px; }
label { display: grid; grid-template-columns: 5em 1fr; gap: 8px; align-items: center; font-size: 13px; }
input { max-width: none; height: var(--control-h); }
.actions { display: flex; gap: 8px; justify-content: flex-end; }
.muted { color: var(--fg-muted); font-size: 13px; margin: 0; }
.err { color: var(--danger-text); font-size: 13px; margin: 0; }
</style>
