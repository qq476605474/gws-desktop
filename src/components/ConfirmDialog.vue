<script setup lang="ts">
import { useCmdStore } from "../stores/cmd";
const cmd = useCmdStore();
</script>

<template>
  <div v-if="cmd.confirmPending" class="mask">
    <div class="dialog">
      <h3>gws 请求确认</h3>
      <pre>{{ cmd.confirmPending.question }}</pre>
      <div class="actions">
        <button class="danger" @click="cmd.answerConfirm(false)">取消（终止命令）</button>
        <button class="primary" @click="cmd.answerConfirm(true)">继续 (y)</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mask { position: fixed; inset: 0; background: rgba(0,0,0,.35); display: flex; align-items: center; justify-content: center; z-index: 99; }
.dialog { background: #fff; border-radius: 8px; padding: 20px; max-width: 520px; }
pre { white-space: pre-wrap; background: #fafafa; padding: 10px; }
.actions { display: flex; gap: 8px; justify-content: flex-end; margin-top: 12px; }
.danger { color: #c62828; } .primary { background: #1565c0; color: #fff; }
</style>
