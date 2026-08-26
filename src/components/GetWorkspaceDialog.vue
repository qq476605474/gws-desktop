<script setup lang="ts">
import { ref, watch } from "vue";
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
    const run = await cmd.exec(`gws get ${branch.value}`, args, hub.path);
    emit("close");
    // 同 NewWorkspaceDialog：等命令结束再通知刷新；瞬间结束的命令现值已是终态，须先查现值
    if (run.state === "done" || run.state === "failed") {
      emit("created");
      return;
    }
    const stop = watch(() => run.state, (s) => {
      if (s === "done" || s === "failed") { stop(); emit("created"); }
    });
  } catch (e) {
    // exec reject（如 IPC 失败）：写入内联提示，弹窗不关闭，让用户看到错误后手动取消
    err.value = String(e);
  } finally {
    submitting.value = false;
  }
}
</script>

<template>
  <div class="mask" @click.self="emit('close')">
    <div class="dialog">
      <h3>拉取已推送的需求</h3>
      <label>远程 feature 分支 <input v-model="branch" placeholder="feature-20260818-checkout-revamp" /></label>
      <label>本地工作区名 <input v-model="name" placeholder="留空自动从分支名反推" /></label>
      <label>标题 <input v-model="title" placeholder="中文标题（可选）" /></label>
      <p v-if="err" class="err">{{ err }}</p>
      <div class="actions">
        <button @click="emit('close')">取消</button>
        <button class="primary" :disabled="!branch || submitting" @click="pull">拉取</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mask { position: fixed; inset: 0; background: rgba(0,0,0,.35); display: flex; align-items: center; justify-content: center; z-index: 100; }
.dialog { background: var(--bg-soft); border-radius: 8px; padding: 20px; width: 460px; display: flex; flex-direction: column; gap: 8px; }
label { display: flex; align-items: center; gap: 8px; font-size: 13px; }
input { flex: 1; }
.actions { display: flex; gap: 8px; justify-content: flex-end; }
.primary { background: var(--primary); color: var(--primary-fg); }
.err { color: var(--danger); font-size: 13px; margin: 0; }
</style>
