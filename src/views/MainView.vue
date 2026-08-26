<script setup lang="ts">
import { ref } from "vue";
import TopBar from "../components/TopBar.vue";
import WorkspacesTab from "../components/tabs/WorkspacesTab.vue";
import ReposTab from "../components/tabs/ReposTab.vue";
import EnvsTab from "../components/tabs/EnvsTab.vue";
import DocsTab from "../components/tabs/DocsTab.vue";
import OutputPanel from "../components/OutputPanel.vue";
import AboutDialog from "../components/AboutDialog.vue";

const tab = ref<"ws" | "repos" | "envs" | "docs">("ws");
const showAbout = ref(false);
</script>

<template>
  <div class="main">
    <TopBar v-model:tab="tab" @open-about="showAbout = true" />
    <div class="body">
      <WorkspacesTab v-if="tab === 'ws'" />
      <ReposTab v-else-if="tab === 'repos'" />
      <EnvsTab v-else-if="tab === 'envs'" />
      <DocsTab v-else />
    </div>
    <OutputPanel />
    <AboutDialog v-if="showAbout" @close="showAbout = false" />
  </div>
</template>

<style scoped>
.main { display: flex; flex-direction: column; height: 100vh; }
.body { flex: 1; overflow: auto; padding: 12px 16px; }
</style>
