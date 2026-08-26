import { defineStore } from "pinia";
import { ref } from "vue";
import { runGws } from "../lib/gws-bridge";
import { parseLs, parseRepoLs, parseEnvLs, type WsEntry, type RepoEntry } from "../lib/parse";

export const useHubStore = defineStore("hub", () => {
  const path = ref("");
  const workspaces = ref<WsEntry[]>([]);
  const repos = ref<RepoEntry[]>([]);
  const envs = ref<string[]>([]);

  function setHub(p: string) {
    path.value = p;
  }

  async function refreshAll() {
    if (!path.value) return;
    const [lsOut, repoOut, envOut] = await Promise.all([
      runGws(["ls"], path.value),
      runGws(["repo", "ls"], path.value),
      runGws(["env", "ls"], path.value),
    ]);
    workspaces.value = parseLs(lsOut.output);
    repos.value = parseRepoLs(repoOut.output);
    envs.value = parseEnvLs(envOut.output);
  }

  return { path, workspaces, repos, envs, setHub, refreshAll };
});
