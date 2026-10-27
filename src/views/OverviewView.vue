<script setup lang="ts">
import { computed } from "vue";
import { NAlert, NButton, NCard, NGi, NGrid, NStatistic, NTag } from "naive-ui";

import { useAppStore } from "../stores/app";
import QuickLinks from "../components/QuickLinks.vue";

const emit = defineEmits<{ (e: "goto", v: string): void }>();
const app = useAppStore();

/**
 * 「快捷入口」：地址由用户在设置页自己配（存在 hub 的 `gws.json` 里）。
 *
 * 开源版**不预设任何发布系统** —— 没配就整块隐藏，不会摆一个点了 404 的按钮。
 *
 * 直接跳、不做二次确认：这些是「每天要跳好几次」的页面，多一步就是白费一步。
 */
const quickLinks = computed(() => app.hub?.quick_links ?? []);

/** 概览的四个数字。**每张卡都能点进对应页面**。 */
/**
 * 点一张卡片：跳到对应页面。
 *
 * 「仓库」和「环境」在同一个页面里，靠标签区分 —— 所以带上「落到哪个标签」
 * 的意图，否则点「环境」会停在「仓库」标签上（实测反馈）。
 */
function goTo(c: { goto: string; tab?: string }) {
  if (c.tab) app.reposTab = c.tab;
  emit("goto", c.goto);
}

const cards = computed(() => [
  { label: "仓库", value: app.repos.length, goto: "repos" },
  { label: "需求", value: app.workspaces.length, goto: "requirements" },
  { label: "环境", value: app.envs.length, goto: "repos", tab: "envs" },
  { label: "文档库", value: app.hub?.dochub_ready ? "已就绪" : "未拉取", goto: "docs" },
]);

/**
 * 需要人关注的事。
 *
 * 实测反馈 #6：这一块「不知道是什么功能，看起来也没用」——原因是它**永远显示**，
 * 而且平时那几条检查几乎都不成立，于是一张写着「没有需要注意的事项」的卡片
 * 常年占着地方。改两点：
 * 1. 没问题时**整块不渲染**（而不是渲染一句"没有事项"）
 * 2. 补一条真正会成立、且能直接动手的检查：没被任何需求/环境使用的仓库
 */
const todos = computed(() => {
  const out: { text: string; goto: string; kind: "warn" | "info"; action: string }[] = [];

  for (const w of app.workspaces) {
    if (w.missing_modules > 0) {
      out.push({
        text: `${w.title || w.branch}：有 ${w.missing_modules} 个模块目录缺失`,
        goto: "requirements",
        kind: "warn",
        action: "去清理",
      });
    }
  }

  // 谁都没用的仓库 = 白占磁盘，也是最容易忘记的一块。
  //
  // **判据是「需求与环境都没用它」**：被环境用着的仓库是**活跃代码**
  // （环境里那份检出就是配置出来要用的），不是闲置。
  // 原来只看需求，于是环境里明明有它也报一句「没被任何需求或环境使用」——
  // 文案与判据对不上，而且不实（实测反馈）。
  //
  // **文档库不参与这条检查**：它是 hub 的固定部件，本来就不该被删，
  // 报它一句「没被使用」纯属噪声（实测反馈）。
  const unused = app.repos.filter(
    (r) => !r.is_doc && !r.usages.length && !r.env_usages.length,
  );
  if (unused.length) {
    out.push({
      text: `有 ${unused.length} 个仓库没被任何需求或环境使用（${unused
        .slice(0, 3)
        .map((r) => r.name)
        .join("、")}${unused.length > 3 ? " 等" : ""}）`,
      goto: "repos",
      kind: "info",
      action: "去看看",
    });
  }

  if (app.repos.length === 0) {
    out.push({ text: "还没有添加任何仓库", goto: "repos", kind: "warn", action: "去添加" });
  }
  if (app.hub && !app.hub.dochub_ready) {
    out.push({
      text: "文档库尚未拉到本地",
      goto: "docs",
      kind: "info",
      action: "去拉取",
    });
  }
  return out;
});
</script>

