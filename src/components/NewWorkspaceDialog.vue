<script setup lang="ts">
import { ref, watch } from "vue";
import { useHubStore } from "../stores/hub";
import { useCmdStore } from "../stores/cmd";

const emit = defineEmits<{ (e: "close"): void; (e: "created"): void }>();
const hub = useHubStore();
const cmd = useCmdStore();

const name = ref("");
const title = ref("");
const modules = ref<string[]>([]);
const prefix = ref("feature");
const customBranch = ref("");
const err = ref("");
const submitting = ref(false);

async function create() {
  if (submitting.value) return; // 防双击：exec 的 IPC 往返间隙 isRunning 尚未生效
  err.value = "";
  const args = ["new", name.value];
  if (modules.value.length) args.push("--modules", modules.value.join(","));
  if (title.value) args.push("--title", title.value);
  if (prefix.value !== "feature") args.push("--prefix", prefix.value);
  if (customBranch.value) args.push("--branch", customBranch.value);
  submitting.value = true;
  try {
    const run = await cmd.exec(`gws new ${name.value}`, args, hub.path);
    emit("close");
    // 关闭弹窗但等命令结束后再通知刷新（watcher 在 await 后创建、不随组件卸载而停止，结束即自停）；
    // 命令若瞬间结束，exit 事件可先于 exec 的 promise 决议送达，此时现值已是终态、watcher 不会再触发，须先查现值
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
      <h3>新建需求工作区</h3>
      <label>名称 <input v-model="name" placeholder="如 checkout-revamp" /></label>
      <label>标题 <input v-model="title" placeholder="中文标题（可选）" /></label>
      <label>分支前缀
        <select v-model="prefix">
          <option value="feature">feature (默认)</option>
          <option value="hotfix">hotfix</option>
        </select>
      </label>
      <label>完全自定义分支名 <input v-model="customBranch" placeholder="留空则用前缀-日期-名称" /></label>
      <fieldset>
        <legend>模块（不选=全部仓库）</legend>
        <label v-for="r in hub.repos" :key="r.name">
          <input type="checkbox" :value="r.name" v-model="modules" /> {{ r.name }}
        </label>
      </fieldset>
      <p v-if="err" class="err">{{ err }}</p>
      <div class="actions">
        <button @click="emit('close')">取消</button>
        <button class="primary" :disabled="!name || submitting" @click="create">创建</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mask { position: fixed; inset: 0; background: var(--mask); display: flex; align-items: center; justify-content: center; z-index: 100; }
.dialog { background: var(--bg-soft); border: 1px solid var(--border); box-shadow: var(--shadow); border-radius: 8px; padding: 20px; width: 460px; display: flex; flex-direction: column; gap: 8px; }
label { display: flex; align-items: center; gap: 8px; font-size: 13px; }
input, select { flex: 1; }
fieldset { max-height: 160px; overflow: auto; }
.actions { display: flex; gap: 8px; justify-content: flex-end; }
.primary { background: var(--primary); color: var(--primary-fg); }
.err { color: var(--danger-text); font-size: 13px; margin: 0; }
</style>
