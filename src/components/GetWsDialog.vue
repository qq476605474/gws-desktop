<script setup lang="ts">
import { NO_AUTO } from "../lib/inputs";
import { computed, ref } from "vue";
import { NAlert, NButton, NInput, NModal, NTag, useMessage } from "naive-ui";

import * as api from "../bridge";
import LinkEditor from "./LinkEditor.vue";
import { useAppStore } from "../stores/app";
import type { Link, SharedRequirement } from "../types";

const emit = defineEmits<{ (e: "close"): void }>();
const msg = useMessage();
const app = useAppStore();
/** 文档库在 hub 里的目录名（从配置读，别写死）。 */
const docName = computed(() => app.hub?.dochub_name ?? "doc-hub");

const branch = ref("");
const title = ref("");
const docFamily = ref("");
const notes = ref("");
const links = ref<Link[]>([]);
const err = ref("");
const submitting = ref(false);

/**
 * 「粘贴对方发来的配置」。
 *
 * # 为什么放在**导入需求**而不是新建需求
 *
 * 实测反馈纠正过我的第一版：这个场景要的是「分支已经推到远程了，我照着
 * 对方发来的配置**拉**下来」—— 那正是「导入需求」的语义（拉取已推送到远程的分支）。
 * 新建需求是另一回事：那条分支还不存在。
 *
 * 而且导入时必须考虑**对方配置里的仓库我本地没有**：按配置里带的地址补拉，
 * 否则模块会少（那才是最要紧的一点）。
 */
const shareText = ref("");
/** 识别出来的那份配置。非 null = 现在走「按对方发来的配置拉取」。 */
const shared = ref<SharedRequirement | null>(null);
const shareErr = ref("");
const recognizing = ref(false);

/** 配置里点名、但本地没有的仓库 —— 拉取时会按配置里的地址自动补建。 */
const missingModules = computed(() =>
  (shared.value?.modules ?? [])
    .filter((m) => !app.repos.some((r) => r.name === m.name))
    .map((m) => m.name),
);

/**
 * 识别粘贴进来的配置，把表单填好。
 *
 * **不直接拉**：先把「要拉什么、本地缺哪几个仓库」摆出来让人确认 ——
 * 补仓库要联网克隆，闷头跑会让人以为卡死。
 */
async function recognize() {
  if (recognizing.value) return;
  recognizing.value = true;
  shareErr.value = "";
  try {
    const s = await api.wsShareParse(shareText.value);
    shared.value = s;
    branch.value = s.branch;
    title.value = s.title;
    docFamily.value = s.doc_family ?? "";
    notes.value = s.notes;
    links.value = s.links.length ? [...s.links] : [];
    err.value = "";
  } catch (e) {
    shared.value = null;
    shareErr.value = api.errText(e);
  } finally {
    recognizing.value = false;
  }
}

function clearShare() {
  shared.value = null;
  shareText.value = "";
  shareErr.value = "";
}

/** 组装要提交的那份配置（人在预览里改过的以改过的为准）。 */
function payloadFromForm(): SharedRequirement {
  const s = shared.value!;
  return {
    schema: s.schema,
    branch: branch.value.trim(),
    title: title.value.trim() || branch.value.trim(),
    // 基线来自对方那份配置（导入时不再扫全部仓库反推，注释见 core 的 share::import）
    base: s.base,
    doc_family: docFamily.value.trim() || null,
    // 原样带回去（信息性字段，后端不看；别因为前端不带而把配置改瘦了）
    doc_hub: s.doc_hub,
    notes: notes.value,
    links: links.value.filter((l) => l.label.trim() || l.url.trim()),
    // 模块集合永远以那份配置为准：对方那边写的是什么，这边就拉什么
    modules: s.modules.map((m) => ({ name: m.name, url: m.url })),
  };
}

async function run() {
  if (submitting.value || !branch.value.trim()) return;
  submitting.value = true;
  err.value = "";
  try {
    if (shared.value) {
      await runShared();
      return;
    }
    const out = await api.wsGet({
      branch: branch.value.trim(),
      title: title.value.trim() || null,
      doc_family: docFamily.value.trim() || null,
      notes: notes.value,
      links: links.value.filter((l) => l.label.trim() || l.url.trim()),
    });
    msg.success(`已拉取需求，命中 ${out.modules.length} 个模块`);
    if (out.not_involved.length) {
      msg.info(`另有 ${out.not_involved.length} 个仓库没有这个分支（正常）`);
    }
    emit("close");
  } catch (e) {
    err.value = api.errText(e);
  } finally {
    submitting.value = false;
  }
}

/**
 * 按对方发来的配置拉取（补仓库 → 建工作区 → 挂文档，都在后端做）。
 *
 * 慢操作，遮罩一直显示到跑完（现在**没有**「后台运行」这个出口 —— 实测反馈：
 * 点了它再去做别的会变成两件事并行，状态说不清）。
 */
async function runShared() {
  const out = await api.wsShareImport(payloadFromForm());
  msg.success(`已拉取需求：${out.branch}（${out.modules.length} 个模块）`);
  if (out.added_repos.length) {
    msg.success(`已自动补拉仓库：${out.added_repos.join("、")}`);
  }
  if (out.not_involved.length) {
    msg.info(`这些模块对方那边也没有该分支，正常：${out.not_involved.join("、")}`);
  }
  // 补进来的仓库**不在任何环境里检出**（导入不替人决定环境分支 —— 分支有覆盖，
  // 见后端 `ws_share_import`）。说清下一步，别让人以后在环境页纳闷。
  if (out.added_repos.length) {
    msg.info("新补的仓库还没在环境里检出，需要时到「环境」页点同步");
  }
  if (out.doc?.attached) msg.success(`文档目录已挂上：${out.doc.relative_dir}`);
  // 远程没有这一期 → **不生成**（生成的那几个文件会挡住之后的 `git pull`）
  else if (out.doc) msg.info("对方还没有把这一期的文档推上来，先不挂文档目录");
  else if (out.doc_error) msg.warning(`文档目录未挂上：${out.doc_error}`);
  emit("close");
}

