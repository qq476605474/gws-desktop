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
    <button class="btn-sm" title="复制路径" @click="copy">📋</button>
    <button class="btn-sm" title="在访达中打开" @click="finder">📂</button>
    <button class="btn-sm" title="在终端中打开" @click="terminal">💻</button>
  </span>
</template>

<style scoped>
/* flex-shrink:0：所在行（表格末列/行卡片）空间不足时不压缩本组，否则按钮被挤变形 */
.pa { display: inline-flex; gap: 4px; flex-shrink: 0; }
</style>
