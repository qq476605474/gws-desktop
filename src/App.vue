<script setup lang="ts">
import { computed, h, onMounted, onUnmounted, ref } from "vue";
import {
  NConfigProvider,
  NDialogProvider,
  NLayout,
  NLayoutSider,
  NMenu,
  NMessageProvider,
  NProgress,
  NSpin,
  darkTheme,
  zhCN,
  dateZhCN,
  type GlobalThemeOverrides,
  type MenuOption,
} from "naive-ui";

import * as api from "./bridge";
import { useAppStore } from "./stores/app";
import { busy, initProgressListener } from "./busy";
import NotifierBridge from "./components/NotifierBridge.vue";
import HubMenu from "./components/HubMenu.vue";
import PaseoSessionButton from "./components/PaseoSessionButton.vue";
import UpdatePill from "./components/UpdatePill.vue";
import VersionBadge from "./components/VersionBadge.vue";
import StartupView from "./views/StartupView.vue";
import OverviewView from "./views/OverviewView.vue";
import RequirementsView from "./views/RequirementsView.vue";
import ReposView from "./views/ReposView.vue";
import DocsView from "./views/DocsView.vue";
import SettingsView from "./views/SettingsView.vue";
import HelpView from "./views/HelpView.vue";

const app = useAppStore();

const ready = ref(false);
const current = ref("overview");
/** 从仓库详情跳过去时要打开的需求分支（一次性）。 */
const pendingWs = ref<string | null>(null);

/** 导航结构。`发布` 与 `MR` 是**需求级**的，收在需求详情里而不是做成顶级入口——
 *  它们都得先选定一个需求才有意义，做成顶级入口会多一次「选哪个需求」的往返。 */
const menuOptions: MenuOption[] = [
  { label: "总览", key: "overview" },
  { label: "需求", key: "requirements" },
  { label: "仓库", key: "repos" },
  { label: "环境", key: "envs" },
  { label: "文档", key: "docs" },
  { label: "帮助", key: "help" },
  { label: "设置", key: "settings" },
];

/** Naive UI 主题。设计令牌集中在这里，不在组件里散写颜色。 */
const themeOverrides: GlobalThemeOverrides = {
  common: {
    primaryColor: "#2563eb",
    primaryColorHover: "#1d4ed8",
    primaryColorPressed: "#1e40af",
    borderRadius: "6px",
    fontSize: "13px",
  },
  Layout: {
    siderColor: "var(--sider-bg)",
  },
};

const theme = computed(() => (app.theme === "dark" ? darkTheme : null));

/**
 * 回到前台时刷新一次。
 *
 * Agent（Paseo）是**另一个应用**：它用 skill 改了需求/模块/文档之后，
 * 桌面端不会收到任何通知，只能靠窗口重新聚焦时自己重读。
 * 页面内自己拉的数据（需求详情、文档树）各自 watch `refreshTick`。
 */
let unlistenFocus: (() => void) | null = null;
function onVisible() {
  if (document.visibilityState === "visible") app.refreshIfStale();
}
onMounted(async () => {
  // 进度订阅要在**任何慢操作之前**建立，否则第一条进度会丢
  await initProgressListener();

  try {
    await app.init();
    document.documentElement.setAttribute("data-theme", app.theme);
    // 上次用过的 hub 自动打开——很少切换，每次都手点一下很烦（反馈 #15）
    const last = localStorage.getItem("lastHub");
    if (last) {
      try {
        const info = await api.openHub(last);
        app.setHub(info);
        void app.refreshAll();
      } catch {
        // 目录可能已被删/移动；静默回落到启动页，不打断
      }
    }
  } finally {
    ready.value = true;
  }

  // Tauri 窗口的聚焦事件；浏览器里没有这个 API，当作可选能力
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    unlistenFocus = await getCurrentWindow().onFocusChanged(({ payload }) => {
      if (payload) app.refreshIfStale();
    });
  } catch {
    // 没有 Tauri 窗口（如浏览器预览）时只留 visibilitychange
  }
  document.addEventListener("visibilitychange", onVisible);
});

onUnmounted(() => {
  unlistenFocus?.();
  document.removeEventListener("visibilitychange", onVisible);
});

// 主题切换要落到 DOM 上，自定义 CSS 变量才知道该用哪套
import { nextTick, watch } from "vue";
watch(
  () => app.theme,
  (t) => document.documentElement.setAttribute("data-theme", t),
);

/** 进度日志的滚动容器。 */
/**
 * 「从总览跳到某个标签」要落到**菜单项**上。
 *
 * 拆成两个菜单项之后，`reposTab='envs'` 意味着应该切到「环境」那一项，
 * 而不是留在「仓库」页再想办法切 tab（那种绕法在 only 模式下根本不生效）。
 */
