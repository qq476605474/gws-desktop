<script setup lang="ts">
/**
 * 开 Paseo AI 会话的按钮 + 选 provider 的弹窗。
 *
 * 两种用法：
 * - 传 `branch`：需求会话（在需求详情里派发任务），标签页标题用需求名称；
 * - 不传 `branch`：**自由会话**，适合建需求前聊需求、直接查线上日志。
 *   这时弹窗里多一个「会话标题」，留空叫「临时会话」。
 *
 * 会话 cwd 永远是 hub 根目录（后端固定），没在这里做选择。
 */
import { ref } from "vue";
import {
  NAlert,
  NButton,
  NInput,
  NModal,
  NRadio,
  NRadioGroup,
  NSpin,
  useMessage,
} from "naive-ui";

import * as api from "../bridge";
import { NO_AUTO } from "../lib/inputs";
import { useAppStore } from "../stores/app";

const props = defineProps<{
  /** 绑定需求时传分支；不传就是自由会话。 */
  branch?: string;
  label: string;
  size?: "tiny" | "small" | "medium" | "large";
  type?: "default" | "tertiary" | "primary" | "info" | "success" | "warning" | "error";
  secondary?: boolean;
  block?: boolean;
  hint?: string;
}>();

const app = useAppStore();
const msg = useMessage();

const show = ref(false);
const busy = ref(false);
const loading = ref(false);
const providers = ref<string[]>([]);
const provider = ref("");
/** 自由会话的标签页标题。 */
const title = ref("");
/** 拉 provider 列表失败时的真实原因（例如没找到 paseo 命令）。 */
const loadError = ref("");

const free = () => !props.branch;

async function open() {
  show.value = true;
  providers.value = [];
  provider.value = "";
  title.value = "";
  loadError.value = "";
  loading.value = true;
  try {
    const list = await api.paseoProviders();
    providers.value = list;
    provider.value = list[0] ?? "";
    if (!list.length) msg.warning("没有可用的 Paseo provider（运行 paseo provider ls 看看）");
  } catch (e) {
    // 只在 toast 里显示会一闪而过；弹窗里留一份，用户能照着装。
    loadError.value = api.errText(e);
    msg.error(loadError.value);
  } finally {
    loading.value = false;
  }
}

async function start() {
  if (!provider.value) {
    msg.warning("先选择一个 provider");
    return;
  }
  busy.value = true;
  try {
    const out = props.branch
      ? await api.paseoOpenSession(props.branch, provider.value)
      : await api.paseoOpenFreeSession(title.value, provider.value);
    msg.success(out.message);
    show.value = false;
  } catch (e) {
    msg.error(api.errText(e));
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <NButton
    :size="size ?? 'small'"
    :type="type ?? 'default'"
    :secondary="secondary"
    :block="block"
    :loading="busy"
    :title="hint"
    @click="open"
  >
    {{ label }}
  </NButton>

  <NModal
    v-if="show"
    :show="true"
    preset="card"
    :title="free() ? '开 AI 会话' : '派发Paseo任务'"
    style="width: 480px"
    @update:show="(v: boolean) => !v && (show = false)"
    @close="show = false"
  >
    <NSpin :show="loading">
      <p class="muted small note">选择用哪个 Paseo provider（必选，默认第一个）：</p>
      <NRadioGroup v-model:value="provider" :disabled="loading">
        <div class="providers">
          <NRadio v-for="p in providers" :key="p" :value="p">{{ p }}</NRadio>
        </div>
      </NRadioGroup>

      <template v-if="free()">
        <label class="field">会话标题</label>
        <NInput
          v-model:value="title"
          placeholder="如 查线上日志（留空就叫「临时会话」）"
          :input-props="NO_AUTO"
        />
      </template>

      <NAlert
        v-if="!loading && loadError"
        type="error"
        :bordered="false"
        class="mt"
      >
        {{ loadError }}
      </NAlert>
      <NAlert
        v-else-if="!loading && !providers.length"
        type="warning"
        :bordered="false"
        class="mt"
      >
        没有可用的 provider。运行 <code>paseo provider ls</code> 看看：每个 provider
        需要先在 Paseo 里装好对应 CLI 并登录（如 Claude Code / Codex）。
      </NAlert>

      <p class="muted small note">
        工作目录：<code>{{ app.hub?.root }}</code>（hub 根目录，不是需求目录）。
      </p>
      <p v-if="free()" class="muted small note">
        不绑定需求：适合先聊清需求、或查线上日志；要改代码时再去「需求」页新建。
      </p>
    </NSpin>
    <template #footer>
      <div class="foot">
        <NButton size="small" @click="show = false">取消</NButton>
        <NButton
          size="small"
          type="primary"
          :loading="busy"
          :disabled="!provider || loading"
          @click="start"
        >
          启动
        </NButton>
      </div>
    </template>
  </NModal>
</template>

<style scoped>
.note {
  margin: 12px 0 8px;
  line-height: 1.6;
}
.providers {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.field {
  display: block;
  margin: 12px 0 6px;
  font-size: 12px;
  color: #888;
}
.mt {
  margin-top: 10px;
}
.foot {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>
