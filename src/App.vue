<script setup lang="ts">
import { onMounted, ref } from "vue";
import { currentView } from "./router";
import { useSettingsStore } from "./stores/settings";
import StartupView from "./views/StartupView.vue";
import MainView from "./views/MainView.vue";
import ConfirmDialog from "./components/ConfirmDialog.vue";

const settings = useSettingsStore();
// WHY ready 门控：子组件 onMounted 先于父组件的异步 init 完成，直接渲染会让 StartupView 读到空 lastHub，上次 hub 自动进入失效
const ready = ref(false);
onMounted(async () => {
  await settings.init().catch((e) => console.warn("设置加载失败，使用默认值", e));
  ready.value = true;
});
</script>

<template>
  <template v-if="ready">
    <StartupView v-if="currentView === 'startup'" />
    <MainView v-else />
  </template>
  <ConfirmDialog />
</template>
