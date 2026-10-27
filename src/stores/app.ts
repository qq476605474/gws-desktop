//! 应用状态：当前 hub、仓库/环境/需求列表、主题与偏好。

import { defineStore } from "pinia";

import { ref, watch } from "vue";
import { load, Store } from "@tauri-apps/plugin-store";

import * as api from "../bridge";
import type { EnvRow, HubInfo, RepoRow, WsRow } from "../types";

export type Theme = "light" | "dark";

export const useAppStore = defineStore("app", () => {
  const hub = ref<HubInfo | null>(null);
  /**
   * 「跳到仓库页的哪个标签」这类跨页意图。
   *
   * 总览的「环境」卡片要落到**环境**标签，但页签状态在 ReposView 内部 ——
   * 所以由一个 store 字段传递意图，ReposView 读走就清掉（一次性）。
   */
  const reposTab = ref<string | null>(null);
  const repos = ref<RepoRow[]>([]);
  const envs = ref<EnvRow[]>([]);
  const workspaces = ref<WsRow[]>([]);
  /** 列表加载失败的原因。空串 = 正常。 */
  const loadError = ref("");
  /**
   * 每次列表刷新成功 +1。
   *
   * 页面自己拉的数据（需求详情的 git 数字、文档树）不在 store 里，
   * 靠它当「该重读了」的信号 —— 否则窗口聚焦拉完列表，打开着的那一页还是旧内容
   * （Agent 里用 skill 改了需求/模块，界面看着像没反应）。
   */
  const refreshTick = ref(0);

  // ── 偏好（存本地文件，不进 hub） ──
  const theme = ref<Theme>("light");
  const terminal = ref<string>("system");
  /** 「用编辑器打开」用哪个：默认 IDEA（用户指定的默认值）。 */
  const editor = ref<string>("idea");
  /** 选「自定义命令」时的命令模板（`{path}` 占位）。 */
  const editorCommand = ref<string>("");
  let store: Store | null = null;
  let initPromise: Promise<void> | null = null;

  async function init(): Promise<void> {
    if (initPromise) return initPromise;
    initPromise = (async () => {
      store = await load("settings.json", { autoSave: true });
      theme.value = (await store.get<Theme>("theme")) ?? "light";
      terminal.value = (await store.get<string>("terminal")) ?? "system";
      editor.value = (await store.get<string>("editor")) ?? "idea";
      editorCommand.value = (await store.get<string>("editorCommand")) ?? "";
      watch([theme, terminal, editor, editorCommand], async () => {
        await store?.set("theme", theme.value);
        await store?.set("terminal", terminal.value);
        await store?.set("editor", editor.value);
        await store?.set("editorCommand", editorCommand.value);
      });
    })();
    try {
      await initPromise;
    } catch (e) {
      // 失败清缓存并 rethrow：允许后续重试，而不是永久缓存一个 rejected promise
      initPromise = null;
      throw e;
    }
  }

  function setHub(info: HubInfo | null) {
    hub.value = info;
    if (!info) {
      repos.value = [];
      envs.value = [];
      workspaces.value = [];
    }
  }

  /** 正在刷新（手动刷新按钮的转圈）。 */
  const refreshing = ref(false);
  /** 上次刷新完成的时间，用来给聚焦触发的刷新去抖。 */
  let refreshedAt = 0;
  let inflight: Promise<void> | null = null;

  /** 拉取 hub 下的三份列表。
   *
   * 三个请求并发；任一失败时**整体标记为错误**而不是显示半份数据——
   * 半份数据会让人以为「仓库就这么多」，比明确报错更误导。
   *
   * 并发调用共用同一个请求：窗口聚焦和页面自己的 `onMounted` 很容易撞在一起。
   */
  function refreshAll(): Promise<void> {
    if (!hub.value) return Promise.resolve();
    if (inflight) return inflight;
    refreshing.value = true;
    inflight = (async () => {
      try {
        const [r, e, w] = await Promise.all([api.repoList(), api.envList(), api.wsList()]);
        repos.value = r;
        envs.value = e;
        workspaces.value = w;
        loadError.value = "";
        // 概览页的数字要跟着更新，否则加完仓库那边还是旧数
        hub.value = await api.hubInfo();
        refreshTick.value += 1;
      } catch (err) {
        loadError.value = api.errText(err);
      } finally {
        refreshedAt = Date.now();
        refreshing.value = false;
        inflight = null;
      }
    })();
    return inflight;
  }

  /**
   * 窗口重新获得焦点时调用：隔得够久才真的刷新。
   *
   * 在 Paseo 与桌面端之间来回切会连着触发好几次聚焦事件，
   * 每次都打一遍 git 太重；2 秒内的重复直接忽略。
   */
  function refreshIfStale(minMs = 2000): void {
    if (!hub.value) return;
    if (Date.now() - refreshedAt < minMs) return;
    void refreshAll();
  }

  return {
    hub,
    repos,
    envs,
    workspaces,
    loadError,
    refreshTick,
    refreshing,
    theme,
    terminal,
    editor,
    editorCommand,
    init,
    reposTab,
    setHub,
    refreshAll,
    refreshIfStale,
  };
});