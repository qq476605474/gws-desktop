<script setup lang="ts">
//! 左上角的**更新提示条**：打开软件时异步检查，有新版才出现一条单行提示，
//! 点它弹出更新对话框（一键更新）。没新版时**什么都不渲染**。
//!
//! # 为什么是「异步 + 单行提示」而不是弹窗 / 大胶囊
//!
//! 实测需求：打开软件时**自动**检查。但检查要联网，网络慢起来几秒 ——
//! 启动时弹窗等着会把「打开就能用」这件事毁掉。所以：
//!
//! - 检查在后台跑，**不挡任何操作**
//! - 有新版在左上角挂一条，**单行、左对齐**，不换行、不占两行
//!   （实测反馈：整宽居中的大胶囊把标题都压过去了，很丑）
//! - 没网 / 检查失败**静默**：这不是用户要办的事，不该在启动时报错
//!
//! 当前版本号由 `VersionBadge` 一直显示；这里只管「该更新了」。

import { computed, onMounted, ref } from "vue";
import { NAlert, NButton, NModal, NProgress, NTag, useMessage } from "naive-ui";

import { useCopy } from "../lib/copy";

import * as api from "../bridge";
import { openUrl } from "../bridge";
import { useUpdater } from "../lib/updater";
import type { UpdateInfo } from "../types";

const msg = useMessage();
const copy = useCopy();

const info = ref<UpdateInfo | null>(null);
const show = ref(false);
// 下载 → 校验 → 装 → 重启，这一串跟设置页共用（见 lib/updater）
const { installing, err, done, install: doInstall } = useUpdater();

/** 有新版才显示。 */
const hasUpdate = computed(() => info.value?.state === "available" && !!info.value.latest);

onMounted(async () => {
  // 启动时**异步**查一次：不 await 任何 UI 状态，失败了也不吭声
  try {
    info.value = await api.checkUpdate();
  } catch {
    // 没网 / 网络不通：静默。更新不是用户此刻要办的事，不该在启动时报错
  }
});

async function install() {
  const out = await doInstall();
  if (out?.next === "restart") msg.success(`已更新到 ${out.version}，正在重启…`);
}
</script>

<template>
  <!-- 只有新版才出现；**单行**（超出用省略号，不换行） -->
  <button
    v-if="hasUpdate"
    class="update-bar"
    :title="`当前 ${info?.current}，最新 ${info?.latest}，点击更新`"
    @click="show = true"
  >
    <span class="up">↑</span>
    <span class="text">有新版本 {{ info?.latest }}</span>
  </button>

  <NModal
    v-if="hasUpdate"
    :show="show"
    preset="card"
    title="软件更新"
    style="width: 520px"
    @update:show="(v: boolean) => !v && (show = false)"
  >
    <div class="kv"><span class="k">当前版本</span><span class="mono">{{ info?.current }}</span></div>
    <div class="kv">
      <span class="k">最新版本</span>
      <span>
        <span class="mono">{{ info?.latest }}</span>
        <NTag size="tiny" type="success" :bordered="false" class="ml">可更新</NTag>
      </span>
    </div>
    <div class="kv"><span class="k">安装包</span><span class="mono small">{{ info?.asset }}</span></div>

    <p class="muted small">
      点「下载并更新」会：下载对应安装包 → <strong>对照 release 里的 sha256 校验</strong> →
      装上去（macOS 替换应用本体、Windows 跑安装包）→ 自动重启。<br />
      任何一步失败都<strong>不会动你现在的安装</strong>，最坏结果是还是旧版。
    </p>

    <div v-if="installing" class="prog">
      <NProgress type="line" :percentage="100" :show-indicator="false" :height="4" :processing="true" />
      <span class="muted small">正在下载并安装…进度见右下角。请等它跑完再操作别的</span>
    </div>

    <NAlert v-if="err" type="error" :bordered="false" class="mt">{{ err }}</NAlert>

    <NAlert v-if="done" :type="done.next === 'restart' ? 'success' : 'info'" :bordered="false" class="mt">
      <div>{{ done.message }}</div>
      <!-- Linux：deb 要 sudo，装不了就老实说 —— 并把**要敲的命令**单独摆出来 +
           一键复制（实测反馈：混在一段话里，小白根本不知道怎么下手） -->
      <template v-if="done.command">
        <div class="cmd mono">{{ done.command }}</div>
        <NButton size="tiny" type="primary" class="mt" @click="copy(done.command!, '命令')">
          复制命令
        </NButton>
      </template>
    </NAlert>

    <template #footer>
      <div class="footer">
        <NButton size="small" @click="openUrl(info?.releases_url ?? '')">打开发布页</NButton>
        <NButton size="small" @click="show = false">稍后</NButton>
        <NButton v-if="!done" size="small" type="primary" :loading="installing" @click="install">
          下载并更新
        </NButton>
      </div>
    </template>
  </NModal>
</template>

<style scoped>
/* 浅橙底、左对齐、单行：像一条状态，而不是一个大按钮 */
.update-bar {
  display: flex;
  align-items: center;
  gap: 4px;
  width: 100%;
  margin-top: 8px;
  padding: 4px 8px;
  border: none;
  border-radius: 6px;
  background: rgba(240, 160, 32, 0.12);
  color: #b26a00;
  font-size: 11px;
  line-height: 18px;
  text-align: left;
  white-space: nowrap;
  overflow: hidden;
  cursor: pointer;
}
.update-bar:hover {
  background: rgba(240, 160, 32, 0.22);
}
.update-bar .up {
  font-weight: 700;
}
.update-bar .text {
  overflow: hidden;
  text-overflow: ellipsis;
}
.kv {
  display: flex;
  gap: 8px;
  padding: 2px 0;
  font-size: 13px;
}
.kv .k {
  color: #888;
  min-width: 5em;
}
.ml {
  margin-left: 6px;
}
.prog {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 10px;
}
.mt {
  margin-top: 10px;
}
/* 要人敲的那条命令：等宽 + 可选中，一眼看清是一整条 */
.cmd {
  margin-top: 8px;
  padding: 6px 8px;
  border-radius: 6px;
  background: rgba(128, 128, 128, 0.12);
  font-size: 12px;
  user-select: all;
  word-break: break-all;
}
.footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>
