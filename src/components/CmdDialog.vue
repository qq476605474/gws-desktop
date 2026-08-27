<script setup lang="ts">
import { useCmdStore } from "../stores/cmd";
import { respondConfirm } from "../lib/gws-bridge";
import { ansiToHtml } from "../lib/ansi";

const cmd = useCmdStore();

/** 终止运行中的命令（持续输出型命令的逃生口）：respondConfirm(id,false) 在 Rust 侧
 *  走 kill 分支（不依赖是否真有 pending confirm），被杀进程 exit code 为 null →
 *  run 进入 failed 终态、关闭按钮恢复可用。不看 holdCount：AddModuleDialog 序列中
 *  用户也应能终止（被终止模块落入失败收集、序列继续——既有行为）。 */
async function terminate() {
  const run = cmd.dialogRun;
  if (!run || (run.state !== "running" && run.state !== "confirm")) return;
  try {
    await respondConfirm(run.id, false);
  } catch {
    // run 已被后端清理等罕见竞态：exit 事件已送达或即将送达，弹窗终态不受影响
  }
}
</script>

<template>
  <!-- z-index 层级约定：busy 遮罩 90 < 表单弹窗 100 < 本弹窗(CmdDialog) 150 < ConfirmDialog 200。
       mask 不可点击关闭：命令结束后须手动点关闭按钮才能继续操作（用户明确要求）。 -->
  <div v-if="cmd.dialogRun" class="mask">
    <div class="dialog">
      <header>
        <span class="label">{{ cmd.dialogRun.label }}</span>
        <span class="state" :class="cmd.dialogRun.state">
          <template v-if="cmd.dialogRun.state === 'done'">✓ 完成</template>
          <template v-else-if="cmd.dialogRun.state === 'failed'">✗ 失败</template>
          <template v-else><span class="spinner"></span>执行中…</template>
        </span>
      </header>
      <pre v-html="ansiToHtml(cmd.dialogRun.output)"></pre>
      <footer>
        <button v-if="cmd.dialogRun.state === 'running' || cmd.dialogRun.state === 'confirm'"
                class="danger" @click="terminate">终止</button>
        <button class="primary" :disabled="cmd.isRunning() || cmd.holdCount > 0" @click="cmd.closeDialog()">关闭</button>
      </footer>
    </div>
  </div>
</template>

<style scoped>
.mask { position: fixed; inset: 0; background: var(--mask); display: flex; align-items: center; justify-content: center; z-index: 150; }
/* 输出区固定高度：弹窗总高恒定，避免输出增多时弹窗从矮到高慢慢撑开（视觉晃动） */
.dialog { background: var(--bg-soft); border: 1px solid var(--border); box-shadow: var(--shadow); border-radius: 8px; width: 640px; max-width: calc(100vw - 48px); display: flex; flex-direction: column; }
header { display: flex; justify-content: space-between; align-items: center; gap: 8px; padding: 12px 16px 8px; font-size: 13px; }
.label { font-weight: 600; word-break: break-all; }
.state { display: inline-flex; align-items: center; gap: 6px; flex-shrink: 0; }
.state.running, .state.confirm { color: var(--primary); }
.state.done { color: var(--ok-text); }
.state.failed { color: var(--danger-text); }
.spinner { width: 12px; height: 12px; border: 2px solid var(--border); border-top-color: var(--primary); border-radius: 50%; animation: cmd-spin 0.8s linear infinite; }
@keyframes cmd-spin { to { transform: rotate(360deg); } }
/* pre 前景 #eee：终端输出恒深底（--bg-panel）恒浅字，三主题一致，无对应 CSS 变量（沿用 OutputPanel）。
   固定 300px：输出在窗口内滚动（含空输出期），弹窗高度恒定；overscroll contain 防滚动穿透 */
pre { margin: 0; padding: 10px 16px; font-size: 12px; line-height: 1.5; overflow-y: auto; overscroll-behavior: contain; background: var(--bg-panel); color: #eee; height: 300px; }
footer { display: flex; justify-content: flex-end; gap: 8px; padding: 8px 16px 12px; }
</style>