<template>
  <div class="page">
    <h2 class="title">总览</h2>

    <NGrid :cols="4" :x-gap="12" :y-gap="12" responsive="screen" item-responsive>
      <!-- 卡片**可点击跳转**：看到数字后下一步一定是「去那个页看看」，
           让人再点一次左侧菜单是白费一步（实测反馈） -->
      <!-- key 用 label：两个卡片的 goto 都是 "repos"（仓库/环境同页），
           用 goto 当 key 会撞，Vue 复用了节点 → 点「需求」跑去了仓库（实测反馈） -->
      <NGi v-for="c in cards" :key="c.label" span="4 s:2 m:1">
        <NCard class="stat" hoverable @click="goTo(c)">
          <NStatistic :label="c.label" :value="c.value" />
        </NCard>
      </NGi>
    </NGrid>

    <!-- hub 信息集中在这里（原来首页与设置各有一份，是重复的）。
         设置页只留主题/终端/GitLab/更新这些"配置"，hub 相关的都在首页。 -->
    <NCard class="block" title="hub">
      <div class="kv"><span class="k">位置</span><code class="path">{{ app.hub?.root }}</code></div>
      <div class="kv"><span class="k">名称</span><span>{{ app.hub?.name }}</span></div>
      <div class="kv">
        <span class="k">环境</span>
        <span class="envs">
          <NTag v-for="[id, label] in app.hub?.envs ?? []" :key="id" size="small" :bordered="false">
            {{ id }}<span v-if="label && label !== id" class="muted"> · {{ label }}</span>
          </NTag>
        </span>
      </div>
      <div class="kv">
        <span class="k">文档库</span>
        <span>
          <NTag size="tiny" :bordered="false" :type="app.hub?.dochub_ready ? 'success' : 'warning'">
            {{ app.hub?.dochub_ready ? "已拉到本地" : "未拉取" }}
          </NTag>
        </span>
      </div>
    </NCard>

    <!-- 没问题时整块不渲染：留一张写着「没有需要注意的事项」的卡片，
           看着就是一块没用的东西（反馈 #6） -->
    <!-- 快捷入口：hub 里配了才显示（开源版不预设发布系统） -->
    <NCard v-if="quickLinks.length" class="block" title="快捷入口">
      <QuickLinks :links="quickLinks" />
    </NCard>

    <NCard v-if="todos.length" class="block" title="待办">
      <NAlert
        v-for="(t, i) in todos"
        :key="i"
        :type="t.kind === 'warn' ? 'warning' : 'info'"
        class="todo"
        :bordered="false"
      >
        <span>{{ t.text }}</span>
        <NButton size="tiny" text type="primary" class="todo-btn" @click="emit('goto', t.goto)">
          {{ t.action }}
        </NButton>
      </NAlert>
    </NCard>

    <NCard v-if="app.loadError" class="block" title="加载出错">
      <p class="err">{{ app.loadError }}</p>
      <NButton size="small" @click="app.refreshAll()">重试</NButton>
    </NCard>
  </div>
</template>

<style scoped>
.page {
  padding: 20px 24px 40px;
}
.title {
  margin: 0 0 16px;
  font-size: 18px;
  font-weight: 600;
}
.block {
  margin-top: 16px;
}
.path {
  word-break: break-all;
}
.envs {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}
.kv {
  display: grid;
  grid-template-columns: 5em 1fr;
  gap: 8px;
  padding: 3px 0;
  font-size: 13px;
  align-items: baseline;
}
.k {
  color: #888;
}
.actions {
  margin-top: 10px;
}
.stat {
  cursor: pointer;
}
.muted {
  color: #888;
  font-size: 12px;
}
.err {
  color: #d03050;
  font-size: 13px;
  white-space: pre-wrap;
}
.todo {
  margin-bottom: 8px;
}
</style>