<script setup lang="ts">
import { answerConfirm, confirmState } from "../lib/confirm";
</script>

<template>
  <div v-if="confirmState.pending" class="mask" @click.self="answerConfirm(false)">
    <div class="dialog">
      <h3>确认操作</h3>
      <p class="msg">{{ confirmState.pending.message }}</p>
      <div class="actions">
        <button @click="answerConfirm(false)">取消</button>
        <button class="primary" @click="answerConfirm(true)">确认</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 须盖在所有业务弹窗之上（busy 90 < 表单 100 < CmdDialog 150 < gws 确认 200 < 本框） */
.mask { position: fixed; inset: 0; background: var(--mask); display: flex; align-items: center; justify-content: center; z-index: 210; }
.dialog { background: var(--bg-soft); border: 1px solid var(--border); box-shadow: var(--shadow); border-radius: 8px; padding: 20px; width: 400px; display: flex; flex-direction: column; gap: 12px; }
h3 { margin: 0; }
.msg { margin: 0; font-size: 13px; line-height: 1.6; white-space: pre-wrap; }
.actions { display: flex; gap: 8px; justify-content: flex-end; }
</style>
