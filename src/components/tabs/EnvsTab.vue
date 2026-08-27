<script setup lang="ts">
import { ref } from "vue";
import { confirm } from "@tauri-apps/plugin-dialog";
import { useHubStore } from "../../stores/hub";
import { useCmdStore } from "../../stores/cmd";
import { listDir } from "../../lib/gws-bridge";
import PathActions from "../PathActions.vue";

const hub = useHubStore();
const cmd = useCmdStore();
const newEnv = ref("");
const submitting = ref(false);

/** 环境行展开状态（键存在 = 展开）：行点击切换 */
const open = ref<Record<string, boolean>>({});
/** 各环境模块列表的懒加载缓存：loaded 后收起再展开直接用缓存不重拉；
 *  失败记 error——收起再展开会重试 */
const modules = ref<Record<string, { loading: boolean; error: string; list: string[] }>>({});

function toggle(e: string) {
  open.value[e] = !open.value[e];
  // 懒加载：仅在首次展开（或上次失败）时拉取；加载中不重复发起
  const m = modules.value[e];
  if (open.value[e] && (!m || (!m.loading && m.error))) void load(e);
}

async function load(e: string) {
  // 赋值后须经 modules.value[e]（reactive 代理）改字段：改本地原始对象不触发重渲染
  modules.value[e] = { loading: true, error: "", list: [] };
  // await 期间缓存槽可能被 invalidateModules/rmEnv 删除重建——须重新取槽，
  // 否则旧请求会写进新槽（覆盖新数据）或对已删槽赋值抛 TypeError
  const slot = modules.value[e]!;
  try {
    slot.list = await listDir(`${hub.path}/envs/${e}`);
  } catch (err) {
    slot.error = String(err);
  } finally {
    slot.loading = false;
  }
}

/** sync 补建模块目录会改变 envs/<名> 下的内容：清缓存并重拉已展开行，
 *  防止停留在过期的“（无模块）”旧列表 */
function invalidateModules() {
  for (const e of Object.keys(modules.value)) delete modules.value[e];
  for (const e of Object.keys(open.value)) {
    if (open.value[e]) void load(e);
  }
}

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
    if (run.state === "done") {
      // 移除成功：清该环境的展开缓存与展开态，防同会话删后重建同名环境命中旧缓存
      delete modules.value[e];
      delete open.value[e];
    }
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
  invalidateModules();
}
</script>

<template>
  <div>
    <div class="toolbar">
      <input v-model="newEnv" :disabled="cmd.isRunning()" autocapitalize="off" spellcheck="false" placeholder="环境分支名（如 pre、dev1）" />
      <button :disabled="!newEnv.trim() || cmd.isRunning() || submitting" @click="addEnv">+ 添加环境</button>
      <button class="primary" :disabled="cmd.isRunning() || submitting" @click="sync">同步最新代码</button>
    </div>
    <p v-if="hub.error" class="error">{{ hub.error }}</p>
    <div class="group-row">📁 envs <code>{{ hub.path }}/envs</code> <PathActions :path="`${hub.path}/envs`" /></div>
    <div v-for="e in hub.envs" :key="e" class="env-item">
      <!-- 整行可点击展开/收起；操作区 @click.stop 避免点按钮误触展开 -->
      <div class="env-row" @click="toggle(e)">
        <span class="env-main">
          <span class="arrow" aria-hidden="true">{{ open[e] ? "▾" : "▸" }}</span>
          <strong>{{ e }}</strong>
        </span>
        <span class="act" @click.stop>
          <PathActions :path="`${hub.path}/envs/${e}`" />
          <button class="btn-sm" :disabled="cmd.isRunning()" @click="rmEnv(e)">移除</button>
        </span>
      </div>
      <div v-if="open[e]" class="env-modules">
        <span v-if="modules[e]?.loading" class="muted">模块加载中…</span>
        <span v-else-if="modules[e]?.error" class="error">模块列表加载失败：{{ modules[e]!.error }}</span>
        <span v-else-if="modules[e] && !modules[e]!.list.length" class="muted">（无模块，跑 gws sync 补建）</span>
        <template v-else-if="modules[e]">
          <span v-for="m in modules[e]!.list" :key="m" class="mod-name">{{ m }}</span>
        </template>
      </div>
    </div>
    <p v-if="!hub.envs.length && !hub.error" class="muted">(暂无环境)</p>
  </div>
</template>

<style scoped>
.toolbar { display: flex; gap: 8px; margin-bottom: 12px; }
.group-row { display: flex; align-items: center; gap: 8px; margin-bottom: 12px; }
.env-item { border: 1px solid var(--border); border-radius: var(--radius); margin-bottom: 6px; }
.env-row { display: flex; justify-content: space-between; align-items: center; padding: 8px 12px; cursor: pointer; }
.env-row:hover { background: var(--bg-soft); }
.env-main { display: inline-flex; align-items: center; gap: 6px; min-width: 0; }
.arrow { color: var(--fg-muted); font-size: 11px; }
.act { display: inline-flex; align-items: center; gap: 8px; }
.env-modules { display: flex; flex-direction: column; gap: 3px; padding: 2px 12px 8px 30px; }
.mod-name { color: var(--fg-muted); font-size: 12px; }
.muted { color: var(--fg-muted); font-size: 12px; }
.error { color: var(--danger-text); font-size: 13px; }
</style>
