<script setup lang="ts">
import { useCopy } from "../lib/copy";
import { NO_AUTO } from "../lib/inputs";
import { groupByFamily } from "../lib/wsGroups";
import { baseChainLabel, hasBase } from "../lib/base";
import { computed, onMounted, ref } from "vue";
import { NButton, NInput, NTag, useDialog, useMessage } from "naive-ui";

import * as api from "../bridge";
import { useAppStore } from "../stores/app";
import { openInFinder, openInTerminal } from "../bridge";
import EditorButton from "../components/EditorButton.vue";
import NewWsDialog from "../components/NewWsDialog.vue";
import GetWsDialog from "../components/GetWsDialog.vue";
import WsDetail from "../components/WsDetail.vue";

const props = defineProps<{ initial?: string | null }>();
const emit = defineEmits<{ (e: "consumed"): void }>();
const app = useAppStore();
const msg = useMessage();
const copy = useCopy();
const dialog = useDialog();

const q = ref("");
const detail = ref<string | null>(null);
const showNew = ref(false);
const showGet = ref(false);

const shown = computed(() => {
  const s = q.value.trim().toLowerCase();
  if (!s) return app.workspaces;
  return app.workspaces.filter(
    (w) => w.branch.toLowerCase().includes(s) || w.title.toLowerCase().includes(s),
  );
});

/** 按需求分类分组；规则与「未分类」兜底见 `lib/wsGroups`（那里有测试）。 */
const groups = computed(() => groupByFamily(shown.value));

onMounted(async () => {
  await app.refreshAll();
  // 从「仓库详情」点某个需求跳过来：直接打开它
  if (props.initial) {
    detail.value = props.initial;
    emit("consumed");
  }
});

async function afterCreate() {
  showNew.value = false;
  showGet.value = false;
  await app.refreshAll();
}

/** 清理 ws/ 下的残留目录（需求 #6）。先扫后问，绝不直接删。 */
async function cleanLeftovers() {
  try {
    const out = await api.wsClean(false);
    if (!out.leftovers.length) {
      msg.success("ws/ 下没有残留目录");
      return;
    }
    dialog.warning({
      title: `发现 ${out.leftovers.length} 个残留目录`,
      content: () => {
        return out.leftovers.join("\n");
      },
      positiveText: "清理",
      negativeText: "取消",
      onPositiveClick: async () => {
        const done = await api.wsClean(true);
        msg.success(`已清理 ${done.removed.length} 个`);
        if (done.failed.length) {
          msg.error(`${done.failed.length} 个清理失败（可能被占用）`);
        }
        await app.refreshAll();
      },
    });
  } catch (e) {
    msg.error(api.errText(e));
  }
}
</script>

