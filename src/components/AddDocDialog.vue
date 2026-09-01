<script setup lang="ts">
import { ref } from "vue";
import { HUB_ROOT } from "../lib/consts";
import { useHubStore } from "../stores/hub";
import { useCmdStore } from "../stores/cmd";
import { joinPath } from "../lib/path";

const emit = defineEmits<{ (e: "close"): void }>();
const hub = useHubStore();
const cmd = useCmdStore();
// 目标目录在弹窗内选（用户反馈 #12）：默认取外层筛选当前选中（根文档或某需求），可改
const props = defineProps<{ defaultDest: string }>();
const dest = ref(props.defaultDest);
const input = ref("");
const err = ref("");
const submitting = ref(false);
/** doc new 的 cwd：根文档在 hub 根跑（gws 无工作区上下文时落 docs/ 第一层），需求在 ws/<名> */
const cwd = () => (dest.value === HUB_ROOT ? hub.path : joinPath(hub.path, "ws", dest.value));

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
    const run = await cmd.execDialog(`gws doc new ${name}`, ["doc", "new", name], cwd());
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
      <label>创建到
        <select v-model="dest">
          <option :value="HUB_ROOT">根文档（docs/）</option>
          <option v-for="w in hub.workspaces.filter((w) => w.name !== HUB_ROOT)" :key="w.name" :value="w.name">{{ w.name }}</option>
        </select>
      </label>
      <label>文档名 <input v-model="input" autocapitalize="off" spellcheck="false" placeholder="如 技术方案.md（不能包含空格）" @keydown.enter="create" /></label>
      <p class="muted">{{ dest === HUB_ROOT ? "在 hub 根 docs/ 下创建（跨需求共享）" : `在需求「${dest}」的文档目录下创建` }}</p>
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
input, select { max-width: none; height: var(--control-h); }
.actions { display: flex; gap: 8px; justify-content: flex-end; }
.muted { color: var(--fg-muted); font-size: 13px; margin: 0; }
.err { color: var(--danger-text); font-size: 13px; margin: 0; }
</style>
