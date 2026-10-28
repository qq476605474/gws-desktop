<script setup lang="ts">
//! 「用编辑器打开」—— 每个「打开目录」旁边都放一个。
//!
//! 用设置里配置的**常用编辑器列表**（有序）：
//! - 没配置 → 提示去设置里加（点了没反应比提示更让人困惑）；
//! - 只配一个 → 直接用它打开；
//! - 配了多个 → 下拉列出全部，顺序就是设置里的顺序。
//!
//! 失败会通过 bridge 的 `visible` 弹提示，不静默。

import { computed } from "vue";
import { NButton, NDropdown } from "naive-ui";

import { openInEditor } from "../bridge";
import { CUSTOM_ID, displayLabel, usableEditors, type EditorEntry } from "../lib/editors";
import { notifyWarning } from "../lib/notify";
import { useAppStore } from "../stores/app";

const props = withDefaults(
  defineProps<{
    path: string;
    size?: "tiny" | "small" | "medium" | "large";
    disabled?: boolean;
    /** 表格行里用 secondary；顶部工具栏用默认样式。 */
    secondary?: boolean;
    /** 按钮文字。结果面板里一行一个模块时要能改，否则一排「用编辑器打开」分不清。 */
    label?: string;
  }>(),
  { size: "small", disabled: false, secondary: true, label: "用编辑器打开" },
);

const app = useAppStore();

/** 自定义条目没填命令的不算数，否则点了必然报错。 */
const usable = computed(() => usableEditors(app.editors));
const options = computed(() =>
  usable.value.map((e) => ({ key: e.key, label: displayLabel(e) })),
);

function launch(entry: EditorEntry) {
  if (!props.path) return;
  void openInEditor(
    props.path,
    entry.id,
    entry.id === CUSTOM_ID ? entry.command.trim() : "",
  );
}

function open() {
  if (!props.path) return;
  if (!usable.value.length) {
    notifyWarning("还没有配置常用编辑器，请到「设置 → 用编辑器打开」里添加");
    return;
  }
  if (usable.value.length === 1) launch(usable.value[0]);
}

function pick(key: string | number) {
  const entry = usable.value.find((e) => e.key === key);
  if (entry) launch(entry);
}
</script>

<template>
  <NDropdown
    v-if="usable.length > 1"
    trigger="click"
    :options="options"
    :disabled="disabled || !path"
    @select="pick"
  >
    <NButton :size="size" :disabled="disabled || !path" :secondary="secondary">
      {{ label }}
    </NButton>
  </NDropdown>
  <NButton
    v-else
    :size="size"
    :disabled="disabled || !path"
    :secondary="secondary"
    @click="open"
  >
    {{ label }}
  </NButton>
</template>