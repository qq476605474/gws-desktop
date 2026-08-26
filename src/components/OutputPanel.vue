<script setup lang="ts">
import { useCmdStore } from "../stores/cmd";
import { ansiToHtml } from "../lib/ansi";

const cmd = useCmdStore();
</script>

<template>
  <section v-if="cmd.current" class="panel">
    <header>
      <span>{{ cmd.current.label }}</span>
      <span class="state" :class="cmd.current.state">{{ cmd.current.state }}</span>
    </header>
    <pre v-html="ansiToHtml(cmd.current.output)"></pre>
  </section>
</template>

<style scoped>
.panel { border-top: 1px solid var(--border); max-height: 220px; display: flex; flex-direction: column; }
header { padding: 4px 12px; font-size: 12px; color: var(--fg-muted); display: flex; justify-content: space-between; }
/* pre 前景 #eee：终端面板恒深底（--bg-panel）恒浅字，三主题一致，无对应 CSS 变量 */
pre { margin: 0; padding: 8px 12px; font-size: 12px; line-height: 1.5; overflow: auto; background: var(--bg-panel); color: #eee; flex: 1; }
.state.running { color: var(--primary); } .state.done { color: var(--ok); } .state.failed { color: var(--danger); }
</style>
