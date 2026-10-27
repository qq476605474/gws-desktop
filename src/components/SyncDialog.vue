<script setup lang="ts">
//! 同步对话框：**选范围 → 同步 → 看结果**。
//!
//! 实测反馈：原来点「同步最新代码」就是全量跑（主干 + 所有环境），
//! 既慢又不是你要的。现在按上下文分：
//! - 仓库页 → 只同步主干
//! - 环境页某个环境 → 只同步那个环境
//! 并且**每次都让你先选模块**，不选就是全部。

import { ref, watch } from "vue";
import { NButton, NModal, useMessage } from "naive-ui";

import ModulePicker from "./ModulePicker.vue";
import ActionResult from "./ActionResult.vue";
import { withBusy } from "../busy";

const props = defineProps<{
  title: string;
  hint?: string;
  /** 待选模块（仓库名）。 */
  names: string[];
  /** 每行的说明（如「主干 main」「环境 pre → pre-202609」）。 */
  hints?: Record<string, string>;
  /** 执行同步。返回结果行。 */
  action: (only: string[]) => Promise<string[]>;
  /** 预勾选的模块。空 = 全选。 */
  preselect?: string[] | null;
}>();

const emit = defineEmits<{ (e: "close"): void; (e: "done"): void }>();
const msg = useMessage();

const picked = ref<string[]>([]);
const result = ref<string[] | null>(null);
const running = ref(false);

// 默认全选：不选 ≠ 全选，得让用户明确知道这次会动哪些
watch(
  () => props.names,
  (n) => {
    if (result.value !== null) return;
    // 父组件指定了预勾就只勾那些，否则全选
    picked.value = props.preselect?.length
      ? n.filter((x) => props.preselect!.includes(x))
      : [...n];
  },
  { immediate: true },
);

/** 构造 ModulePicker 需要的形态。同步不关心改动/推送状态，说明走 `hints`。 */
const modules = () =>
  props.names.map((name) => ({
    module: name,
    present: true,
    dirty: 0,
    pushed: false,
    ahead: 0,
    behind: 0,
    ahead_of_main: null,
    error: null,
  }));

async function run() {
  if (running.value) return;
  if (!picked.value.length) {
    msg.warning("至少要选一个项目");
    return;
  }
  running.value = true;
  try {
    result.value = await withBusy(props.title, () => props.action(picked.value));
  } catch (e) {
    result.value = [`✗ ${String((e as { message?: string })?.message ?? e)}`];
  } finally {
    running.value = false;
  }
}
</script>

<template>
  <NModal :show="true" @update:show="(v: boolean) => !v && emit('close')"
    preset="card"
    :title="title"
    style="width: 660px"
    :mask-closable="!running"
    @close="emit('close')"
  >
    <!-- 主体高度定死（`.body-fixed`），弹窗就不再随输出一行行长高（实测反馈 #8）。
         不依赖 NModal 内部结构，纯靠自己的容器定高，换 Naive UI 版本也不受影响。 -->
    <div class="body-fixed">
      <p v-if="hint" class="hint">{{ hint }}</p>

      <div class="mid">
        <!-- 一开始执行就收起选择区（实测反馈 #24）：留着的话跑完还杵在那儿，
             人会以为还能再点一次。执行中给一行「正在同步哪些」，保留了知情权。 -->
        <template v-if="result === null">
          <div class="label">同步哪些项目（默认全部）</div>
          <ModulePicker v-model="picked" :modules="modules()" :hints="hints" />
        </template>
        <template v-else>
          <div class="ran">
            <span class="label">本次同步</span>
            <span class="mono">{{ picked.join("、") }}</span>
          </div>
          <ActionResult class="mt" :lines="result" title="同步结果" fill />
        </template>
      </div>
    </div>

    <template #footer>
      <div class="footer">
        <NButton :disabled="running" @click="emit('close')">{{ result ? "关闭" : "取消" }}</NButton>
        <NButton v-if="!result" type="primary" :loading="running" @click="run">
          同步（{{ picked.length }} 个）
        </NButton>
        <NButton
          v-else
          type="primary"
          @click="
            () => {
              emit('done');
              emit('close');
            }
          "
        >
          完成
        </NButton>
      </div>
    </template>
  </NModal>
</template>

<style scoped>
.hint {
  color: #888;
  font-size: 12px;
  margin: 0 0 12px;
  flex: none;
}
/* 主体定高：弹窗高度从打开那一刻就是确定的 */
.body-fixed {
  height: 420px;
  display: flex;
  flex-direction: column;
  min-height: 0;
}
/* 中间这块吃掉剩余高度；结果多长都在里面滚，不会把弹窗顶高 */
.mid {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
.mid :deep(.picker) {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
.mid :deep(.picker .list) {
  flex: 1;
  min-height: 0;
  max-height: none;
}
.label {
  font-size: 12px;
  color: #888;
  margin-bottom: 6px;
  flex: none;
}
.ran {
  display: flex;
  align-items: baseline;
  gap: 8px;
  font-size: 12px;
  flex: none;
}
.ran .label {
  margin-bottom: 0;
  white-space: nowrap;
}
.mt {
  margin-top: 12px;
}
.footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>