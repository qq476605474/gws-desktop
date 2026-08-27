<script setup lang="ts">
// 纯展示查看器：文件内容由父组件读取完成后经 props 传入，读取与错误处理都不在此组件
defineProps<{ fileName: string; path: string; content: string }>();
const emit = defineEmits<{ (e: "close"): void }>();
</script>

<template>
  <!-- 只读无破坏性：点 mask 即关（对照 CmdDialog 须手动关闭的破坏性语义）；z-index 100（表单弹窗层级） -->
  <div class="mask" @click.self="emit('close')">
    <div class="dialog">
      <header>
        <div class="heading">
          <span class="name">{{ fileName }}</span>
          <span class="path">{{ path }}</span>
        </div>
        <button @click="emit('close')">关闭</button>
      </header>
      <!-- pre-wrap：保留换行与缩进的同时折长行，避免横向滚动 -->
      <pre>{{ content }}</pre>
    </div>
  </div>
</template>

<style scoped>
.mask { position: fixed; inset: 0; background: var(--mask); display: flex; align-items: center; justify-content: center; z-index: 100; }
/* 对齐 CmdDialog：上限 70vh，内容区独立滚动；overscroll contain 防滚动穿透到外层 */
.dialog { background: var(--bg-soft); border: 1px solid var(--border); box-shadow: var(--shadow); border-radius: 8px; width: 720px; max-width: calc(100vw - 48px); max-height: 70vh; display: flex; flex-direction: column; }
header { display: flex; justify-content: space-between; align-items: center; gap: 8px; padding: 12px 16px 8px; }
.heading { display: flex; flex-direction: column; min-width: 0; }
.name { font-weight: 600; font-size: 14px; word-break: break-all; }
.path { color: var(--fg-muted); font-size: 12px; word-break: break-all; }
pre { margin: 0; padding: 10px 16px 12px; font-size: 13px; line-height: 1.6; overflow-y: auto; overscroll-behavior: contain; white-space: pre-wrap; word-break: break-word; flex: 1; min-height: 0; }
</style>
