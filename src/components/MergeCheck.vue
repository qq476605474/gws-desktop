<script setup lang="ts">
//! 合并后检查：这个需求到底合进环境了什么、推没推、有没有冲突。
//!
//! # 为什么需要它
//!
//! 原来要确认「dev 上现在是什么样」，得用 IDE 打开 `envs/dev/` —— 那里涉及
//! 十几个仓库，**内存直接吃满**（实测反馈）。这里只读 git 元数据，不开 IDE
//! 就能回答三个问题：
//!   1. 有没有冲突（有 → 去解决）
//!   2. 合了什么上去（提交清单 + 改动文件数）
//!   3. 推没推（没推 → 可补一次推送）

import { computed, ref } from "vue";
import { NButton, NCard, NModal, NSelect, NSpin, NTag, useMessage } from "naive-ui";

import * as api from "../bridge";
import FileBrowser from "./FileBrowser.vue";
import { useAppStore } from "../stores/app";
import type { EnvModuleCheck } from "../types";

const props = defineProps<{ branch: string; title?: string }>();
const emit = defineEmits<{ (e: "close"): void }>();
const app = useAppStore();
const msg = useMessage();

/**
 * **默认不选**，由人指定看哪个环境。
 *
 * 一开始我给了个默认（dev），但实测反馈：不一定看 dev，可能看 pre 或别的
 * —— 默认值反而会让人以为「它只查 dev」，或者不自觉地就查了错的那个。
 * 空着 + 一句提示，比猜一个更安全。
 */
const env = ref<string | null>(null);
const rows = ref<EnvModuleCheck[]>([]);
const loading = ref(false);
const pushing = ref(false);
const browse = ref<{ dir: string; repo: string; label: string; base: string | null } | null>(null);

const conflicts = computed(() => rows.value.filter((r) => r.conflict));
const unpushed = computed(() => rows.value.filter((r) => r.unpushed > 0));
const totalCommits = computed(() => rows.value.reduce((n, r) => n + r.commits.length, 0));

async function load() {
  if (!env.value) return;
  loading.value = true;
  try {
    rows.value = await api.envMergeCheck(props.branch, env.value);
  } catch (e) {
    msg.error(api.errText(e));
    rows.value = [];
  } finally {
    loading.value = false;
  }
}

/** 补推：把还没上远程的环境分支推上去。 */
async function pushAll() {
  if (!env.value) return;
  pushing.value = true;
  try {
    const mods = unpushed.value.map((r) => r.module);
    // 后端返回的是 `[模块, 失败原因][]` —— **空原因才是成功**。
    // 原来把返回值整个丢掉、无条件报「已推送 N 个」，某个模块被拒（远程有他人提交、
    // 凭据失效）时也是这个提示：假成功（实测反馈里最忌讳的一类）。
    const out = await api.wsPushToEnv(props.branch, env.value, mods);
    const ok = out.filter(([, why]) => !why).map(([m]) => m);
    const failed = out.filter(([, why]) => why);
    if (ok.length) msg.success(`已推送 ${ok.length} 个模块`);
    for (const [m, why] of failed) msg.error(`${m} 推送失败：${why}`);
    if (!ok.length && !failed.length) msg.warning("没有需要推送的模块");
    await load();
  } catch (e) {
    msg.error(api.errText(e));
  } finally {
    pushing.value = false;
  }
}

// 不自动查：环境要人先指定（见 `env` 的注释）
</script>