watch(
  () => app.reposTab,
  (v) => {
    if (!v) return;
    current.value = v === "envs" ? "envs" : "repos";
    app.reposTab = null;
  },
);

const logEl = ref<HTMLElement | null>(null);

/**
 * 新进度追加后**自动滚到底**。
 *
 * 原来一直停在顶部：几十条进度滚过之后，用户看到的是最早那几条，
 * 完全不知道当前跑到哪了——而「当前跑到哪」正是进度存在的意义。
 */
watch(
  () => busy.lines.length,
  async () => {
    await nextTick();
    const el = logEl.value;
    if (el) el.scrollTop = el.scrollHeight;
  },
);

function renderIcon(text: string) {
  return () => h("span", { style: "font-size:15px" }, text);
}

const menu = computed<MenuOption[]>(() =>
  menuOptions.map((o) => ({ ...o, icon: renderIcon(iconOf(String(o.key))) })),
);

/**
 * 菜单图标。每个都跟**这一栏到底是什么**对上（实测反馈：原来的 📋/📦/🧪 不够贴）：
 *
 * - `需求` = **正在开发的任务** → 🚧（施工中：一眼看出是「还在做」）
 * - `仓库` = **主干代码**，干净代码，所有代码的父亲 → 🌳
 *   （git 自己的隐喻就是树：主干 + 分支；根/主干在那，分支才从它长出来）
 * - `环境` = **验证代码的测试环境** → 🖥️（就是那台跑起来给你验的机器）
 * - `文档` = 一批文档 → 📄；`总览` = 首页 → 🏠；`设置` → ⚙️
 */
function iconOf(key: string): string {
  switch (key) {
    case "overview":
      return "🏠";
    case "requirements":
      return "🚧";
    case "repos":
      return "🌳";
    case "envs":
      return "🖥️";
    case "docs":
      return "📄";
    case "help":
      // 「帮助」不是设置 —— 原来跟设置一样都是 ⚙️，两个入口分不清（实测反馈）
      return "📘";
    default:
      return "⚙️";
  }
}
</script>

<template>
  <NConfigProvider :theme="theme" :theme-overrides="themeOverrides" :locale="zhCN" :date-locale="dateZhCN">
    <NMessageProvider>
      <!-- 把 message 实例装进全局出口：bridge 里的打开/复制这类命令
           失败时要弹得出来（它们不在组件里，调不到 useMessage） -->
      <NotifierBridge />
      <NDialogProvider>
        <!-- 偏好未读完不渲染子组件：否则 StartupView 会读到空的 lastHub，
             「上次打开的 hub 自动进入」会失效 -->
        <div v-if="!ready" class="boot">
          <NSpin size="large" />
        </div>

        <StartupView v-else-if="!app.hub" @opened="current = 'overview'" />

        <NLayout v-else class="shell" has-sider>
          <NLayoutSider bordered :width="180" :native-scrollbar="false">
            <div class="brand">
              <!-- 版本号跟标题同一行放在右边；hub 名字单独一行、点开是菜单
                   （改名 / 切换 hub / 打开 hub 目录）。有新版时下面再出一条
                   单行提示（见 UpdatePill）。 -->
              <div class="brand-head">
                <div class="brand-title">GWS</div>
                <VersionBadge />
              </div>
              <div class="brand-hub">
                <HubMenu />
              </div>
              <UpdatePill />
            </div>
            <NMenu
              :value="current"
              :options="menu"
              :indent="18"
              @update:value="(v: string) => (current = v)"
            />
            <div class="sider-foot">
              <!-- 全局 AI 会话：不绑定需求。建需求前先聊清需求、或直接查线上日志；
                   有 bug 再从「需求」页建一个来修。
                   左下角只留这一个按钮 —— 版本号和切换 hub 已经挪到左上角。 -->
              <PaseoSessionButton
                label="开 AI 会话"
                size="small"
                block
                secondary
                hint="在 hub 根目录开一个不绑定需求的 AI 会话（聊需求、查线上日志）"
              />
            </div>
          </NLayoutSider>

          <NLayout class="content" :native-scrollbar="false">
            <OverviewView v-if="current === 'overview'" @goto="(v: string) => (current = v)" />
            <RequirementsView v-else-if="current === 'requirements'" :initial="pendingWs" @consumed="pendingWs = null" />
            <!-- 仓库与环境是**两个菜单项、两个页面**（实测反馈：想让它们拆开）。
                 内部仍是同一个组件（共用同步弹窗与环境状态），靠 only 锁死显示哪一栏。 -->
            <ReposView
              v-else-if="current === 'repos' || current === 'envs'"
              :key="current"
              :only="current === 'envs' ? 'envs' : 'repos'"
              @goto-ws="
                (b: string) => {
                  pendingWs = b;
                  current = 'requirements';
                }
              "
            />
            <DocsView v-else-if="current === 'docs'" />
            <HelpView v-else-if="current === 'help'" />
            <SettingsView v-else-if="current === 'settings'" />
          </NLayout>
        </NLayout>

        <!-- 慢操作遮罩。**必须显示实时进度**：只给一个转圈的话，
             几十秒的操作分不清是在跑还是卡死（实际使用反馈里最影响体验的一条） -->
        <!-- 遮罩**不给"收起来"的出口**：实测反馈——点过「后台运行」之后再去点别的，
             会变成两件事并行，状态说不清（弹窗还开着、目录还在建）。宁可等。
             所以这里没有小条、也没有那个按钮（原来有，已按反馈移除）。 -->
        <!-- 只给 withBusy 包过的慢操作拉遮罩。普通请求（开 AI 会话之类）也拉的话，
             会连着上一次慢操作的旧标题和日志一起弹出来（实测「补齐环境 pre」） -->
        <div v-if="busy.mask > 0" class="busy-mask">
          <div class="busy-box">
            <div class="busy-head">
              <NSpin size="small" />
              <span class="busy-text">{{ busy.label || "处理中…" }}</span>
              <span v-if="busy.total > 0" class="busy-count">
                {{ busy.done }} / {{ busy.total }}
              </span>
            </div>
            <NProgress
              v-if="busy.total > 0"
              type="line"
              :percentage="Math.round((busy.done / busy.total) * 100)"
              :show-indicator="false"
              :height="4"
            />
            <!-- 日志容器**始终渲染**（固定高度）：
                 原来只在有内容时渲染，于是弹窗从「一行提示」跳到「带日志的大块」，
                 看着像在抽动。固定高度后整个框从出现到结束都是一个大小。 -->
            <div ref="logEl" class="busy-log">
              <div
                v-for="(l, i) in busy.lines"
                :key="i"
                class="busy-line"
                :class="{ bad: !l.ok }"
              >
                {{ l.ok ? "✓" : "✗" }} {{ l.text }}
              </div>
              <div v-if="!busy.lines.length" class="busy-line muted-line">等待输出…</div>
            </div>
            <div class="busy-foot">
              <span class="busy-hint">
                大仓库首次操作可能要几分钟（每个仓库都要检出完整工作树）。运行期间请等它跑完。
              </span>
            </div>
          </div>
        </div>
      </NDialogProvider>
    </NMessageProvider>
  </NConfigProvider>
