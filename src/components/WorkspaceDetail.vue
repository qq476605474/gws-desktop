<script setup lang="ts">
defineProps<{ name: string }>();
const emit = defineEmits<{ (e: "close"): void }>();
</script>

<template>
  <div class="detail">
    <div class="head">
      <button @click="emit('close')">← 返回列表</button>
      <h3>{{ name }}</h3>
    </div>
    <p class="muted">工作区详情（任务 10 实装）</p>
  </div>
</template>

<style scoped>
.head { display: flex; align-items: center; gap: 12px; margin-bottom: 8px; }
.muted { color: #888; font-size: 13px; }
</style>
