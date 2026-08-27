<script setup lang="ts">
import { computed, ref } from "vue";
import { useHubStore } from "../stores/hub";
import { useCmdStore } from "../stores/cmd";

/** existing：该工作区已有模块名——这些模块不再列为候选（gws add 重复添加只会报错/重复，不友好） */
const props = defineProps<{ wsPath: string; existing: string[] }>();
const emit = defineEmits<{ (e: "close"): void; (e: "added"): void }>();
const hub = useHubStore();
const cmd = useCmdStore();
const selected = ref<string[]>([]);
const submitting = ref(false);
const err = ref("");
const progress = ref("");
const candidates = computed(() => hub.repos.filter((r) => !props.existing.includes(r.name)));

async function add() {
  if (submitting.value) return;
  submitting.value = true;
  err.value = "";
  // 整段序列持有命令弹窗（hold 计数>0 时关闭禁用）：逐模块 add 期间弹窗不消失，
  // 避免模块间隙弹窗被关掉后后续命令无输出可见；release 放 finally 保证异常路径也释放
  cmd.holdDialog();
  const failed: string[] = [];
  try {
    const total = selected.value.length;
    for (let i = 0; i < total; i++) {
      const m = selected.value[i];
      progress.value = `正在添加 ${m}（${i + 1}/${total}）…`;
      const run = await cmd.execDialog(`gws add ${m}`, ["add", m], props.wsPath);
      await cmd.waitDone(run); // 逐个等结束，避免并发 worktree 操作
      // 模块间独立：单个失败不中断循环，继续尝试后续模块
      if (run.state !== "done") failed.push(m);
    }
    if (failed.length) {
      // 部分失败：不 emit added、不关弹窗（列表刷新留给用户重试成功或手动取消后）；
      // 从 selected 剔除已成功项，弹窗停留在失败项上，用户可直接重试
      err.value = `部分模块添加失败：${failed.join("、")}（详见命令弹窗）`;
      selected.value = failed;
      return;
    }
    emit("added");
    emit("close");
  } catch (e) {
    err.value = String(e);
  } finally {
    cmd.releaseDialog();
    submitting.value = false;
  }
}
</script>

<template>
  <div class="mask" @click.self="!submitting && emit('close')">
    <div class="dialog">
      <h3>增加模块</h3>
      <label v-for="r in candidates" :key="r.name">
        <input type="checkbox" :value="r.name" v-model="selected" :disabled="submitting" /> {{ r.name }}
      </label>
      <!-- 有仓库但全被过滤成空列表时给出明确空态，避免一个“空白弹窗” -->
      <p v-if="hub.repos.length && !candidates.length" class="empty">该工作区已包含所有模块</p>
      <p v-if="progress" class="progress">{{ progress }}</p>
      <p v-if="err" class="err">{{ err }}</p>
      <div class="actions">
        <button :disabled="submitting" @click="emit('close')">取消</button>
        <button class="primary" :disabled="!selected.length || submitting" @click="add">添加</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mask { position: fixed; inset: 0; background: var(--mask); display: flex; align-items: center; justify-content: center; z-index: 100; }
/* overscroll contain：模块多时整卡滚动，防滚动链穿透到背后页面 */
.dialog { background: var(--bg-soft); border: 1px solid var(--border); box-shadow: var(--shadow); border-radius: 8px; padding: 20px; width: 380px; display: flex; flex-direction: column; gap: 8px; max-height: 80vh; overflow: auto; overscroll-behavior: contain; }
label { display: flex; align-items: center; gap: 8px; font-size: 13px; }
.actions { display: flex; gap: 8px; justify-content: flex-end; }
.progress { color: var(--fg-muted); font-size: 13px; margin: 0; }
.empty { color: var(--fg-muted); font-size: 13px; margin: 0; }
.err { color: var(--danger-text); font-size: 13px; margin: 0; }
</style>
