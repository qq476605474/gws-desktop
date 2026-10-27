<script setup lang="ts">
//! 左上角的**当前版本号**：一直安静地显示，不抢标题。
//!
//! 更新提醒单独由 `UpdatePill` 负责（只在有新版时出一条单行提示）——
//! 以前两者挤在一个大胶囊里，有新版时版本号反而看不见了。

import { onMounted, ref } from "vue";
import { getVersion } from "@tauri-apps/api/app";

const version = ref("—");

onMounted(async () => {
  try {
    version.value = await getVersion();
  } catch {
    version.value = "—";
  }
});
</script>

<template>
  <span class="version-badge" :title="`当前版本 ${version}`">v{{ version }}</span>
</template>

<style scoped>
.version-badge {
  flex: none;
  color: #aaa;
  font-size: 11px;
  font-variant-numeric: tabular-nums;
  line-height: 1;
}
</style>
