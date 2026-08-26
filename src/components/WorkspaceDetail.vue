<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { confirm } from "@tauri-apps/plugin-dialog";
import { useHubStore } from "../stores/hub";
import { useCmdStore } from "../stores/cmd";
import { runGws } from "../lib/gws-bridge";
import { parseSt, type StResult } from "../lib/parse";
import PathActions from "./PathActions.vue";
import AddModuleDialog from "./AddModuleDialog.vue";

const props = defineProps<{ name: string }>();
const emit = defineEmits<{ (e: "close"): void }>();
const hub = useHubStore();
const cmd = useCmdStore();
const st = ref<StResult | null>(null);
const wsPath = computed(() => `${hub.path}/ws/${props.name}`);
const showAdd = ref(false);
const mergeEnv = ref("");

async function refresh() {
  const r = await runGws(["st"], wsPath.value);
  st.value = parseSt(r.output);
}
async function doCmd(label: string, args: string[], cwd: string = wsPath.value) {
  const run = await cmd.exec(label, args, cwd);
  await cmd.waitDone(run); // 等终态再 refresh，否则命令仍在跑、拿到的是旧数据
  await refresh();
}
async function removeWs() {
  // macOS WKWebView 下原生 window.confirm 恒返回 false，须用插件的原生对话框
  let ok = false;
  try {
    ok = await confirm(`确认删除工作区 ${props.name}？文档自动归档，未推送代码分支保留。`);
  } catch {
    return; // 理论上不 reject；万一异常按取消处理，避免组件崩
  }
  if (!ok) return;
  const run = await cmd.exec(`gws rm ${props.name} --force`, ["rm", props.name, "--force"], hub.path);
  await cmd.waitDone(run);
  if (run.state === "done") emit("close");
}
onMounted(refresh);
</script>

<template>
  <div class="detail">
    <header class="head">
      <button @click="emit('close')">← 返回</button>
      <h2>{{ name }} <small>{{ st?.title }}</small></h2>
      <code>{{ wsPath }}</code>
      <PathActions :path="wsPath" />
    </header>
    <div class="ops">
      <button :disabled="cmd.isRunning()" @click="doCmd('gws pull', ['pull'])">Pull</button>
      <button :disabled="cmd.isRunning()" @click="doCmd('gws pull --rebase', ['pull', '--rebase'])">Pull --rebase</button>
      <button :disabled="cmd.isRunning()" @click="doCmd('gws push', ['push'])">Push</button>
      <select v-model="mergeEnv">
        <option value="" disabled>选择环境…</option>
        <option v-for="e in hub.envs" :key="e" :value="e">{{ e }}</option>
      </select>
      <button :disabled="!mergeEnv || cmd.isRunning()" @click="mergeEnv && doCmd(`gws merge ${mergeEnv}`, ['merge', mergeEnv])">Merge（本地）</button>
      <button :disabled="!mergeEnv || cmd.isRunning()" @click="mergeEnv && doCmd(`gws merge ${mergeEnv} --push`, ['merge', mergeEnv, '--push'])">Merge+Push</button>
      <button :disabled="cmd.isRunning()" @click="doCmd('gws sync-main', ['sync-main', '--yes'])">Sync-main</button>
      <button :disabled="cmd.isRunning()" @click="doCmd('gws sync', ['sync'], hub.path)">Sync</button>
      <button :disabled="cmd.isRunning()" @click="doCmd('gws done', ['done'])">Done 校验</button>
      <button :disabled="cmd.isRunning()" @click="showAdd = true">+ 模块</button>
      <button class="danger" :disabled="cmd.isRunning()" @click="removeWs">删除工作区</button>
    </div>
    <table v-if="st">
      <thead><tr><th>模块</th><th>分支</th><th>改动</th><th>vs远程</th><th>vs主干</th><th>操作</th></tr></thead>
      <tbody>
        <tr v-for="m in st.modules" :key="m.name">
          <td>{{ m.name }} <PathActions :path="`${wsPath}/${m.name}`" /></td>
          <td><code>{{ st.branch }}</code></td>
          <td :class="{ warn: (m.dirty ?? 0) > 0 }">{{ m.missing ? "目录缺失" : m.dirty }}</td>
          <td>{{ m.pushed === false ? "未推送" : `↑${m.ahead} ↓${m.behind}` }}</td>
          <td>+{{ m.aheadOfMain }}</td>
          <td><button :disabled="cmd.isRunning()" @click="doCmd(`gws drop ${m.name}`, ['drop', m.name])">移除</button></td>
        </tr>
      </tbody>
    </table>
    <p v-else class="muted">加载中…</p>
    <AddModuleDialog v-if="showAdd" :ws-path="wsPath" @close="showAdd = false" @added="refresh" />
  </div>
</template>

<style scoped>
.detail { padding: 4px 0; }
.head { display: flex; align-items: center; gap: 10px; margin-bottom: 10px; flex-wrap: wrap; }
.ops { display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 12px; align-items: center; }
table { width: 100%; border-collapse: collapse; font-size: 13px; }
th, td { text-align: left; padding: 6px 10px; border-bottom: 1px solid #eee; }
.warn { color: #e65100; font-weight: 600; }
.danger { color: #c62828; }
small { color: #888; font-weight: 400; }
.muted { color: #888; font-size: 13px; }
</style>
