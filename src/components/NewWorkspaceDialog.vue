<script setup lang="ts">
import { computed, ref } from "vue";
import { useHubStore } from "../stores/hub";
import { useCmdStore } from "../stores/cmd";

const emit = defineEmits<{ (e: "close"): void; (e: "created"): void }>();
const hub = useHubStore();
const cmd = useCmdStore();

const nameInput = ref("");
const title = ref("");
const modules = ref<string[]>([]);
const prefix = ref("feature");
const customBranch = ref("");
const fromInput = ref("");
const err = ref("");
const submitting = ref(false);

/** gws get 同款反推：<前缀>-YYYYMMDD-<名称> → <名称>；不匹配该模式则退回完整分支名 */
function deriveName(branch: string): string {
  const m = /^[a-zA-Z]+-\d{8}-(.+)$/.exec(branch);
  return m?.[1] ?? branch;
}
/** customBranch 非空时名称从分支名反推（分支名里已有名称信息，不必重复手填） */
const derivedName = computed(() => {
  const branch = customBranch.value.trim();
  return branch ? deriveName(branch) : "";
});
/** 基线来源（--from）：空格或逗号分隔可填多个，顺序即优先级（主干自动兜底） */
const froms = computed(() => fromInput.value.split(/[\s,]+/).filter(Boolean));

async function create() {
  if (submitting.value) return; // 防双击：exec 的 IPC 往返间隙 isRunning 尚未生效
  err.value = "";
  const branch = customBranch.value.trim();
  const name = branch ? derivedName.value : nameInput.value.trim();
  const args = ["new", name];
  // 模块必选（不选=全部仓库的语义在 GUI 下容易误建超大工作区）
  args.push("--modules", modules.value.join(","));
  if (froms.value.length) args.push("--from", froms.value.join(","));
  if (title.value) args.push("--title", title.value);
  if (prefix.value !== "feature") args.push("--prefix", prefix.value);
  if (branch) args.push("--branch", branch);
  submitting.value = true;
  try {
    const run = await cmd.execDialog(`gws new ${name}`, args, hub.path);
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
      <!-- customBranch 非空：名称已在分支名里（反推规则同 gws get），隐藏输入框改为实时展示反推结果 -->
      <label v-if="!customBranch.trim()">名称 <input v-model="nameInput" autocapitalize="off" spellcheck="false" placeholder="必填，如 checkout-revamp（默认分支=前缀-日期-名称）" /></label>
      <p v-else class="muted">名称将从分支名反推：{{ derivedName }}</p>
      <label>标题 <input v-model="title" autocapitalize="off" spellcheck="false" placeholder="中文标题（可选，默认同名称）" /></label>
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
      <label>完全自定义分支名 <input v-model="customBranch" autocapitalize="off" spellcheck="false" placeholder="留空则用前缀-日期-名称" /></label>
      <!-- gws new --from 基线[,基线]...：可多次/逗号分隔，顺序即优先级，主干自动兜底 -->
      <label>基线来源 <input v-model="fromInput" autocapitalize="off" spellcheck="false" placeholder="可选，如 需求A 阶段2（空格/逗号分隔多个，留空=主干）" /></label>
      <fieldset>
        <legend>模块（必选）</legend>
        <label v-for="r in hub.repos" :key="r.name">
          <input type="checkbox" :value="r.name" v-model="modules" /> {{ r.name }}
        </label>
      </fieldset>
      <p v-if="err" class="err">{{ err }}</p>
      <div class="actions">
        <button :disabled="submitting" @click="emit('close')">取消</button>
        <button class="primary" :disabled="!(derivedName || nameInput.trim()) || !modules.length || submitting" @click="create">创建</button>
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
.muted { color: var(--fg-muted); font-size: 13px; margin: 0; }
.err { color: var(--danger-text); font-size: 13px; margin: 0; }
</style>