</template>

<style scoped>
.boot {
  height: 100%;
  display: grid;
  place-items: center;
}
.shell {
  height: 100vh;
}
.content {
  height: 100vh;
}
.brand {
  padding: 16px 16px 12px;
  border-bottom: 1px solid var(--xg-border);
}
/* 标题和版本号同一行：版本号安静地待在标题右边 */
.brand-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 8px;
  min-width: 0;
}
.brand-title {
  font-weight: 600;
  font-size: 15px;
  letter-spacing: 0.02em;
  white-space: nowrap;
}
.brand-hub {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  margin-top: 4px;
}
.sider-foot {
  position: absolute;
  bottom: 0;
  width: 100%;
  padding: 10px 16px;
  border-top: 1px solid var(--xg-border);
}
.busy-mask {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.18);
  display: grid;
  place-items: center;
  z-index: 3000;
}
.busy-box {
  background: var(--n-color, #fff);
  border-radius: 8px;
  padding: 18px 22px;
  display: flex;
  flex-direction: column;
  gap: 10px;
  box-shadow: 0 6px 24px rgba(0, 0, 0, 0.18);
  width: 520px;
  max-width: 90vw;
}
.busy-head {
  display: flex;
  align-items: center;
  gap: 10px;
}
.busy-text {
  font-size: 13px;
  font-weight: 500;
  flex: 1;
}
.busy-count {
  font-size: 12px;
  color: #888;
  font-variant-numeric: tabular-nums;
}
/* 固定高度：max-height 会让框随内容长高，看着在抽动。
   定死 200px，内容多了自己滚。 */
.busy-log {
  height: 200px;
  overflow: auto;
  background: rgba(128, 128, 128, 0.08);
  border-radius: 6px;
  padding: 8px 10px;
  font-family: var(--mono);
  font-size: 12px;
  line-height: 1.6;
}
.busy-line {
  white-space: pre-wrap;
  word-break: break-all;
}
.busy-line.bad {
  color: #d03050;
}
.muted-line {
  color: #999;
}
.busy-hint {
  font-size: 11px;
  color: #999;
  flex: 1;
}
.busy-foot {
  display: flex;
  align-items: center;
  gap: 10px;
}
</style>