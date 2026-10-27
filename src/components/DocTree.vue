<script setup lang="ts">
//! 文档目录树 —— 用 Naive UI 的 `NTree`，默认全展开。
//!
//! # 为什么换成 NTree
//!
//! 原来是手搓的 `<div>` 列表：缩进、行距、层级感全靠自己调，调了几轮还是
//! 「说不出的丑」（实测反馈），而且**没有展开箭头、没有整行 hover、没有引导线**
//! —— 这些 NTree 本来就自带，还跟界面其它部分观感一致。
//!
//! 默认全展开：每个分支的文档本来就不多，但都很重要 —— 收起来反而要多点几下。

import { computed, h } from "vue";
import { NTree } from "naive-ui";
import type { TreeOption } from "naive-ui";

import { openPath } from "../bridge";
import type { DocNode } from "../types";
import { isEmptyDir, isLeafNode } from "../lib/docTree";

const props = defineProps<{ nodes: DocNode[] }>();

/** `DocNode` → `NTree` 的 `TreeOption`。`key` 用绝对路径（天然唯一）。 */
function toOptions(nodes: DocNode[]): TreeOption[] {
  return nodes.map((n) => ({
    key: n.abs,
    label: n.name,
    // **空目录也必须是叶子**，否则 NTree 把它当成「异步待加载」节点，
    // 展开时转圈且**永不结束**（判定与原因见 `lib/docTree`，那里有测试）。
    isLeaf: isLeafNode(n),
    children: n.dir && n.children?.length ? toOptions(n.children) : undefined,
    prefix: n.dir ? () => h("span", "📁") : undefined,
    // 空目录标一下：否则看不出它是"空的"还是"没读出来"
    suffix: isEmptyDir(n) ? () => h("span", { class: "empty" }, "（空）") : undefined,
  }));
}

const data = computed(() => toOptions(props.nodes));

/**
 * 文件名渲染成可点链接（目录保持普通文字 —— 点它是展开/收起）。
 *
 * `selectable: false` + 自己处理点击：否则选中态会把整行染蓝，而这里
 * 「选中」没有意义（点文件就是打开它）。
 */
function renderLabel({ option }: { option: TreeOption }) {
  if (option.isLeaf) {
    return h(
      "span",
      { class: "file-link", onClick: () => openPath(String(option.key)) },
      String(option.label),
    );
  }
  return option.label as string;
}
</script>

<template>
  <NTree
    :data="data"
    :selectable="false"
    block-line
    default-expand-all
    :render-label="renderLabel"
  />
</template>

<style scoped>
/* 文件名做成链接的样子 —— 点它打开文件（目录名点的是展开/收起，样式不动） */
:deep(.file-link) {
  color: #2563eb;
  cursor: pointer;
}
:deep(.file-link:hover) {
  text-decoration: underline;
}
:deep(.empty) {
  color: #aaa;
  font-size: 12px;
}
</style>