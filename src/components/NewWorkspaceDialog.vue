<script setup lang="ts">
import { ref } from "vue";
import { useHubStore } from "../stores/hub";
import { useCmdStore } from "../stores/cmd";

const emit = defineEmits<{ (e: "close"): void; (e: "created"): void }>();
const hub = useHubStore();
const cmd = useCmdStore();

const name = ref("");
const title = ref("");
const modules = ref<string[]>([]);
const prefix = ref("feature");
const customBranch = ref("");
const err = ref("");
const submitting = ref(false);

async function create() {
  if (submitting.value) return; // 防双击：exec 的 IPC 往返间隙 isRunning 尚未生效
  err.value = "";
  const args = ["new", name.value];
  if (modules.value.length) args.push("--modules", modules.value.join(","));
  if (title.value) args.push("--title", title.value);
  if (prefix.value !== "feature") args.push("--prefix", prefix.value);
  if (customBranch.value) args.push("--branch", customBranch.value);
  submitting.value = true;
  try {
    const run = await cmd.execDialog(`gws new ${name.value}`, args, hub.path);
    // 等命令终态再收尾：组件必须存活到 emit 时（Vue 对已卸载实例的 emit 是 no-op，
    // 先前的「立即关窗 + watcher 里补发 created」会让刷新通知永远丢失）
    await cmd.waitDone(run);
    if (run.state !== "done") return; // 失败：不关窗不 emit created——错误见命令弹窗，输入保留便于重试
    emit("created"); // 先让父组件刷新列表
    emit("close"); // 再关窗（同一同步块内，父组件先收 created）
  } catch (e) {
    // exec/waitDone reject（如 IPC 失败）：写入内联提示，弹窗不关闭，让用户看到错误后手动取消
    err.value = String(e);
  } finally {
    submitting.value = false;
  }
}
</script>

<template>
  <div class="mask" @click.self="!submitting && emit('close')">
    <div class="dialog">
      <h3>新建需求工作区</h3>
      <!-- gws new 的名称是必填位置参数（无留空反推：留空直接报用法错误）；
           默认分支 = <前缀>-<日期YYYYMMDD>-<名称>，标题缺省同名称 -->
      <label>名称 <input v-model="name" placeholder="必填，如 checkout-revamp（默认分支=前缀-日期-名称）" /></label>
      <label>标题 <input v-model="title" placeholder="中文标题（可选，默认同名称）" /></label>
      <label>分支前缀
        <select v-model="prefix">
          <option value="feature">feature (默认)</option>
          <option value="bugfix">bugfix</option>
          <option value="hotfix">hotfix</option>
          <option value="release">release</option>
          <option value="support">support</option>
          <option value="docs">docs</option>
          <option value="refactor">refactor</option>
          <option value="test">test</option>
          <option value="chore">chore</option>
        </select>
      </label>
      <label>完全自定义分支名 <input v-model="customBranch" placeholder="留空则用前缀-日期-名称" /></label>
      <fieldset>
        <legend>模块（不选=全部仓库）</legend>
        <label v-for="r in hub.repos" :key="r.name">
          <input type="checkbox" :value="r.name" v-model="modules" /> {{ r.name }}
        </label>
      </fieldset>
      <p v-if="err" class="err">{{ err }}</p>
      <div class="actions">
        <button :disabled="submitting" @click="emit('close')">取消</button>
        <button class="primary" :disabled="!name || submitting" @click="create">创建</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mask { position: fixed; inset: 0; background: var(--mask); display: flex; align-items: center; justify-content: center; z-index: 100; }
.dialog { background: var(--bg-soft); border: 1px solid var(--border); box-shadow: var(--shadow); border-radius: 8px; padding: 20px; width: 460px; display: flex; flex-direction: column; gap: 8px; }
label { display: flex; align-items: center; gap: 8px; font-size: 13px; }
/* 排除 checkbox：flex:1 会把模块勾选框横向拉满整行（配合 base.css 的 checkbox 定尺寸） */
input:not([type="checkbox"]), select { flex: 1; }
/* 滚动叶子区：contain 防模块列表滚到底后滚动链穿透 */
fieldset { max-height: 160px; overflow: auto; overscroll-behavior: contain; }
.actions { display: flex; gap: 8px; justify-content: flex-end; }
.err { color: var(--danger-text); font-size: 13px; margin: 0; }
</style>
