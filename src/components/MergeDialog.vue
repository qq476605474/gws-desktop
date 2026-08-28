<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useHubStore } from "../stores/hub";
import { runGws } from "../lib/gws-bridge";
import { parseEnvLs } from "../lib/parse";

/** mode：local=合到环境本地分支（不推送）；push=合并并推送到远程 */
const props = defineProps<{ mode: "local" | "push" }>();
const emit = defineEmits<{ (e: "close"): void; (e: "run", env: string): void }>();
const hub = useHubStore();
const envs = ref<string[]>([]);
const selected = ref("");
const loading = ref(true);
const err = ref("");

// 每次打开（挂载）都重取 env ls：hub.envs 是进 tab 时的快照，期间新建/删除环境会
// 滞后，合并目标以现场实时列表为准
onMounted(refresh);

async function refresh() {
  loading.value = true;
  err.value = "";
  try {
    const r = await runGws(["env", "ls"], hub.path);
    if (r.code === null) {
      err.value = r.output; // spawn 失败（如 gws 被卸载）：错误信息直接展示
      return;
    }
    // env ls 无环境时退出码非 0 属正常，照常解析（模式同 hub.refreshAll）
    envs.value = parseEnvLs(r.output);
  } catch (e) {
    err.value = String(e); // runGws reject（IPC 失败）
  } finally {
    loading.value = false;
  }
}

function start() {
  if (!selected.value) return; // 未选环境不允许提交（按钮亦禁用，双保险）
  emit("run", selected.value);
  emit("close"); // 双发同 SyncMainDialog：父组件 v-if 卸载，独立使用时也自关闭
}
</script>

<template>
  <div class="mask" @click.self="emit('close')">
    <div class="dialog">
      <h3>{{ mode === "push" ? "合并并推送" : "合并到环境（本地）" }}</h3>
      <p class="muted">
        {{ mode === "push"
          ? "把当前需求合到所选环境并推送到远程"
          : "把当前需求合到所选环境的本地分支（不推送），可先 review 再推" }}
      </p>
      <p v-if="loading" class="muted">加载环境中…</p>
      <p v-else-if="err" class="err">{{ err }} <button @click="refresh">重试</button></p>
      <p v-else-if="!envs.length" class="muted">（暂无环境，先在「环境」tab 创建）</p>
      <div v-else class="envs">
        <button v-for="e in envs" :key="e" :class="{ on: selected === e }" @click="selected = e">{{ e }}</button>
      </div>
      <div class="actions">
        <button @click="emit('close')">取消</button>
        <button class="primary" :disabled="!selected" @click="start">
          {{ mode === "push" ? "合并并推送" : "开始合并" }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mask { position: fixed; inset: 0; background: var(--mask); display: flex; align-items: center; justify-content: center; z-index: 100; }
.dialog { background: var(--bg-soft); border: 1px solid var(--border); box-shadow: var(--shadow); border-radius: 8px; padding: 20px; width: 460px; display: flex; flex-direction: column; gap: 8px; }
.envs { display: flex; flex-wrap: wrap; gap: 8px; margin: 4px 0; }
.envs button.on { border-color: var(--primary); color: var(--primary); font-weight: 600; }
.actions { display: flex; gap: 8px; justify-content: flex-end; }
.muted { color: var(--fg-muted); font-size: 13px; margin: 0; }
.err { color: var(--danger-text); font-size: 13px; margin: 0; }
.err button { margin-left: 6px; }
</style>
