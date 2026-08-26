<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useCmdStore } from "../stores/cmd";
import { useHubStore } from "../stores/hub";
import { runGws, latestGwsVersion } from "../lib/gws-bridge";
import { parseVersion } from "../lib/parse";
import { stripAnsi } from "../lib/ansi";

const emit = defineEmits<{ (e: "close"): void }>();
const cmd = useCmdStore();
const hub = useHubStore();
const current = ref("");
// null = 尚未检查（远端行隐藏）；检查失败时同样复位 null
const latest = ref<string | null>(null);
const checking = ref(false);
const updating = ref(false);
const err = ref("");

async function loadCurrent() {
  err.value = "";
  try {
    const r = await runGws(["version"], hub.path);
    if (r.code === null || r.code !== 0) {
      // spawn 失败或非零退出：错误输出不能喂给 parseVersion（会显示垃圾版本号），取尾行展示
      current.value = "";
      err.value = stripAnsi(r.output.split("\n").filter(Boolean).pop() ?? "") || "gws version 执行失败";
      return;
    }
    current.value = parseVersion(r.output);
  } catch (e) {
    current.value = "";
    err.value = String(e);
  }
}
onMounted(loadCurrent);

async function check() {
  // 入口守卫：镜像 update——disabled 已防真实用户，此处防程序化双击（同步第二击落在
  // patch 滞后窗口内仍会派发）导致 checking 状态错乱
  if (checking.value || updating.value || cmd.isRunning()) return;
  checking.value = true;
  try {
    // Rust 侧 Result<String,String>：远端不可达/内容无效时 reject（走 catch），不会 resolve null
    latest.value = await latestGwsVersion();
  } catch (e) {
    latest.value = null;
    err.value = "检查更新失败: " + String(e);
  }
  checking.value = false;
}
async function update() {
  // 入口守卫：更新在途（防双击）或另有命令在跑时不发第二条
  if (updating.value || cmd.isRunning()) return;
  updating.value = true;
  try {
    const run = await cmd.exec("gws update", ["update"], hub.path);
    await cmd.waitDone(run); // exec 返回时 update 尚未跑完——等终态再查版本，否则拿到旧版本号
    // 非零退出上报（对照 AddModuleDialog 检查 state !== "done"）：loadCurrent 入口清 err，
    // 失败提示须设在其后，否则被静默抹掉
    const failed = run.state !== "done";
    await loadCurrent();
    if (failed) err.value = "gws update 未成功（详见输出面板）";
  } catch (e) {
    err.value = String(e);
  } finally {
    updating.value = false;
  }
}
function close() {
  // 更新在途禁关闭（mask 与关闭按钮共用）；检查更新无副作用，在途允许关闭
  if (!updating.value) emit("close");
}
</script>

<template>
  <div class="mask" @click.self="close">
    <div class="dialog">
      <h3>gws CLI</h3>
      <p v-if="err" class="error">{{ err }}</p>
      <p>当前版本: <strong>{{ current || "未知" }}</strong></p>
      <p v-if="latest !== null">远端最新: <strong>{{ latest || "无法获取" }}</strong>
        <span v-if="latest && current && latest === current" class="ok">● 已是最新版本</span>
      </p>
      <div class="actions">
        <button :disabled="updating" @click="close">关闭</button>
        <button :disabled="checking || updating || cmd.isRunning()" @click="check">{{ checking ? "检查中…" : "检查更新" }}</button>
        <button v-if="latest && current && latest !== current" class="primary" :disabled="updating || cmd.isRunning()" @click="update">{{ updating ? "更新中…" : `更新到 ${latest}` }}</button>
      </div>
      <p class="muted">更新源: github.com/qq476605474/gws（GWS_UPDATE_URL 可覆盖）</p>
    </div>
  </div>
</template>

<style scoped>
.mask { position: fixed; inset: 0; background: rgba(0, 0, 0, .35); display: flex; align-items: center; justify-content: center; z-index: 100; }
.dialog { background: #fff; border-radius: 8px; padding: 24px; width: 380px; }
.actions { display: flex; gap: 8px; justify-content: flex-end; }
.error { color: #c62828; font-size: 13px; }
.ok { color: #2e7d32; }
.muted { color: #888; font-size: 12px; }
.primary { background: #1565c0; color: #fff; }
</style>
