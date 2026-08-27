<script setup lang="ts">
import { ref } from "vue";
import { useHubStore } from "../stores/hub";
import { useCmdStore } from "../stores/cmd";

const emit = defineEmits<{ (e: "close"): void; (e: "created"): void }>();
const hub = useHubStore();
const cmd = useCmdStore();
const branch = ref("");
const name = ref("");
const title = ref("");
const err = ref("");
const submitting = ref(false);

async function pull() {
  if (submitting.value) return; // 防双击：exec 的 IPC 往返间隙 isRunning 尚未生效
  err.value = "";
  const args = ["get", branch.value];
  if (name.value) args.push("--name", name.value);
  if (title.value) args.push("--title", title.value);
  submitting.value = true;
  try {
    const run = await cmd.execDialog(`gws get ${branch.value}`, args, hub.path);
    // 同 NewWorkspaceDialog：等命令终态再收尾（组件存活到 emit 时，刷新通知不丢）
    await cmd.waitDone(run);
    if (run.state !== "done") return; // 失败：不关窗不 emit created——错误见命令弹窗，输入保留便于重试
    emit("created"); // 先让父组件刷新列表
    emit("close"); // 再关窗
  } catch (e) {
    // exec/waitDone reject（如 IPC 失败）：写入内联提示，弹窗不关闭，让用户看到错误后手动取消
    err.value = String(e);
  } finally {
    submitting.value = false;
  }
}
</script>

<template>
  <div class="mask" @click.self="!submitting && emit('close')">
    <div class="dialog">
      <h3>拉取已推送的需求</h3>
      <label>远程 feature 分支 <input v-model="branch" placeholder="feature-20260818-checkout-revamp" /></label>
      <label>本地工作区名 <input v-model="name" placeholder="留空自动从分支名反推" /></label>
      <label>标题 <input v-model="title" placeholder="中文标题（可选）" /></label>
      <p v-if="err" class="err">{{ err }}</p>
      <div class="actions">
        <button :disabled="submitting" @click="emit('close')">取消</button>
        <button class="primary" :disabled="!branch || submitting" @click="pull">拉取</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mask { position: fixed; inset: 0; background: var(--mask); display: flex; align-items: center; justify-content: center; z-index: 100; }
.dialog { background: var(--bg-soft); border: 1px solid var(--border); box-shadow: var(--shadow); border-radius: 8px; padding: 20px; width: 460px; display: flex; flex-direction: column; gap: 8px; }
label { display: flex; align-items: center; gap: 8px; font-size: 13px; }
input { flex: 1; }
.actions { display: flex; gap: 8px; justify-content: flex-end; }
.primary { background: var(--primary); color: var(--primary-fg); }
.err { color: var(--danger-text); font-size: 13px; margin: 0; }
</style>
