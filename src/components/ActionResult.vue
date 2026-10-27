<script setup lang="ts">
//! 操作结果面板：**可滚动的**文本清单。
//!
//! 实测反馈 #6：原来把结果拼成一段长文本塞进对话框，没有换行、把窗口撑破了。
//! 这里用 `<pre>` 保换行 + 限高内滚，长清单也不会挤破布局。
//!
//! 实测反馈 #8：进度一行行蹦出来时，视图**停在顶部**——最新那条在下面看不见，
//! 人以为卡住了。这里在每次内容变化后滚到底。用户手动往上翻过就先不打扰
//! （见 `stickToBottom`），否则一行一行地把自己拽回去，反而看不了历史。

import { nextTick, ref, watch } from "vue";

const props = defineProps<{
  lines: string[];
  title?: string;
  /**
   * 填满父容器剩余高度。
   *
   * 用在高度固定的弹窗里（如同步弹窗）：结果区高度定死，弹窗就不会随着
   * 输出一行行长高（实测反馈 #8）。
   */
  fill?: boolean;
}>();

const body = ref<HTMLElement | null>(null);
/** 是否跟随到底部。用户往上滚了就不跟了，滚回底部再自动跟上。 */
const stickToBottom = ref(true);

function onScroll() {
  const el = body.value;
  if (!el) return;
  // 距底部 24px 以内都算「在底部」——像素级相等太苛刻，滚动本身有误差
  stickToBottom.value = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
}

watch(
  () => props.lines,
  async () => {
    if (!stickToBottom.value) return;
    await nextTick(); // 等 DOM 长高之后再滚，否则滚到的是旧高度
    const el = body.value;
    if (el) el.scrollTop = el.scrollHeight;
  },
  { deep: true },
);
</script>

<template>
  <div class="result" :class="{ fill }">
    <div class="head">
      <span class="title">{{ title ?? "结果" }}</span>
    </div>
    <pre ref="body" class="body" @scroll="onScroll">{{ lines.join("\n") }}</pre>
  </div>
</template>

<style scoped>
.result {
  border: 1px solid var(--xg-border);
  border-radius: 6px;
  margin-bottom: 12px;
  overflow: hidden;
  /* 纵向 flex，好让 fill 时正文能撑满剩余高度 */
  display: flex;
  flex-direction: column;
}
/* fill：作为 flex 子项吃掉父容器剩余高度。flex 属性写在自己身上，
   父组件的 scoped 作用域不影响它。 */
.result.fill {
  flex: 1;
  min-height: 0;
  margin-bottom: 0;
}
.head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 6px 10px;
  background: rgba(128, 128, 128, 0.06);
  border-bottom: 1px solid var(--xg-border);
  flex: none;
}
.title {
  font-size: 12px;
  color: #888;
}
/* 这里原来有个「关闭」按钮，点了没反应：它 emit 的 close 事件在任何调用点
   都没被接（DangerDialog / SyncDialog 都只用 :lines）。而它们各自的弹窗页脚
   本来就有「关闭」。所以直接删掉——不去补一个重复的关闭入口。 */
.body {
  margin: 0;
  padding: 10px 12px;
  max-height: 260px;
  overflow: auto;
  font-family: var(--mono);
  font-size: 12px;
  line-height: 1.7;
  white-space: pre-wrap;
  word-break: break-all;
}
/* fill 时不限高，改由父容器定高——高度定死才不会「越输出越高」 */
.result.fill .body {
  flex: 1;
  min-height: 0;
  max-height: none;
}
</style>