<template>
  <NModal
    :show="true"
    @update:show="(v: boolean) => !v && emit('close')"
    preset="card"
    :title="`合并检查 · ${title || branch}`"
    style="width: 92vw; height: 88vh"
    @close="emit('close')"
  >
    <template v-if="browse">
      <div class="back">
        <NButton size="tiny" @click="browse = null">← 回到检查结果</NButton>
        <span class="mono small">{{ browse.label }}</span>
      </div>
      <FileBrowser
        :dir="browse.dir"
        :repo="browse.repo"
        :label="browse.label"
        :base="browse.base"
      />
    </template>

    <template v-else>
      <div class="toolbar">
        <span class="muted small">环境</span>
        <NSelect
          v-model:value="env"
          :options="app.envs.map((e) => ({ label: e.label && e.label !== e.id ? `${e.label}（${e.id}）` : e.id, value: e.id }))"
          size="small"
          class="env-sel"
          @update:value="load"
        />
        <NButton size="small" :loading="loading" :disabled="!env" @click="load">
          {{ rows.length ? "重新检查" : "开始检查" }}
        </NButton>
        <div class="spacer" />
        <NButton
          v-if="unpushed.length"
          size="small"
          type="primary"
          :loading="pushing"
          @click="pushAll"
        >
          推送未上远程的 {{ unpushed.length }} 个模块
        </NButton>
      </div>

      <p v-if="!env" class="muted">先选一个环境，再点「开始检查」。</p>
      <NSpin v-else-if="loading" size="small" />

      <template v-else-if="env">
        <!-- 模块一多会把内容顶出窗口 —— 这一层自己滚（实测反馈）。
             `max-height` 用 vh 而不是 flex：模态框的内容区不吃父级高度。 -->
        <div class="scroll">
        <!-- 结论先摆出来：冲突最要紧，其次「没推」 -->
        <NCard v-if="conflicts.length" size="small" class="verdict bad">
          <span class="v-title">有 {{ conflicts.length }} 个模块存在未解决的冲突</span>
          <div class="muted small">
            合并卡在中途，环境里是**半合并状态**。到对应目录里解决后提交即可
            （点右边的「浏览文件」能看到冲突文件）。
          </div>
        </NCard>
        <NCard
          v-else-if="unpushed.length"
          size="small"
          class="verdict warn"
        >
          <span class="v-title">合并干净，但有 {{ unpushed.length }} 个模块还没推上远程</span>
          <div class="muted small">
            线上（和测试环境）看到的还是旧的。点右上角补推一次。
          </div>
        </NCard>
        <NCard v-else-if="rows.length" size="small" class="verdict ok">
          <span class="v-title">合并干净、也已全部推送</span>
          <div class="muted small">共 {{ rows.length }} 个模块、{{ totalCommits }} 个提交进了 {{ env }}。</div>
        </NCard>
        <p v-if="!rows.length" class="muted">
          这个需求没有模块参与 {{ env }} 环境（或环境目录还不存在）。
        </p>

        <NCard v-for="r in rows" :key="r.module" size="small" class="mod">
          <template #header>
            <span class="mono">{{ r.module }}</span>
            <NTag v-if="r.conflict" size="tiny" type="error" :bordered="false" class="tag">冲突</NTag>
            <NTag
              v-else-if="r.unpushed > 0"
              size="tiny"
              type="warning"
              :bordered="false"
              class="tag"
            >
              未推送 {{ r.unpushed }}
            </NTag>
            <NTag v-else size="tiny" type="success" :bordered="false" class="tag">已推送</NTag>
            <span class="muted small">{{ r.commits.length }} 个提交 · {{ r.files }} 个文件</span>
          </template>
          <template #header-extra>
            <NButton
              size="tiny"
              @click="
                browse = {
                  dir: `${app.hub?.root}/envs/${env}/${r.module}`,
                  repo: r.module,
                  label: `${r.module} @ ${env}`,
                  // **基准是远程环境分支**：检查合并时关心的是「本地 dev 比远程
                  // dev 多了什么」，而不是「跟主干比」（实测反馈）
                  base: r.env_branch ? `origin/${r.env_branch}` : null,
                }
              "
            >
              浏览文件
            </NButton>
          </template>

          <p v-if="r.error" class="err small">{{ r.error }}</p>
          <div v-if="r.conflict" class="conflict">
            <div v-for="f in r.conflict_files" :key="f" class="mono small">✗ {{ f }}</div>
          </div>
          <div v-if="r.commits.length" class="commits">
            <div v-for="[sha, subject] in r.commits.slice(0, 30)" :key="sha" class="mono small">
              <span class="sha">{{ sha }}</span> {{ subject }}
            </div>
            <p v-if="r.commits.length > 30" class="muted small">
              还有 {{ r.commits.length - 30 }} 个提交未列出
            </p>
          </div>
          <p v-else class="muted small">相对主干没有多出来的提交（该模块在环境里就是主干的样子）。</p>
        </NCard>
        </div>
      </template>
    </template>

    <template #footer>
      <div class="footer">
        <NButton @click="emit('close')">关闭</NButton>
      </div>
    </template>
  </NModal>
</template>

<style scoped>
.scroll {
  max-height: calc(88vh - 210px);
  overflow: auto;
}
.toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 10px;
}
.env-sel {
  width: 200px;
}
.spacer {
  flex: 1;
}
.verdict {
  margin-bottom: 10px;
}
.verdict.bad {
  border-left: 3px solid #d03050;
}
.verdict.warn {
  border-left: 3px solid #f0a020;
}
.verdict.ok {
  border-left: 3px solid #18a058;
}
.v-title {
  font-size: 14px;
  font-weight: 600;
}
.mod {
  margin-bottom: 10px;
}
.tag {
  margin-left: 6px;
}
.commits {
  max-height: 200px;
  overflow: auto;
}
.sha {
  color: #888;
}
.conflict {
  margin-bottom: 6px;
}
.back {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 8px;
}
.footer {
  display: flex;
  justify-content: flex-end;
}
.muted {
  color: #888;
}
.small {
  font-size: 12px;
}
.err {
  color: #d03050;
}
</style>