<script setup lang="ts">
import { ref } from "vue";
import { confirm } from "@tauri-apps/plugin-dialog";
import { useHubStore } from "../../stores/hub";
import { useCmdStore } from "../../stores/cmd";
import PathActions from "../PathActions.vue";

const hub = useHubStore();
const cmd = useCmdStore();
const input = ref("");
const submitting = ref(false);

async function addRepos() {
  if (submitting.value) return; // 防双击：exec 的 IPC 往返间隙 isRunning 尚未生效
  const urls = input.value.split(/\s+/).filter(Boolean);
  if (!urls.length) return;
  submitting.value = true;
  try {
    const run = await cmd.exec("gws repo add", ["repo", "add", ...urls], hub.path);
    // exec 返回时命令仍在跑（事件流异步），须等终态再刷新，否则拿到的是旧列表
    await cmd.waitDone(run);
    if (run.state === "done") input.value = ""; // 失败保留输入便于重试
  } catch {
    // exec reject（如 IPC 失败）：吞掉避免 unhandled rejection；数据未变，刷新无害
  } finally {
    submitting.value = false;
  }
  await hub.refreshAll();
}

async function rm(name: string) {
  // macOS WKWebView 下原生 confirm 恒返回 false，须用插件原生对话框；异常按取消处理
  let ok = false;
  try {
    ok = await confirm(`移除仓库 ${name}？`);
  } catch {
    return;
  }
  if (!ok) return;
  if (cmd.isRunning()) return; // confirm 弹窗打开期间用户可能已从另一入口启动命令
  try {
    const run = await cmd.exec(`gws repo rm ${name}`, ["repo", "rm", name], hub.path);
    await cmd.waitDone(run);
  } catch {
    // 同 addRepos：吞 reject，仍刷新
  }
  await hub.refreshAll();
}
</script>

<template>
  <div>
    <div class="toolbar">
      <input v-model="input" :disabled="cmd.isRunning()" placeholder="git 地址（可多个，空格分隔）" style="flex:1" />
      <button class="primary" :disabled="!input.trim() || cmd.isRunning() || submitting" @click="addRepos">+ 添加仓库</button>
    </div>
    <p v-if="hub.error" class="error">{{ hub.error }}</p>
    <div class="group-row">📁 repos <code>{{ hub.path }}/repos</code> <PathActions :path="`${hub.path}/repos`" /></div>
    <div v-for="r in hub.repos" :key="r.name" class="repo-row">
      <div>
        <strong>{{ r.name }}</strong> <span class="muted">主干 {{ r.mainBranch }}</span>
      </div>
      <span>
        <PathActions :path="`${hub.path}/repos/${r.name}`" />
        <button :disabled="cmd.isRunning()" @click="rm(r.name)">移除</button>
      </span>
    </div>
    <p v-if="!hub.repos.length && !hub.error" class="muted">(暂无仓库)</p>
    <p v-if="cmd.current?.label === 'gws repo add' && cmd.current.state !== 'failed'" class="hint">添加后执行环境 Tab 的 gws sync 补建 worktree</p>
  </div>
</template>

<style scoped>
.toolbar { display: flex; gap: 8px; margin-bottom: 12px; }
.group-row { display: flex; align-items: center; gap: 8px; margin-bottom: 12px; }
.repo-row { display: flex; justify-content: space-between; align-items: center; border: 1px solid var(--border); border-radius: var(--radius); padding: 8px 12px; margin-bottom: 6px; }
.repo-row > span { display: inline-flex; align-items: center; gap: 8px; }
.muted { color: var(--fg-muted); font-size: 12px; }
.hint { color: var(--fg-muted); font-size: 13px; }
.error { color: var(--danger-text); font-size: 13px; }
.primary { background: var(--primary); color: var(--primary-fg); }
</style>
