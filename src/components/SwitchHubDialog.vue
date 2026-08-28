<script setup lang="ts">
import { ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { useHubStore } from "../stores/hub";
import { useSettingsStore } from "../stores/settings";
import { useCmdStore } from "../stores/cmd";
import { hubExists } from "../lib/gws-bridge";

const emit = defineEmits<{ (e: "close"): void }>();
const hub = useHubStore();
const settings = useSettingsStore();
const cmd = useCmdStore();
// 回显当前 hub 目录：既是查看处（顶栏不再占位展示长路径），也方便基于它微调
const hubPath = ref(hub.path);
const checking = ref(false);
const error = ref("");

// 新建 hub（gws init）：与 StartupView 的同名弹窗行为一致，成功后同样整体切换
const showInit = ref(false);
const initParent = ref("");
const initName = ref("");
const initErr = ref("");
const initing = ref(false);

async function pickHub() {
  try {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir === "string") hubPath.value = dir;
  } catch {
    // 选择器异常（极罕见）按取消处理
  }
}

async function openHub() {
  const p = hubPath.value.trim();
  if (!p || checking.value) return;
  // 同路径 no-op：setHub 会清空列表 store，但 path 不变则 MainView 不重挂载，
  // 页面会停在空列表——直接关窗即可
  if (p === hub.path) {
    emit("close");
    return;
  }
  checking.value = true;
  error.value = "";
  try {
    if (!(await hubExists(p))) {
      error.value = "该目录不是 gws hub（缺少 .gws-hub 标记）";
      return;
    }
    hub.setHub(p);
    settings.lastHub = p; // 下次启动自动进入新 hub
    emit("close"); // hub.path 变化令 App.vue 的 MainView :key 失效整体重挂载
  } catch (e) {
    error.value = String(e);
  } finally {
    checking.value = false;
  }
}

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
    emit("close"); // 新 hub 生效：MainView 重挂载，各 Tab 对空 hub 自然显示空态
  } catch (e) {
    initErr.value = String(e);
  } finally {
    initing.value = false;
  }
}
</script>

<template>
  <div class="mask" @click.self="emit('close')">
    <div class="dialog">
      <h3>切换 Hub 目录</h3>
      <label>目录路径 <input v-model="hubPath" autocapitalize="off" autocorrect="off" spellcheck="false" placeholder="hub 目录路径（含 .gws-hub）" /></label>
      <p class="muted">切换后页面状态（如已打开的需求详情）将重置</p>
      <p class="muted">没有 hub？<button class="link" @click="showInit = true">新建一个 →</button></p>
      <p v-if="error" class="err">{{ error }}</p>
      <div class="actions">
        <button @click="emit('close')">取消</button>
        <button @click="pickHub">浏览…</button>
        <button class="primary" :disabled="!hubPath.trim() || checking" @click="openHub">打开 Hub</button>
      </div>
    </div>

    <!-- 新建 hub 弹窗叠在切换弹窗之上（更高 z-index 挡住下层） -->
    <div v-if="showInit" class="mask init-mask" @click.self="!initing && (showInit = false)">
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
  </div>
</template>

<style scoped>
.mask { position: fixed; inset: 0; background: var(--mask); display: flex; align-items: center; justify-content: center; z-index: 100; }
/* hub 路径往往很长：弹窗放宽到 560px、输入框单行完整容纳 */
.dialog { background: var(--bg-soft); border: 1px solid var(--border); box-shadow: var(--shadow); border-radius: 8px; padding: 20px; width: 560px; display: flex; flex-direction: column; gap: 8px; }
label { display: grid; grid-template-columns: 5em 1fr; gap: 8px; align-items: center; font-size: 13px; }
input { max-width: none; height: var(--control-h); }
.actions { display: flex; gap: 8px; justify-content: flex-end; }
.muted { color: var(--fg-muted); font-size: 13px; margin: 0; }
.err { color: var(--danger-text); font-size: 13px; margin: 0; }
.link { border: none; background: none; padding: 0; min-height: 0; height: auto; width: auto; color: var(--primary); cursor: pointer; font-size: 13px; }
.link:hover { text-decoration: underline; }
.init-mask { z-index: 101; }
</style>
