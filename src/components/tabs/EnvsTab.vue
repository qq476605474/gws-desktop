<script setup lang="ts">
import { ref } from "vue";
import { confirm } from "@tauri-apps/plugin-dialog";
import { useHubStore } from "../../stores/hub";
import { useCmdStore } from "../../stores/cmd";
import PathActions from "../PathActions.vue";

const hub = useHubStore();
const cmd = useCmdStore();
const newEnv = ref("");
const submitting = ref(false);

async function addEnv() {
  if (submitting.value) return; // 防双击：exec 的 IPC 往返间隙 isRunning 尚未生效
  const name = newEnv.value.trim(); // 纯空格输入视同空，避免把空白当分支名传给 gws
  if (!name) return;
  submitting.value = true;
  try {
    const run = await cmd.execDialog(`gws env add ${name}`, ["env", "add", name], hub.path);
    // exec 返回时命令仍在跑（事件流异步），须等终态再刷新，否则拿到的是旧列表
    await cmd.waitDone(run);
    if (run.state === "done") newEnv.value = ""; // 失败保留输入便于重试
  } catch {
    // exec reject（如 IPC 失败）：吞掉避免 unhandled rejection；数据未变，刷新无害
  } finally {
    submitting.value = false;
  }
  await hub.refreshAll();
}

async function rmEnv(e: string) {
  // macOS WKWebView 下原生 confirm 恒返回 false，须用插件原生对话框；异常按取消处理
  let ok = false;
  try {
    ok = await confirm(`移除环境 ${e}？`);
  } catch {
    return;
  }
  if (!ok) return;
  if (cmd.isRunning()) return; // confirm 弹窗打开期间用户可能已从另一入口启动命令
  try {
    const run = await cmd.execDialog(`gws env rm ${e}`, ["env", "rm", e], hub.path);
    await cmd.waitDone(run);
  } catch {
    // 同 addEnv：吞 reject，仍刷新
  }
  await hub.refreshAll();
}

async function sync() {
  if (submitting.value) return; // 同 addEnv：防本地在途时重复提交
  submitting.value = true;
  try {
    const run = await cmd.execDialog("gws sync", ["sync"], hub.path);
    await cmd.waitDone(run);
  } catch {
    // 同 addEnv：吞 reject，仍刷新
  } finally {
    submitting.value = false;
  }
  await hub.refreshAll();
}
</script>

<template>
  <div>
    <div class="toolbar">
      <input v-model="newEnv" :disabled="cmd.isRunning()" placeholder="环境分支名（如 pre、dev1）" />
      <button :disabled="!newEnv.trim() || cmd.isRunning() || submitting" @click="addEnv">+ 添加环境</button>
      <button class="primary" :disabled="cmd.isRunning() || submitting" @click="sync">🔄 gws sync</button>
    </div>
    <p v-if="hub.error" class="error">{{ hub.error }}</p>
    <div class="group-row">📁 envs <code>{{ hub.path }}/envs</code> <PathActions :path="`${hub.path}/envs`" /></div>
    <div v-for="e in hub.envs" :key="e" class="env-row">
      <div><strong>{{ e }}</strong> <span class="muted">模块数见 sync 输出</span></div>
      <span>
        <PathActions :path="`${hub.path}/envs/${e}`" />
        <button class="btn-sm" :disabled="cmd.isRunning()" @click="rmEnv(e)">移除</button>
      </span>
    </div>
    <p v-if="!hub.envs.length && !hub.error" class="muted">(暂无环境)</p>
  </div>
</template>

<style scoped>
.toolbar { display: flex; gap: 8px; margin-bottom: 12px; }
.group-row { display: flex; align-items: center; gap: 8px; margin-bottom: 12px; }
.env-row { display: flex; justify-content: space-between; align-items: center; border: 1px solid var(--border); border-radius: var(--radius); padding: 8px 12px; margin-bottom: 6px; }
.env-row > span { display: inline-flex; align-items: center; gap: 8px; }
.muted { color: var(--fg-muted); font-size: 12px; }
.error { color: var(--danger-text); font-size: 13px; }
</style>
