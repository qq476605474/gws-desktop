<script setup lang="ts">
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { openInFinder, openInTerminal } from "../lib/gws-bridge";
import { useSettingsStore } from "../stores/settings";

const props = defineProps<{ path: string }>();
const settings = useSettingsStore();
async function copy() { await writeText(props.path); }
async function finder() { await openInFinder(props.path); }
async function terminal() {
  await openInTerminal(props.path, settings.terminal === "system" ? null : settings.terminal);
}
</script>

<template>
  <span class="pa" @click.stop>
    <button title="复制路径" @click="copy">📋</button>
    <button title="在访达中打开" @click="finder">📂</button>
    <button title="在终端中打开" @click="terminal">💻</button>
  </span>
</template>

<style scoped>
.pa { display: inline-flex; gap: 4px; }
button { border: 1px solid var(--border); background: var(--bg-soft); border-radius: 4px; cursor: pointer; }
</style>