<template>
  <div class="page">
    <template v-if="!detail">
      <div class="head">
        <h2 class="title">需求</h2>
        <div class="tools">
          <NInput
            v-model:value="q"
            size="small"
            clearable
            placeholder="筛选分支名 / 中文名…"
            class="search"
           :input-props="NO_AUTO"/>
          <!-- 手动刷新：窗口聚焦会自动刷，但「刚从别处改完、窗口一直没失焦」时
               需要一个明确的重读入口（Agent 用 skill 改完也常是这种情况）。 -->
          <NButton size="small" :loading="app.refreshing" @click="app.refreshAll()">刷新</NButton>
          <NButton size="small" @click="cleanLeftovers">清理残留</NButton>
          <NButton size="small" @click="showGet = true">导入需求</NButton>
          <NButton size="small" type="primary" @click="showNew = true">新建需求</NButton>
        </div>
      </div>

      <p v-if="app.loadError" class="err">{{ app.loadError }}</p>

      <div v-for="g in groups" :key="g.family" class="ws-group">
        <div class="group-head">
          <span class="group-name">{{ g.family }}</span>
          <span
            class="group-count"
            :title="
              /^\d{4}-\d{2}$/.test(g.family)
                ? `这些需求没填「需求分类」，按创建年月归在一起（${g.items.length} 个）`
                : `这个需求分类下有 ${g.items.length} 个期次`
            "
          >
            {{ g.items.length }} 个期次
          </span>
        </div>
        <div v-for="w in g.items" :key="w.branch" class="ws-row" @click="detail = w.branch">
        <div class="ws-main">
          <span class="ws-title">{{ w.title || w.branch }}</span>
          <code class="branch">{{ w.branch }}</code>
          <!-- 基线来源（按优先级），主干显式挂在链尾 —— 它是兜底，不写出来
               就看不出「链上一个都不命中时落到哪」。**没填基线就不显示**：
               那是绝大多数需求的默认值（等于直接主干），每行都写一遍只是噪声。 -->
          <span
            v-if="hasBase(w.base)"
            class="base"
            :title="`按这个顺序找上游：第一个有该模块分支的用它，都不命中就兜底主干`"
          >
            基线 {{ baseChainLabel(w.base) }}
          </span>
          <!-- 「归入 X」这个标签**去掉了**：按需求分类分组之后，组头就是分类名，
               再挂一个标签是把同一个词写两遍（实测反馈 #3 早就说过「不知道是什么」）。 -->
          <NTag v-if="w.missing_modules > 0" size="tiny" :bordered="false" type="warning">
            {{ w.missing_modules }} 个模块目录缺失
          </NTag>
        </div>
        <div class="ws-side" @click.stop>
          <span class="muted">{{ w.modules }} 个模块 · {{ w.created }}</span>
          <NButton size="tiny" secondary @click="openInFinder(app.hub!.root + '/ws/' + w.branch)">
            打开目录
          </NButton>
          <EditorButton size="tiny" :path="app.hub!.root + '/ws/' + w.branch" />
          <NButton
            size="tiny"
            secondary
            @click="openInTerminal(app.hub!.root + '/ws/' + w.branch, app.terminal)"
          >
            终端
          </NButton>
          <NButton size="tiny" secondary @click="copy(app.hub!.root + '/ws/' + w.branch, '目录')">
            复制目录
          </NButton>
          </div>
        </div>
      </div>
      <p v-if="!shown.length && !app.loadError" class="muted">
        {{ q ? "没有匹配的需求" : "还没有需求，点右上角「新建需求」" }}
      </p>
    </template>

    <WsDetail v-else :key="detail" :branch="detail" @close="detail = null" @changed="app.refreshAll()" />

    <NewWsDialog v-if="showNew" @close="afterCreate" />
    <GetWsDialog v-if="showGet" @close="afterCreate" />
  </div>
</template>

<style scoped>
/* ⚠️ 这几条必须在**顶层**。
   上一版我插到了 `.ws-row { }` 里面 —— 普通 CSS 不支持嵌套，浏览器把整块
   直接丢掉，于是分组头一点样式都没有（实测反馈：「很丑 很挤 很不明显」）。
   Vue 的 scoped 样式是普通 CSS，写嵌套只会在编译期静默丢掉，不报错。 */

/* 需求分类分组：组头一行 + 一条分割线，下面挂该分类的各期次 */
.ws-group {
  margin-bottom: 26px;
}
.group-head {
  display: flex;
  align-items: baseline;
  gap: 10px;
  padding: 0 2px 10px;
  margin-bottom: 10px;
  border-bottom: 1px solid var(--xg-border);
}
.group-name {
  font-size: 15px;
  font-weight: 600;
  letter-spacing: 0.3px;
}
/* 「N 个期次」做成一个淡标签，跟名字之间靠 flex 的 gap 拉开 ——
   上一版这里被整块丢掉，于是两者粘在一起（实测反馈） */
.group-count {
  font-size: 12px;
  color: #888;
  background: rgba(128, 128, 128, 0.1);
  border-radius: 10px;
  padding: 1px 8px;
}
/* 基线来源：比分支名弱一档，它是补充信息 */
.base {
  font-size: 12px;
  color: #888;
}
.page {
  padding: 20px 24px 40px;
}
.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  margin-bottom: 16px;
  flex-wrap: wrap;
}
.title {
  margin: 0;
  font-size: 18px;
  font-weight: 600;
}
.tools {
  display: flex;
  align-items: center;
  gap: 8px;
}
.search {
  width: 220px;
}
.ws-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
  border: 1px solid var(--xg-border);
  border-radius: 6px;
  /* 行高给足。实测反馈 #7：原来 9px 太紧凑，一行挤得跟表格行一样，
     而需求数量本来就不多，没必要为了省这点高度牺牲可读性 */
  padding: 14px 16px;
  margin-bottom: 8px;
  cursor: pointer;
}
.ws-row:hover {
  background: rgba(128, 128, 128, 0.05);
}
.ws-main {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}
.ws-title {
  font-weight: 600;
  font-size: 14px;
}
.branch {
  color: #888;
}
.ws-side {
  display: flex;
  align-items: center;
  gap: 10px;
  white-space: nowrap;
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
</style>