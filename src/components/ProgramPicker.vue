<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { NAlert, NButton, NEmpty, NInput, NModal, NSpin } from "naive-ui";

import * as api from "../bridge";
import type { InstalledApp } from "../types";

/**
 * 「选择程序」= 本机已安装程序的列表。
 *
 * 原来的做法是打开系统文件对话框：Windows / Linux 都落在「文档」或程序目录，
 * 一眼看不到几个 App（实测反馈「根本很难用」）。这里直接列出应用列表 ——
 * macOS 的 `/Applications`、Windows 的开始菜单、Linux 的 `.desktop` 都是
 * 用户心里「装了哪些程序」的样子。列表里没有的再从「浏览文件…」挑。
 */
const emit = defineEmits<{
  (e: "pick", app: InstalledApp): void;
  (e: "browse"): void;
  (e: "close"): void;
}>();

const apps = ref<InstalledApp[]>([]);
const query = ref("");
const loading = ref(true);
const err = ref("");

const filtered = computed(() => {
  const q = query.value.trim().toLowerCase();
  return q ? apps.value.filter((a) => a.name.toLowerCase().includes(q)) : apps.value;
});

onMounted(async () => {
  try {
    apps.value = await api.installedApps();
  } catch (e) {
    err.value = api.errText(e);
  } finally {
    loading.value = false;
  }
});
</script>

<template>
  <NModal
    :show="true"
    @update:show="(v: boolean) => !v && emit('close')"
    preset="card"
    title="选择程序"
    style="width: 520px"
    @close="emit('close')"
  >
    <NInput
      v-model:value="query"
      placeholder="搜索程序名，如 IDEA / VS Code"
      clearable
      :input-props="{ autocomplete: 'off' }"
    />
    <NSpin :show="loading" class="spin">
      <div class="apps">
        <button
          v-for="a in filtered"
          :key="a.name + a.command"
          type="button"
          class="app"
          @click="emit('pick', a)"
        >
          {{ a.name }}
        </button>
        <NEmpty
          v-if="!loading && !filtered.length"
          :description="apps.length ? '没有匹配的程序' : '没找到已安装的程序'"
          size="small"
        />
      </div>
    </NSpin>
    <NAlert v-if="err" type="error" :bordered="false" class="mt">{{ err }}</NAlert>

    <template #footer>
      <div class="footer">
        <span class="muted small">列表里没有？</span>
        <NButton size="small" @click="emit('browse')">浏览文件…</NButton>
      </div>
    </template>
  </NModal>
</template>

<style scoped>
.spin {
  display: block;
  margin-top: 12px;
}
.apps {
  max-height: 46vh;
  min-height: 96px;
  overflow: auto;
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding-right: 4px;
}
.app {
  display: block;
  width: 100%;
  padding: 7px 10px;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: pointer;
}
.app:hover {
  background: var(--xg-fill-soft);
}
.footer {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
}
.small {
  font-size: 12px;
}
.mt {
  margin-top: 12px;
}
</style>