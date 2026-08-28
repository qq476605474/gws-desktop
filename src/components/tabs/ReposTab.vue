<script setup lang="ts">
import { ref } from "vue";
import { confirm } from "@tauri-apps/plugin-dialog";
import { useHubStore } from "../../stores/hub";
import { useCmdStore } from "../../stores/cmd";
import PathActions from "../PathActions.vue";
import AddRepoDialog from "../AddRepoDialog.vue";

const hub = useHubStore();
const cmd = useCmdStore();
const showAdd = ref(false);
const submitting = ref(false);

/** 弹窗关闭即刷新：添加成功/失败/用户取消统一走此路径——repo add 失败也可能
 *  部分成功（多 URL 时），不刷新会让列表停留过期状态 */
async function onAddClose() {
  showAdd.value = false;
  await hub.refreshAll();
}

async function sync() {
  if (submitting.value) return; // 防本地在途时重复提交
  submitting.value = true;
  try {
    const run = await cmd.execDialog("gws sync", ["sync"], hub.path);
    await cmd.waitDone(run);
  } catch {
    // 同 addRepos：吞 reject，仍刷新
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
    const run = await cmd.execDialog(`gws repo rm ${name}`, ["repo", "rm", name], hub.path);
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
      <button :disabled="cmd.isRunning()" @click="showAdd = true">+ 添加仓库</button>
      <button class="primary" :disabled="cmd.isRunning() || submitting" @click="sync">同步最新代码</button>
    </div>
    <p v-if="hub.error" class="error">{{ hub.error }}</p>
    <div class="group-row">📁 repos <code>{{ hub.path }}/repos</code> <PathActions :path="`${hub.path}/repos`" /></div>
    <div v-for="r in hub.repos" :key="r.name" class="repo-row">
      <div>
        <strong>{{ r.name }}</strong> <span class="muted">主干 {{ r.mainBranch }}</span>
      </div>
      <span>
        <PathActions :path="`${hub.path}/repos/${r.name}`" />
        <button class="btn-sm" :disabled="cmd.isRunning()" @click="rm(r.name)">移除</button>
      </span>
    </div>
    <p v-if="!hub.repos.length && !hub.error" class="muted">(暂无仓库)</p>
    <p v-if="cmd.current?.label === 'gws repo add' && cmd.current.state !== 'failed'" class="hint">添加后点击「同步最新代码」补建 worktree</p>
    <AddRepoDialog v-if="showAdd" @close="onAddClose" />
  </div>
</template>

<style scoped>
.toolbar { display: flex; flex-wrap: wrap; gap: 8px; margin-bottom: 12px; }
.group-row { display: flex; align-items: center; gap: 8px; margin-bottom: 12px; }
.repo-row { display: flex; justify-content: space-between; align-items: center; border: 1px solid var(--border); border-radius: var(--radius); padding: 8px 12px; margin-bottom: 6px; }
.repo-row > span { display: inline-flex; align-items: center; gap: 8px; }
.muted { color: var(--fg-muted); font-size: 12px; }
.hint { color: var(--fg-muted); font-size: 13px; }
.error { color: var(--danger-text); font-size: 13px; }
</style>
