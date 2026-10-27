<script setup lang="ts">
//! 分支基线来源的**有序列表编辑器**（实测反馈：希望更好用）。
//!
//! # 要解决的问题
//!
//! 原来是一个逗号分隔的输入框 —— 顺序即优先级，但：
//! - 看不出顺序（要自己数逗号）
//! - 挪动顺序只能靠删了重敲
//! - 乱填不会当场报错，等到建需求/同步基线时才以奇怪的方式失败
//!
//! # 规则（与 core 的 `normalise_base_chain` 一致）
//!
//! - **留空 = 主干**（主干是缺省语义，不是要填的值，所以「主干」这个写法会被丢掉）
//! - 每项可以是：`ws:<上游分支名>`（另一个需求工作区）或一个分支名（可带 `origin/`）
//! - **顺序即优先级**：前面命不中才往下走
//! - 命名要合法；**重复要拦住**（静默接受再在别处失败最难查）

import { computed } from "vue";
import { NButton, NInput } from "naive-ui";

import { NO_AUTO } from "../lib/inputs";

const props = defineProps<{ modelValue: string[] }>();
const emit = defineEmits<{ (e: "update:modelValue", v: string[]): void }>();

/**
 * 校验一项。返回错误文案；空串 = 合法。
 *
 * 规则跟 core 保持一致（那边是权威，前端只是**提前**告诉人，省一次往返）。
 * 不做完整 `check-ref-format`，够挡住手误即可。
 */
function checkOne(raw: string): string {
  const s = raw.trim();
  if (!s) return "";
  const target = s.startsWith("ws:") ? s.slice(3).trim() : s;
  if (!target) return "`ws:` 后面要跟上上游需求的分支名";
  if (/\s/.test(target)) return "分支名里不能有空格";
  if (target.includes("..")) return "不能包含连续的两个点 `..`";
  if (target.includes("@{")) return "不能包含 `@{`";
  if (target.includes("//")) return "不能包含连续的两个斜杠";
  const bad = [..."~^:?*[\\"].find((c) => target.includes(c));
  if (bad) return `不能包含字符 \`${bad}\``;
  if (target.startsWith("-") || target.startsWith("/")) return "不能以 `-` 或 `/` 开头";
  if (target.endsWith("/") || target.endsWith(".")) return "不能以 `/` 或 `.` 结尾";
  if (target.endsWith(".lock")) return "不能以 `.lock` 结尾";
  return "";
}

/** 每一项的校验结果（与 modelValue 同长度）。 */
const errors = computed(() => props.modelValue.map(checkOne));

/** 重复项的下标（第二条起都算重复）。 */
const dups = computed(() => {
  const seen = new Set<string>();
  return props.modelValue.map((v) => {
    const s = v.trim();
    if (!s) return false;
    if (seen.has(s)) return true;
    seen.add(s);
    return false;
  });
});

/** 汇总成一句话，给父组件判断能不能保存。 */
const firstError = computed(() => {
  const i = errors.value.findIndex((e) => e);
  if (i >= 0) return `第 ${i + 1} 项：${errors.value[i]}`;
  const d = dups.value.findIndex(Boolean);
  if (d >= 0) return `第 ${d + 1} 项与前面重复`;
  return "";
});

defineExpose({ firstError });

function update(next: string[]) {
  emit("update:modelValue", next);
}

function setItem(i: number, v: string) {
  update(props.modelValue.map((x, idx) => (idx === i ? v : x)));
}

function add() {
  update([...props.modelValue, ""]);
}

function remove(i: number) {
  update(props.modelValue.filter((_, idx) => idx !== i));
}

function move(i: number, delta: number) {
  const next = [...props.modelValue];
  const j = i + delta;
  if (j < 0 || j >= next.length) return;
  [next[i], next[j]] = [next[j], next[i]];
  update(next);
}
</script>

<template>
  <div class="base">
    <!-- 有序列表：**从上到下就是优先级** -->
    <div v-for="(item, i) in modelValue" :key="i" class="row">
      <span class="ord">{{ i + 1 }}</span>
      <NInput
        :value="item"
        size="small"
        :status="errors[i] ? 'error' : dups[i] ? 'warning' : undefined"
        placeholder="如 feature-260901-xxx，或 ws:上游需求分支名"
        :input-props="NO_AUTO"
        @update:value="(v: string) => setItem(i, v)"
      />
      <NButton size="tiny" :disabled="i === 0" title="上移（更优先）" @click="move(i, -1)">
        ↑
      </NButton>
      <NButton
        size="tiny"
        :disabled="i === modelValue.length - 1"
        title="下移"
        @click="move(i, 1)"
      >
        ↓
      </NButton>
      <NButton size="tiny" type="error" title="移除" @click="remove(i)">删</NButton>
    </div>

    <div class="row">
      <NButton size="tiny" @click="add">＋ 加一条</NButton>
      <span class="muted small">从上到下就是优先级：前面的命不中才往下走。</span>
    </div>

    <p v-if="!modelValue.length" class="muted small">
      没有配置 = <strong>用主干</strong>。主干是缺省语义，不用专门填一项「主干」。
    </p>

    <!-- 逐条把错误点出来（汇总那条是给父组件做保存判断的，不再重复显示） -->
    <div v-for="(e, i) in errors" :key="'e' + i" class="err small">
      <template v-if="e">第 {{ i + 1 }} 项：{{ e }}</template>
    </div>
  </div>
</template>

<style scoped>
.base {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.row {
  display: flex;
  align-items: center;
  gap: 6px;
}
/* 序号让人一眼看出"谁在前" —— 顺序就是优先级 */
.ord {
  flex: none;
  width: 1.4em;
  text-align: right;
  color: #888;
  font-size: 12px;
}
.muted {
  color: #888;
}
.small {
  font-size: 12px;
}
.err {
  color: #d03050;
  font-size: 12px;
  margin: 0;
}
</style>