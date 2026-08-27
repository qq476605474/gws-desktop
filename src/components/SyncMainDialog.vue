<script setup lang="ts">
import { ref } from "vue";

/** defaultFrom：来源预填值（当前无法得知工作区创建时基线，一般不传） */
const props = defineProps<{ defaultFrom?: string }>();
const emit = defineEmits<{ (e: "close"): void; (e: "run", from: string): void }>();
const from = ref(props.defaultFrom ?? "");

function start() {
  // 执行由父组件接手（confirm + 命令弹窗）：run 后随即 close，本弹窗不留驻也无在途态可防重入
  emit("run", from.value);
  emit("close");
}
</script>

<template>
  <div class="mask" @click.self="emit('close')">
    <div class="dialog">
      <h3>同步最新代码</h3>
      <p class="muted">把远程基线最新提交合进当前工作区分支</p>
      <label>来源 <input v-model="from" autocapitalize="off" spellcheck="false" placeholder="来源分支/ref，留空 = 创建时基线（默认主干）" /></label>
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
label { display: flex; align-items: center; gap: 8px; font-size: 13px; }
input { flex: 1; }
.actions { display: flex; gap: 8px; justify-content: flex-end; }
.muted { color: var(--fg-muted); font-size: 13px; margin: 0; }
</style>
