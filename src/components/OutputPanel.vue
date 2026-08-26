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
.panel { border-top: 1px solid #e0e0e0; max-height: 220px; display: flex; flex-direction: column; }
header { padding: 4px 12px; font-size: 12px; color: #666; display: flex; justify-content: space-between; }
pre { margin: 0; padding: 8px 12px; font-size: 12px; line-height: 1.5; overflow: auto; background: #1e1e1e; color: #eee; flex: 1; }
.state.running { color: #1565c0; } .state.done { color: #2e7d32; } .state.failed { color: #c62828; }
</style>
