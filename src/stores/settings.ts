import { defineStore } from "pinia";
import { ref, watch } from "vue";
import { load, Store } from "@tauri-apps/plugin-store";

export type TerminalPref = "system" | "iTerm2" | "Terminal.app" | "Warp";
export type Theme = "light" | "dark" | "macos";

export const useSettingsStore = defineStore("settings", () => {
  const lastHub = ref<string>("");
  const terminal = ref<TerminalPref>("system");
  const theme = ref<Theme>("light");
  let store: Store | null = null;

  function applyTheme() {
    document.documentElement.setAttribute("data-theme", theme.value);
  }

  async function init() {
    if (store) return; // 幂等守卫：防重复调用注册多个 watch 造成重复写盘
    store = await load("settings.json", { autoSave: true });
    lastHub.value = (await store.get<string>("lastHub")) ?? "";
    terminal.value = (await store.get<TerminalPref>("terminal")) ?? "system";
    theme.value = (await store.get<Theme>("theme")) ?? "light";
    applyTheme();
    watch([lastHub, terminal, theme], async () => {
      applyTheme();
      await store?.set("lastHub", lastHub.value);
      await store?.set("terminal", terminal.value);
      await store?.set("theme", theme.value);
    });
  }

  return { lastHub, terminal, theme, init, applyTheme };
});