</script>

<template>
  <NModal :show="true" @update:show="(v: boolean) => !v && emit('close')"
    preset="card"
    title="导入需求"
    style="width: 560px"
    :mask-closable="!submitting"
    @close="emit('close')"
  >
    <NAlert type="info" :bordered="false" class="mb">
      拉取已经推到远程的需求（也可以是你自己在另一台电脑上推的）。<br />
      <span class="small">
        模块集合<strong>自动识别</strong>：扫描所有仓库看谁有这个分支，不用手工指定。
        拉下来之后各模块的上游会指向该远程分支，方便直接 pull / push。
      </span>
    </NAlert>

    <!-- 对方发来的配置：粘进来 → 识别 → 拉取。
         比"念分支名"多出来的关键一点：**对方本地没有的仓库会按配置里的地址补拉**。 -->
    <div v-if="!shared" class="share">
      <NInput
        v-model:value="shareText"
        type="textarea"
        size="small"
        :rows="3"
        placeholder="对方发来的需求配置？整段粘到这里，点「识别」——也可以不用，直接填下面的远程分支名"
        :input-props="NO_AUTO"
      />
      <div class="share-actions">
        <NButton
          size="tiny"
          type="primary"
          secondary
          :loading="recognizing"
          :disabled="!shareText.trim()"
          @click="recognize"
        >
          识别配置
        </NButton>
        <span class="muted small">识别后表单会填好，确认无误再点「拉取」</span>
      </div>
      <NAlert v-if="shareErr" type="error" :bordered="false" class="mt">{{ shareErr }}</NAlert>
    </div>

    <!-- 已识别：把「要拉什么」摆出来（模块集合以这份配置为准） -->
    <div v-else class="share on">
      <div class="share-head">
        <NTag size="tiny" type="success" :bordered="false">按对方发来的配置拉取</NTag>
        <code class="mono">{{ shared.branch }}</code>
        <div class="spacer" />
        <NButton size="tiny" @click="clearShare">不用这份配置</NButton>
      </div>
      <div class="mods-list">
        <div v-for="m in shared.modules" :key="m.name" class="mod-row">
          <span class="mono">{{ m.name }}</span>
          <NTag v-if="missingModules.includes(m.name)" size="tiny" type="warning" :bordered="false">
            本地没有，拉取时自动补建
          </NTag>
          <NTag v-else size="tiny" :bordered="false">已有</NTag>
        </div>
        <!-- 文档库**必须也列出来**：它在配置里不是写在 modules 里，而是用「需求分类」
             表达的（`doc_family`），导入时按分类自动挂上。不显示的话，人一看
             「我有 4 个仓库，怎么只复制了 3 个」（实测反馈）。 -->
        <div v-if="shared.doc_family" class="mod-row">
          <span class="mono">{{ docName }}</span>
          <NTag size="tiny" type="info" :bordered="false">
            文档 · 按需求分类「{{ shared.doc_family }}」自动挂上
          </NTag>
        </div>
      </div>
      <p v-if="missingModules.length" class="muted small">
        补建要联网克隆（{{ missingModules.length }} 个仓库），可能要一会儿 —— 拉取时会显示进度。
      </p>
    </div>

    <div class="form">
      <label>远程分支名</label>
      <NInput v-model:value="branch" placeholder="如 feature-20261010-收银台改版"  :input-props="NO_AUTO"/>
      <label>需求名称</label>
      <NInput v-model:value="title" placeholder="留空同分支名"  :input-props="NO_AUTO"/>
      <label>备注</label>
      <NInput v-model:value="notes" type="textarea" size="small" :rows="2" :input-props="NO_AUTO" />
      <label>自定义信息</label>
      <LinkEditor v-model="links" :placeholders="['如 tapd / 飞书文档', 'https://…']" />
      <label>需求分类</label>
      <NInput v-model:value="docFamily" placeholder="留空按年月归组（如 2026-10）"  :input-props="NO_AUTO"/>
    </div>

    <NAlert v-if="err" type="error" :bordered="false" class="mt">{{ err }}</NAlert>

    <template #footer>
      <div class="footer">
        <NButton :disabled="submitting" @click="emit('close')">取消</NButton>
        <NButton type="primary" :loading="submitting" :disabled="!branch.trim()" @click="run">
          拉取
        </NButton>
      </div>
    </template>
  </NModal>
</template>

<style scoped>
.share {
  margin-bottom: 14px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--xg-border);
}
.share-head {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}
.share-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 6px;
}
.share .spacer {
  flex: 1;
}
.mod-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 2px 0;
}
.form {
  display: grid;
  grid-template-columns: 6em 1fr;
  gap: 10px 12px;
  align-items: center;
  font-size: 13px;
  margin-top: 12px;
}
.mb {
  margin-bottom: 4px;
  font-size: 12px;
}
.small {
  font-size: 12px;
}
.mt {
  margin-top: 12px;
}
.footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>