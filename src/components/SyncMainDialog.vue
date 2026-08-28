<script setup lang="ts">
import { computed, ref } from "vue";

/** defaultFrom：来源预填值（当前无法得知工作区创建时基线，一般不传） */
const props = defineProps<{ defaultFrom?: string }>();
const emit = defineEmits<{ (e: "close"): void; (e: "run", from: string): void }>();
const from = ref(props.defaultFrom ?? "");
/** 多来源归一：gws 的 --from 值支持逗号分隔链（resolve_chain_base 按逗号拆、
 *  顺序即优先级、全不存在自动兜底主干）；空格/逗号输入统一拼成 a,b */
const fromValue = computed(() => from.value.split(/[\s,]+/).filter(Boolean).join(","));

function start() {
  // 纯空格视同留空（创建时基线）：不归一会把 --from "  " 传给 gws 导致 rev-parse 失败
  emit("run", fromValue.value);
  emit("close");
}
</script>

<template>
  <div class="mask" @click.self="emit('close')">
    <div class="dialog">
      <h3>同步最新代码</h3>
      <p class="muted">把远程基线最新提交合进当前需求分支</p>
      <label>来源 <input v-model="from" autocapitalize="off" spellcheck="false" placeholder="来源/ref，可多个（空格/逗号分隔，按优先级取），留空 = 创建时基线" /></label>
      <div class="actions">
        <button @click="emit('close')">取消</button>
        <button class="primary" @click="start">开始同步</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mask { position: fixed; inset: 0; background: var(--mask); display: flex; align-items: center; justify-content: center; z-index: 100; }
.dialog { background: var(--bg-soft); border: 1px solid var(--border); box-shadow: var(--shadow); border-radius: 8px; padding: 20px; width: 460px; display: flex; flex-direction: column; gap: 8px; }
/* 表单两列对齐（表格样）：标签列固定 5em；控件去全局 320px 上限并锁定 32px 高 */
label { display: grid; grid-template-columns: 5em 1fr; gap: 8px; align-items: center; font-size: 13px; }
input { max-width: none; height: var(--control-h); }
.actions { display: flex; gap: 8px; justify-content: flex-end; }
.muted { color: var(--fg-muted); font-size: 13px; margin: 0; }
</style>
