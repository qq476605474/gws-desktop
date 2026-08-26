<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useHubStore } from "../../stores/hub";
import { useCmdStore } from "../../stores/cmd";
import PathActions from "../PathActions.vue";
import NewWorkspaceDialog from "../NewWorkspaceDialog.vue";
import GetWorkspaceDialog from "../GetWorkspaceDialog.vue";
import WorkspaceDetail from "../WorkspaceDetail.vue";

const hub = useHubStore();
const cmd = useCmdStore();
const showNew = ref(false);
const showGet = ref(false);
const detail = ref<string | null>(null);

onMounted(() => hub.refreshAll());
</script>

<template>
  <div>
    <div class="toolbar">
      <button class="primary" @click="showNew = true">+ 新建需求</button>
      <button @click="showGet = true">⇄ gws get 拉取</button>
      <button :disabled="cmd.isRunning()" @click="hub.refreshAll()">刷新</button>
    </div>
    <p v-if="hub.error" class="error">{{ hub.error }}</p>
    <div v-for="ws in hub.workspaces" :key="ws.name" class="ws-row" @click="detail = ws.name">
      <div class="ws-main">
        <strong>{{ ws.name }}</strong>
        <span class="muted">{{ ws.title }}</span>
        <code class="branch">{{ ws.branch }}</code>
      </div>
      <PathActions :path="`${hub.path}/ws/${ws.name}`" />
    </div>
    <p v-if="!hub.workspaces.length && !hub.error" class="muted">(暂无工作区)</p>
    <WorkspaceDetail v-if="detail" :name="detail" @close="detail = null" />
    <NewWorkspaceDialog v-if="showNew" @close="showNew = false" @created="hub.refreshAll()" />
    <GetWorkspaceDialog v-if="showGet" @close="showGet = false" @created="hub.refreshAll()" />
  </div>
</template>

<style scoped>
.toolbar { display: flex; gap: 8px; margin-bottom: 12px; }
.ws-row { display: flex; justify-content: space-between; align-items: center; border: 1px solid #e0e0e0; border-radius: 6px; padding: 8px 12px; margin-bottom: 6px; cursor: pointer; }
.ws-row:hover { background: #fafafa; }
.ws-main { display: flex; align-items: center; gap: 10px; min-width: 0; }
.branch { background: #f0f0f0; padding: 1px 6px; border-radius: 3px; font-size: 12px; }
.muted { color: #888; font-size: 12px; }
.error { color: #c62828; font-size: 13px; }
.primary { background: #1565c0; color: #fff; }
</style>
