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
const fromInput = ref("");
const err = ref("");
const submitting = ref(false);

// 分支两种互斥填法（用户反馈 #12）：组合 = 前缀+分支名（提交时拼 <前缀>-<日期>-<分支名>），
// 完整 = 整支手填原样使用；单选切换，未选模式的一组输入不渲染（互斥由 v-if 天然保证）
const branchMode = ref<"compose" | "full">("compose");
const PREFIXES = ["feature", "bugfix", "hotfix", "release", "support", "docs", "refactor", "test", "chore"];
const prefix = ref("feature");
const branchName = ref("");
const customBranch = ref("");

/** 切换分支方式时清空另一组输入：隐藏字段的残留值不该再出现在提交参数里 */
function onModeChange() {
  if (branchMode.value === "compose") {
    customBranch.value = "";
  } else {
    prefix.value = "feature";
    branchName.value = "";
  }
}

function today(): string {
  const d = new Date();
  return `${d.getFullYear()}${String(d.getMonth() + 1).padStart(2, "0")}${String(d.getDate()).padStart(2, "0")}`;
}
/** 组合模式的完整分支（提交即此值）：分支名留空退回名称作后缀（= gws 默认拼接） */
const composedBranch = computed(() => {
  return `${prefix.value}-${today()}-${branchName.value.trim() || nameInput.value.trim()}`;
});
/** 基线来源（--from）：空格或逗号分隔可填多个，顺序即优先级（主干自动兜底） */
const froms = computed(() => fromInput.value.split(/[\s,]+/).filter(Boolean));
/** 创建可用：名称必填；完整模式分支必填（组合模式前缀是固定 select 恒有值）；模块恒必选 */
const canCreate = computed(() => {
  if (!nameInput.value.trim() || !modules.value.length) return false;
  return branchMode.value === "compose" ? true : !!customBranch.value.trim();
});

async function create() {
  if (submitting.value || !canCreate.value) return; // 防双击：exec 的 IPC 往返间隙 isRunning 尚未生效
  err.value = "";
  const name = nameInput.value.trim();
  const branch = branchMode.value === "compose" ? composedBranch.value : customBranch.value.trim();
  const args = ["new", name];
  // 模块必选（不选=全部仓库的语义在 GUI 下容易误建超大工作区）
  args.push("--modules", modules.value.join(","));
  if (froms.value.length) args.push("--from", froms.value.join(","));
  if (title.value) args.push("--title", title.value);
  // 分支恒显式传（--branch 优先级高于 gws 默认拼接，预览即所得）
  args.push("--branch", branch);
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
      <h3>新建需求</h3>
      <!-- 名称是必填位置参数（目录名，可中文）；分支两种填法二选一，标题缺省同名称 -->
      <label>名称 <input v-model="nameInput" autocapitalize="off" spellcheck="false" placeholder="目录名（必填，可中文），如 收银台改版" /></label>
      <div class="field-row">分支方式
        <span class="radios">
          <label class="inline"><input type="radio" value="compose" v-model="branchMode" @change="onModeChange" />前缀组合</label>
          <label class="inline"><input type="radio" value="full" v-model="branchMode" @change="onModeChange" />完整分支名</label>
        </span>
      </div>
      <template v-if="branchMode === 'compose'">
        <!-- 前缀固定下拉（用户反馈：datalist 输入过滤选中后看不到其他项）；默认 feature -->
        <label>前缀
          <select v-model="prefix">
            <option v-for="p in PREFIXES" :key="p" :value="p">{{ p === "feature" ? "feature (默认)" : p }}</option>
          </select>
        </label>
        <label>分支名 <input v-model="branchName" autocapitalize="off" spellcheck="false" placeholder="英文后缀，如 checkout-revamp（留空用名称）" /></label>
        <p class="muted">分支：{{ composedBranch }}</p>
      </template>
      <label v-else>完整分支名 <input v-model="customBranch" autocapitalize="off" spellcheck="false" placeholder="如 feature-20260828-checkout-revamp" /></label>
      <label>标题 <input v-model="title" autocapitalize="off" spellcheck="false" placeholder="中文标题（可选，默认同名称）" /></label>
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
        <button class="primary" :disabled="!canCreate || submitting" @click="create">创建</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mask { position: fixed; inset: 0; background: var(--mask); display: flex; align-items: center; justify-content: center; z-index: 100; }
.dialog { background: var(--bg-soft); border: 1px solid var(--border); box-shadow: var(--shadow); border-radius: 8px; padding: 20px; width: 460px; display: flex; flex-direction: column; gap: 8px; }
/* 表单两列对齐（表格样）：标签列固定 5em，输入列各行右缘对齐；
   控件去全局 320px 上限并锁定 32px 高，与相邻输入框完全等高 */
label { display: grid; grid-template-columns: 5em 1fr; gap: 8px; align-items: center; font-size: 13px; }
input:not([type="checkbox"]):not([type="radio"]), select { max-width: none; height: var(--control-h); }
/* 分支方式行与普通 label 同构两列；单选组与行内复选框不受两列网格约束 */
.field-row { display: grid; grid-template-columns: 5em 1fr; gap: 8px; align-items: center; font-size: 13px; }
.radios { display: flex; gap: 16px; }
/* 全局 input 规则（width:100%/min-height:32px）会把 radio 撑成大控件：
   与 checkbox 同款显式压缩成 15px 圆点（base.css 只豁免了 checkbox） */
input[type="radio"] {
  min-height: 0;
  width: 15px;
  height: 15px;
  padding: 0;
  margin: 0;
  accent-color: var(--primary);
}
/* 单选文字不吃控件色：label 的 grid/flex 布局下文字直接继承 --fg */
label.inline { display: flex; align-items: center; gap: 6px; font-size: 13px; color: var(--fg); white-space: nowrap; }
/* 模块勾选列表仍是行内 checkbox 行，不进两列网格 */
fieldset label { display: flex; align-items: center; gap: 6px; }
/* 滚动叶子区：contain 防模块列表滚到底后滚动链穿透 */
fieldset { max-height: 160px; overflow: auto; overscroll-behavior: contain; }
.actions { display: flex; gap: 8px; justify-content: flex-end; }
.muted { color: var(--fg-muted); font-size: 13px; margin: 0; }
.err { color: var(--danger-text); font-size: 13px; margin: 0; }
</style>
