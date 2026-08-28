<script setup lang="ts">
import { onMounted, ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { navigate } from "../router";
import { useSettingsStore } from "../stores/settings";
import { useHubStore } from "../stores/hub";
import { useCmdStore } from "../stores/cmd";
import { checkGwsInstalled, hubExists } from "../lib/gws-bridge";

const settings = useSettingsStore();
const hub = useHubStore();
const cmd = useCmdStore();
const installed = ref(true);
const hubPath = ref("");
const error = ref("");
// 新建 hub：输入父目录下要创建的新目录名，gws init <父/名> 建 hub 标记+标准结构
const showInit = ref(false);
const initParent = ref("");
const initName = ref("");
const initErr = ref("");
const initing = ref(false);

async function pickHub() {
  const dir = await open({ directory: true, multiple: false });
  if (typeof dir === "string") hubPath.value = dir;
}
async function openHub() {
  if (!hubPath.value) return;
  if (!(await hubExists(hubPath.value))) {
    error.value = "该目录不是 gws hub（缺少 .gws-hub 标记）";
    return;
  }
  hub.setHub(hubPath.value);
  settings.lastHub = hubPath.value;
  navigate("main");
}
onMounted(async () => {
  installed.value = await checkGwsInstalled();
  if (installed.value && settings.lastHub) {
    hubPath.value = settings.lastHub;
    await openHub(); // 上次 hub 存在且 gws 可用则直接进
  }
});

async function pickInitParent() {
  try {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir === "string") initParent.value = dir;
  } catch {
    // 选择器异常（极罕见）按取消处理
  }
}
/** 新建 hub 的完整目标路径：父目录 + 新目录名（末尾斜杠容错） */
function initTarget(): string {
  const parent = initParent.value.trim().replace(/\/+$/, "");
  const name = initName.value.trim();
  return name ? `${parent}/${name}` : "";
}
async function initHub() {
  const target = initTarget();
  if (!target || initing.value) return;
  initing.value = true;
  initErr.value = "";
  try {
    // gws init 的校验很全（已存在/嵌套/非空目录都会 die），界面不重复校验，
    // 命令弹窗可见完整输出；失败留在弹窗可改可取消
    const run = await cmd.execDialog(`gws init ${target}`, ["init", target], initParent.value.trim() || ".");
    await cmd.waitDone(run);
    if (run.state !== "done") return;
    hub.setHub(target);
    settings.lastHub = target;
    showInit.value = false;
    navigate("main"); // 新 hub 直达主界面（各 Tab 对空 hub 自然显示空态）
  } catch (e) {
    // exec/waitDone reject（如 IPC 失败）：内联提示，弹窗不关闭
    initErr.value = String(e);
  } finally {
    initing.value = false;
  }
}
</script>

<template>
  <main class="startup">
    <h1>GwsDesk</h1>
    <p v-if="!installed" class="warn">
      未检测到 gws 命令。安装：
      <code>curl -fsSL https://raw.githubusercontent.com/qq476605474/gws/main/gws -o ~/.local/bin/gws && chmod +x ~/.local/bin/gws</code>
    </p>
    <div class="pick">
      <input v-model="hubPath" placeholder="hub 目录路径（含 .gws-hub）" />
      <button @click="pickHub">浏览…</button>
      <button :disabled="!hubPath" @click="openHub">打开 Hub</button>
    </div>
    <p v-if="error" class="warn">{{ error }}</p>
    <button class="link" @click="showInit = true">没有 hub？新建一个 →</button>

    <div v-if="showInit" class="mask" @click.self="!initing && (showInit = false)">
      <div class="dialog">
        <h3>新建 Hub</h3>
        <label>父目录 <input v-model="initParent" autocapitalize="off" autocorrect="off" spellcheck="false" placeholder="新 hub 所在的父目录（如 ~/Documents/dev）" /></label>
        <label>新目录名 <input v-model="initName" autocapitalize="off" autocorrect="off" spellcheck="false" placeholder="如 myhub（在该父目录下创建）" /></label>
        <p class="muted">gws init 在父目录下创建 hub 标准结构（repos/envs/ws/docs），环境分支默认 dev</p>
        <p v-if="initErr" class="err">{{ initErr }}</p>
        <div class="actions">
          <button :disabled="initing" @click="showInit = false">取消</button>
          <button @click="pickInitParent">浏览…</button>
          <button class="primary" :disabled="!initTarget() || initing" @click="initHub">创建</button>
        </div>
      </div>
    </div>
  </main>
</template>

<style scoped>
.startup { display: flex; flex-direction: column; align-items: center; justify-content: center; height: 100vh; gap: 16px; }
.warn { color: var(--danger-text); max-width: 480px; }
.pick { display: flex; gap: 8px; }
.link { border: none; background: none; padding: 0; min-height: 0; color: var(--primary); cursor: pointer; }
.link:hover { text-decoration: underline; }
.mask { position: fixed; inset: 0; background: var(--mask); display: flex; align-items: center; justify-content: center; z-index: 100; }
.dialog { background: var(--bg-soft); border: 1px solid var(--border); box-shadow: var(--shadow); border-radius: 8px; padding: 20px; width: 560px; display: flex; flex-direction: column; gap: 8px; }
label { display: grid; grid-template-columns: 5em 1fr; gap: 8px; align-items: center; font-size: 13px; }
input { max-width: none; height: var(--control-h); }
.actions { display: flex; gap: 8px; justify-content: flex-end; }
.muted { color: var(--fg-muted); font-size: 13px; margin: 0; }
.err { color: var(--danger-text); font-size: 13px; margin: 0; }
</style>
