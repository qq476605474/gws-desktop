<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { currentView } from "./router";
import { useSettingsStore } from "./stores/settings";
import { useHubStore } from "./stores/hub";
import { busyCount } from "./lib/busy";
import StartupView from "./views/StartupView.vue";
import MainView from "./views/MainView.vue";
import ConfirmDialog from "./components/ConfirmDialog.vue";
import ConfirmBox from "./components/ConfirmBox.vue";
import CmdDialog from "./components/CmdDialog.vue";
import ToastList from "./components/ToastList.vue";

const settings = useSettingsStore();
const hub = useHubStore();
// 切换 hub（SwitchHubDialog）时 MainView 整体重挂载：各 Tab 的 onMounted 取数重跑，
// 旧 hub 的列表/详情/展开状态随之丢弃——与「回 startup 重进」等价但无闪屏
const mainKey = computed(() => hub.path);
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
    <MainView v-else :key="mainKey" />
  </template>
  <!-- z-index 层级：busy 遮罩 90 < 表单弹窗 100 < CmdDialog 150 < ConfirmDialog 200 < ConfirmBox 210 -->
  <div v-if="busyCount > 0" class="busy-mask"><span class="busy-spinner"></span>加载中…</div>
  <CmdDialog />
  <ConfirmDialog />
  <ConfirmBox />
  <ToastList />
</template>

<style scoped>
/* 数据刷新类（st/ls/doc ls 等取数）的全局加载遮罩：全屏挡操作，非命令弹窗形态 */
.busy-mask { position: fixed; inset: 0; background: var(--mask); z-index: 90; display: flex; align-items: center; justify-content: center; gap: 8px; color: var(--fg); font-size: 14px; }
.busy-spinner { width: 14px; height: 14px; border: 2px solid var(--border); border-top-color: var(--primary); border-radius: 50%; animation: busy-spin 0.8s linear infinite; }
@keyframes busy-spin { to { transform: rotate(360deg); } }
</style>
