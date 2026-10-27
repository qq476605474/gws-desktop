<script setup lang="ts">
//! 「用编辑器打开」—— 每个「打开目录」旁边都放一个。
//!
//! 用设置里选的编辑器（默认 IDEA）。后端优先调 App 包内的 launcher
//! （GUI 进程的 PATH 里没有 `idea` / `code`），已打开的实例会被切到前台。
//! 失败会通过 bridge 的 `visible` 弹提示，不静默。

import { NButton } from "naive-ui";

import { openInEditor } from "../bridge";
import { useAppStore } from "../stores/app";

const props = withDefaults(
  defineProps<{
    path: string;
    size?: "tiny" | "small" | "medium" | "large";
    disabled?: boolean;
    /** 表格行里用 secondary；顶部工具栏用默认样式。 */
    secondary?: boolean;
  }>(),
  { size: "small", disabled: false, secondary: true },
);

const app = useAppStore();

function open() {
  if (!props.path) return;
  void openInEditor(props.path, app.editor, app.editorCommand);
}
</script>

<template>
  <NButton :size="size" :disabled="disabled || !path" :secondary="secondary" @click="open">
    用编辑器打开
  </NButton>
</template>
