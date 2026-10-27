<script setup lang="ts">
//! 把 Naive UI 的 message 实例装进全局出口（`lib/notify`），**不渲染任何东西**。
//!
//! 它必须挂在 `NMessageProvider` 内部才拿得到 `useMessage()`；App.vue 自己
//! 是 Provider 的父级，所以只能在 App.vue 里放这么一个子组件当锚点。
//!
//! 错误的停留时间比默认的 3 秒长：这类消息（路径不对、系统拒绝打开）
//! 用户往往要照着念给别人或截图，一闪而过等于没提示。

import { useMessage } from "naive-ui";

import { installNotifier } from "../lib/notify";

const msg = useMessage();

installNotifier({
  error: (t) => msg.error(t, { duration: 8000 }),
  warning: (t) => msg.warning(t),
  success: (t) => msg.success(t),
});
</script>

<template>
  <!-- 功能组件：无可见输出 -->
</template>