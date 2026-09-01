import { defineStore } from "pinia";
import { ref } from "vue";
import { runGws } from "../lib/gws-bridge";
import { normalizePath } from "../lib/path";
import { parseLs, parseRepoLs, parseEnvLs, type WsEntry, type RepoEntry } from "../lib/parse";

export const useHubStore = defineStore("hub", () => {
  const path = ref("");
  const workspaces = ref<WsEntry[]>([]);
  const repos = ref<RepoEntry[]>([]);
  const envs = ref<string[]>([]);
  const error = ref("");

  function setHub(p: string) {
    // hub 路径唯一入口（lastHub 回放/手输/新建）：进来先规范化，
    // 混合分隔符（C:\a\/b）在显示、复制、拼接处都会别扭，源头一次清掉
    path.value = normalizePath(p);
    workspaces.value = [];
    repos.value = [];
    envs.value = [];
    error.value = "";
  }

  async function refreshAll() {
    if (!path.value) return;
    try {
      const [lsOut, repoOut, envOut] = await Promise.all([
        runGws(["ls"], path.value),
        runGws(["repo", "ls"], path.value),
        runGws(["env", "ls"], path.value),
      ]);
      // 仅 code===null（spawn 失败）算错误；非零退出码不视为失败（env ls 有环境时退出码为 1 属正常）
      for (const out of [lsOut, repoOut, envOut]) {
        if (out.code === null) {
          error.value = out.output;
          return;
        }
      }
      workspaces.value = parseLs(lsOut.output);
      repos.value = parseRepoLs(repoOut.output);
      envs.value = parseEnvLs(envOut.output);
      error.value = "";
    } catch (e) {
      error.value = String(e);
    }
  }

  return { path, workspaces, repos, envs, error, setHub, refreshAll };
});
