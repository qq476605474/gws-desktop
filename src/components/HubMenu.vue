<script setup lang="ts">
//! 左上角的 hub 名字 + 下拉菜单。
//!
//! 实测反馈：以前「改名」是名字旁一个很淡的铅笔、「切换 hub」是左下角单独
//! 一行的裸链接，左上角还被更新提示挤成两行大胶囊，又乱又丑。
//! 现在低频操作统一收进 hub 名字的菜单：改个名字 / 切换 hub / 打开 hub 目录。
//!
//! # 为什么整块放在子组件里
//!
//! 改名要弹提示，必须用 `useMessage()`；而 App.vue 正是 `NMessageProvider`
//! 的提供方，在里面调会把整个界面崩成白屏（踩过，见
//! `scripts/check-provider-hooks.mjs`）。所以菜单和改名弹窗都放这里。

import { NButton, NDropdown, NInput, NModal, useMessage } from "naive-ui";
import type { DropdownOption } from "naive-ui";
import { ref } from "vue";

import * as api from "../bridge";
import { openInFinder } from "../bridge";
import { NO_AUTO } from "../lib/inputs";
import { useAppStore } from "../stores/app";

const app = useAppStore();
const msg = useMessage();

const showRename = ref(false);
const draft = ref("");
const saving = ref(false);

const options: DropdownOption[] = [
  { label: "改个名字", key: "rename" },
  { label: "切换 hub", key: "switch" },
  // 「访达」是 Mac 专有词；「打开文件管理器」又容易被理解成只启动应用。
  // 叫「打开 hub 目录」最准确：打开的就是当前 hub 这个目录（各平台各自的文件管理器）。
  { label: "打开 hub 目录", key: "open-root" },
];

function onSelect(key: string | number) {
  const k = String(key);
  if (k === "rename") {
    draft.value = app.hub?.name ?? "";
    showRename.value = true;
    return;
  }
  if (k === "switch") {
    // **后端也要放掉这个 hub**：只清前端的话，后端会话态还拿着旧 hub ——
    // 切回启动页后任何「读当前 hub」的命令仍会报上一个 hub（设置页会拿着旧
    // hub 的地址与凭据说「连接成功」）。失败也照样切：留着更糟。
    void api.closeHub().catch(() => undefined);
    app.setHub(null);
    return;
  }
  if (k === "open-root" && app.hub) void openInFinder(app.hub.root);
}

async function save() {
  const name = draft.value.trim();
  if (!name) {
    msg.warning("名字不能为空");
    return;
  }
  saving.value = true;
  try {
    const info = await api.hubRename(name);
    app.setHub(info);
    msg.success("名字已改");
    showRename.value = false;
  } catch (e) {
    msg.error(api.errText(e));
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <NDropdown trigger="click" placement="bottom-start" :options="options" @select="onSelect">
    <button
      type="button"
      class="hub-trigger"
      aria-haspopup="menu"
      :aria-label="`${app.hub?.name ?? '当前 hub'}：改名、切换 hub、打开目录`"
      :title="`${app.hub?.root ?? ''}（点开：改名 / 切换 hub / 打开目录）`"
    >
      <span class="hub-name">{{ app.hub?.name }}</span>
      <svg class="chev" viewBox="0 0 12 12" aria-hidden="true">
        <path d="M3 4.5 6 7.5 9 4.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
    </button>
  </NDropdown>

  <NModal
    v-model:show="showRename"
    preset="card"
    title="改 hub 的名字"
    style="width: 460px"
    @close="showRename = false"
  >
    <NInput
      v-model:value="draft"
      :input-props="NO_AUTO"
      placeholder="比如 my-hub"
      @keyup.enter="save"
    />
    <p class="muted small note">
      只是界面上的显示名（存在 hub 的 <code>gws.json</code>），不会改目录名，
      也不影响任何路径。
    </p>
    <template #footer>
      <div class="footer">
        <NButton @click="showRename = false">取消</NButton>
        <NButton type="primary" :loading="saving" @click="save">保存</NButton>
      </div>
    </template>
  </NModal>
</template>

<style scoped>
/* 整行都是入口：名字在左、箭头在右，hover 整行浮起。
   胶囊边框太重（实测反馈「很丑」），改用工作区切换器那种安静的行样式。 */
.hub-trigger {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  box-sizing: border-box;
  width: calc(100% + 12px);
  min-width: 0;
  margin-left: -6px;
  padding: 3px 6px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: #666;
  font-size: 12px;
  line-height: 18px;
  cursor: pointer;
  transition:
    background 0.15s,
    color 0.15s;
}
.hub-trigger:hover {
  background: rgba(128, 128, 128, 0.12);
  color: #222;
}
.hub-trigger:focus-visible {
  outline: 2px solid #18a058;
  outline-offset: 1px;
}
.hub-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.chev {
  flex: none;
  width: 12px;
  height: 12px;
  opacity: 0.85;
}
.muted {
  color: #888;
}
.small {
  font-size: 12px;
}
.note {
  margin: 8px 0 0;
}
.footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>
