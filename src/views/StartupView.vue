<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import {
  NButton,
  NCard,
  NCheckbox,
  NInput,
  NList,
  NListItem,
  NModal,
  NTag,
  useMessage,
} from "naive-ui";

import { NO_AUTO } from "../lib/inputs";
import * as api from "../bridge";
import { useAppStore } from "../stores/app";
import { withBusy } from "../busy";
import type { HubInfo } from "../types";

const emit = defineEmits<{ (e: "opened"): void }>();
const app = useAppStore();
const msg = useMessage();

const recent = ref<string[]>([]);
const opening = ref(false);
const err = ref("");

// ── 新建 hub ──
const showNew = ref(false);
const parentDir = ref("");
/** hub 目录名。**必须参与路径拼接**——反馈 #1 就是这一步缺失导致的。 */
const dirName = ref("");
const newEnvs = ref<string[]>(["dev"]);
const creating = ref(false);

/**
 * GitLab 绑定三件套。
 *
 * **建 hub 时就得填全**：地址与文档库地址落盘后不可改，而 token 决定
 * 之后所有操作以谁的身份进行。建一个没绑定的 hub，用户加仓库时才发现
 * 什么都干不了，那时再补绑会让人以为工具坏了。
 */
const gitlabUrl = ref("");
const gitlabToken = ref("");
const dochubUrl = ref("");
/**
 * 提交身份（`user.name` / `user.email`）。
 *
 * 建 hub 时一并填好 —— 它和 token 一样**之后随时可改**（不像地址那样锁定）。
 * 留空则用 git 全局配置；打开对话框时会拿全局身份**预填**，多数人不用手打。
 */
const gitName = ref("");
const gitEmail = ref("");
/** 「测试连接」的状态：null = 没测过。 */
const probe = ref<{ ok: boolean; text: string } | null>(null);
const probing = ref(false);

/** 最终会创建在哪。**实时显示**，避免「填了名称却建在别处」。 */
const fullPath = computed(() => {
  const parent = parentDir.value.trim().replace(/[/\\]+$/, "");
  const name = dirName.value.trim();
  if (!parent) return "";
  if (!name) return parent;
  const sep = parent.includes("\\") && !parent.includes("/") ? "\\" : "/";
  return `${parent}${sep}${name}`;
});

const canCreate = computed(
  () =>
    !!parentDir.value.trim() &&
    !!dirName.value.trim() &&
    !!gitlabUrl.value.trim() &&
    !!gitlabToken.value.trim() &&
    !!dochubUrl.value.trim(),
);

/** 改动了地址或 token 就把上一次的测试结果作废 —— 否则会显示一个过期的「连接成功」。 */
function invalidateProbe() {
  probe.value = null;
}

/**
 * 三项一起测：地址、token、文档库。
 *
 * 文档库填错了**现在就得知道** —— 它和地址一样建完不可改，等创建时才失败
 * 会把 hub 弄成半成品（用户只能删掉重来）。
 */
async function testConnection() {
  if (!gitlabUrl.value.trim() || !gitlabToken.value.trim()) return;
  probing.value = true;
  err.value = "";
  const doc = dochubUrl.value.trim();
  try {
    const [name, username] = await api.gitlabProbe(
      gitlabUrl.value.trim(),
      gitlabToken.value.trim(),
      doc,
    );
    probe.value = {
      ok: true,
      text: `连接成功：${name}（${username}）${doc ? "；文档库可用" : ""}`,
    };
  } catch (e) {
    probe.value = { ok: false, text: api.errText(e) };
  } finally {
    probing.value = false;
  }
}

onMounted(async () => {
  try {
    const raw = localStorage.getItem("recentHubs");
    recent.value = raw ? (JSON.parse(raw) as string[]) : [];
  } catch {
    // 存的东西坏了就当没有——启动失败在这里不值得打断
    recent.value = [];
  }
});

/** 从「最近打开」里删掉一条（不影响磁盘上的任何东西）。 */
function forget(path: string) {
  const next = recent.value.filter((p) => p !== path);
  recent.value = next;
  localStorage.setItem("recentHubs", JSON.stringify(next));
  // 如果删掉的正是「上次打开的」，也一并清掉 —— 否则下次启动还会去开它
  if (localStorage.getItem("lastHub") === path) localStorage.removeItem("lastHub");
}

function remember(path: string) {
  const next = [path, ...recent.value.filter((p) => p !== path)].slice(0, 5);
  recent.value = next;
  localStorage.setItem("recentHubs", JSON.stringify(next));
  // 单独存一份「上次用的」，用于下次启动自动打开（反馈 #15）
  localStorage.setItem("lastHub", path);
}

async function pickDir(): Promise<string | null> {
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const r = await open({ directory: true, multiple: false, title: "选择目录" });
    return typeof r === "string" ? r : null;
  } catch {
    // 浏览器里跑 dev 时没有对话框，回退到手输
    return null;
  }
}

async function doOpen(path: string) {
  if (!path.trim() || opening.value) return;
  opening.value = true;
  err.value = "";
  try {
    const info = await api.openHub(path.trim());
    finish(info);
  } catch (e) {
    err.value = api.errText(e);
  } finally {
    opening.value = false;
  }
}

async function chooseExisting() {
  const dir = await pickDir();
  if (dir) void doOpen(dir);
}

async function pickParent() {
  const dir = await pickDir();
  if (dir) {
    parentDir.value = dir;
    if (!dirName.value) dirName.value = "my-hub";
  }
}

function openNewDialog() {
  showNew.value = true;
  parentDir.value = "";
  dirName.value = "my-hub";
  err.value = "";
  gitlabUrl.value = "";
  gitlabToken.value = "";
  dochubUrl.value = "";
  probe.value = null;
  // 拿 git 全局身份预填（读不到就留空 = 用全局配置，不必手打）
  void api
    .globalGitIdentity()
    .then(([n, e]) => {
      gitName.value = n ?? "";
      gitEmail.value = e ?? "";
    })
    .catch(() => {
      // 读不到不算错（没配全局身份很正常），留空即可
    });
}

async function doInit() {
  if (!canCreate.value || creating.value) return;
  creating.value = true;
  err.value = "";
  try {
    const info = await withBusy("正在创建 hub", () =>
      api.initHub({
        path: fullPath.value,
        name: dirName.value.trim(),
        envs: newEnvs.value,
        force: false,
        // 三件套一起给：core 会在**建目录之前**校验连通性，
        // 地址/token 不对时磁盘上不会留下任何东西
        gitlab_url: gitlabUrl.value.trim(),
        gitlab_token: gitlabToken.value.trim(),
        dochub_url: dochubUrl.value.trim(),
        // 提交身份：不强制填（留空 = 用 git 全局配置），但建完就能直接提交
        git_name: gitName.value.trim(),
        git_email: gitEmail.value.trim(),
      }),
    );
    showNew.value = false;

    // 文档库**无条件拉取**（反馈 #4）：它是最最高频的东西——每个需求都要写文档，
    // 而常用仓库是按需的。放在这里一次拉好，免得每个人第一次写文档时再等一次。
    try {
      // 这一步也会把文档库检出到各环境（它在 repos/ 下就是个普通仓库）
      const r = await withBusy("拉取文档库", () => api.dochubEnsure());
      msg.success("文档库已就绪");
      if (r.env_created.length) msg.success(`已在环境里检出 ${r.env_created.length} 份`);
      for (const [what, why] of r.env_failures) {
        msg.warning(`环境检出失败 ${what}：${why}`);
      }
    } catch (e) {
      msg.warning(`文档库未拉取成功（之后可在「文档」页重试）: ${api.errText(e)}`);
    }

    finish(info);
  } catch (e) {
    err.value = api.errText(e);
  } finally {
    creating.value = false;
  }
}

function finish(info: HubInfo) {
  remember(info.root);
  app.setHub(info);
  void app.refreshAll();
  // 打开时可能做过目录迁移 / 元数据清理 —— 逐条说清「我动了什么」，
  // 否则用户会以为文件是自己少掉的
  for (const note of info.migration_notes ?? []) {
    if (note.startsWith("⚠")) msg.error(note.slice(1).trim());
    else msg.info(note, { duration: 8000 });
  }
  emit("opened");
}
</script>

<template>
  <div class="wrap">
    <div class="hero">
      <h1>GWS</h1>
      <p class="sub">多仓库需求开发管理：拉代码、合环境、提 MR、写文档</p>
    </div>

    <div class="cards">
      <NCard class="card" title="打开已有 hub">
        <div class="card-body">
          <NList v-if="recent.length" hoverable clickable>
            <NListItem v-for="p in recent" :key="p" @click="doOpen(p)">
              <div class="recent-row">
                <div class="recent-path" :title="p">{{ p }}</div>
                <NTag size="small" :bordered="false">最近</NTag>
                <!-- 删掉这条记录。目录被搬走/删掉之后，这里会留一条永远打不开的
                     死路径，而以前**没法删**，只能等它被新记录挤出去（实测反馈）。 -->
                <button class="forget" title="从列表里删掉" @click.stop="forget(p)">×</button>
              </div>
            </NListItem>
          </NList>
          <p v-else class="muted">还没有打开过 hub</p>
        </div>
        <template #action>
          <NButton :disabled="opening" @click="chooseExisting">选择目录…</NButton>
        </template>
      </NCard>

      <NCard class="card" title="新建 hub">
        <div class="card-body">
          <p class="muted">
            hub 是统一管理所有仓库与需求工作区的根目录。建议放在容量充足的本地盘，
            例如 <code>~/Documents/hub</code>。
          </p>
        </div>
        <template #action>
          <NButton type="primary" :disabled="opening" @click="openNewDialog">新建…</NButton>
        </template>
      </NCard>
    </div>

    <p v-if="err" class="err">{{ err }}</p>

    <NModal
      v-model:show="showNew"
      preset="card"
      title="新建 hub"
      style="width: 640px"
      :mask-closable="!creating"
    >
      <div class="form">
        <label>建在哪</label>
        <div class="row">
          <NInput
            v-model:value="parentDir"
            :input-props="NO_AUTO"
            placeholder="选择父目录"
          />
          <NButton @click="pickParent">选择…</NButton>
        </div>

        <label>目录名</label>
        <NInput v-model:value="dirName" :input-props="NO_AUTO" placeholder="如 my-hub" />

        <label>完整路径</label>
        <div class="path-preview">
          <code v-if="fullPath">{{ fullPath }}</code>
          <span v-else class="muted">先选父目录并填写目录名</span>
        </div>
        <p class="muted small">
          hub 会建在这个路径下。若该目录已存在且非空，会拒绝创建（避免覆盖你的文件）。
        </p>

        <label>环境</label>
        <div class="envs">
          <NCheckbox
            :checked="newEnvs.includes('dev')"
            @update:checked="
              (c: boolean) => (newEnvs = c ? [...newEnvs, 'dev'] : newEnvs.filter((e) => e !== 'dev'))
            "
          >
            dev（开发）
          </NCheckbox>
          <NCheckbox
            :checked="newEnvs.includes('pre')"
            @update:checked="
              (c: boolean) => (newEnvs = c ? [...newEnvs, 'pre'] : newEnvs.filter((e) => e !== 'pre'))
            "
          >
            pre（预发）
          </NCheckbox>
        </div>

        <label>GitLab 地址</label>
        <div class="row">
          <NInput
            v-model:value="gitlabUrl"
            :input-props="NO_AUTO"
            placeholder="https://gitlab.example.com"
            @update:value="invalidateProbe"
          />
          <NButton :loading="probing" :disabled="!gitlabUrl.trim() || !gitlabToken.trim()" @click="testConnection">
            测试连接
          </NButton>
        </div>

        <label>GitLab token</label>
        <NInput
          v-model:value="gitlabToken"
          type="password"
          show-password-on="click"
          :input-props="NO_AUTO"
          placeholder="Personal Access Token（需要 api 权限）"
          @update:value="invalidateProbe"
        />
        <p v-if="probe" class="small" :class="probe.ok ? 'probe-ok' : 'err'">{{ probe.text }}</p>
        <p v-else class="muted small">
          在 GitLab 的「用户设置 → 访问令牌」里创建，勾选 <code>api</code> 权限。
        </p>

        <label>文档库地址</label>
        <NInput
          v-model:value="dochubUrl"
          :input-props="NO_AUTO"
          placeholder="https://gitlab.example.com/你的组/文档库.git"
        />
        <p class="muted small">
          一个普通的 git 仓库，需求文档按分支存在里面。没有就先建一个空仓库。
        </p>

        <label>提交身份</label>
        <div class="row">
          <NInput
            v-model:value="gitName"
            :input-props="NO_AUTO"
            placeholder="名字（如 张三）"
          />
          <NInput
            v-model:value="gitEmail"
            :input-props="NO_AUTO"
            placeholder="邮箱（如 zhangsan@example.com）"
          />
        </div>
        <p class="muted small">
          在这个 hub 里提交代码时用的 <code>user.name</code> / <code>user.email</code>
          （已按你 git 全局配置预填）。<strong>跟全局配置解耦</strong> ——
          同一台机器上有多套身份时不会串；留空则用全局配置。
        </p>

        <p class="muted small always">
          <strong>地址与文档库地址一旦创建就不可修改</strong>（它们是「哪些仓库能纳管」和
          文档挂载位置的依据）；<strong>token 与提交身份之后随时可以改</strong>
          （「设置 → GitLab 与身份」）。
        </p>
      </div>

      <p v-if="err" class="err">{{ err }}</p>

      <template #footer>
        <div class="footer">
          <NButton :disabled="creating" @click="showNew = false">取消</NButton>
          <NButton type="primary" :loading="creating" :disabled="!canCreate" @click="doInit">
            创建
          </NButton>
        </div>
      </template>
    </NModal>
  </div>
</template>

<style scoped>
.wrap {
  max-width: 700px;
  margin: 0 auto;
  padding: 9vh 20px 40px;
}
.hero {
  text-align: center;
  margin-bottom: 28px;
}
.hero h1 {
  margin: 0;
  font-size: 26px;
  font-weight: 600;
  letter-spacing: 0.01em;
}
.sub {
  color: #888;
  margin: 8px 0 0;
  font-size: 13px;
}
/* 两张卡片**上下两行**，不要并排两列：并排时宽度被压窄，
   路径这类长文本一折行整个卡片就高矮不一（实测反馈） */
.cards {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.card {
  display: flex;
  flex-direction: column;
}
.card-body {
  flex: 1;
}
.muted {
  color: #888;
  font-size: 12px;
  margin: 0;
}
.small {
  font-size: 12px;
  margin-top: 2px;
}
.always {
  color: #18a058;
}
.err {
  color: #d03050;
  font-size: 13px;
  white-space: pre-wrap;
  margin-top: 12px;
}
/* 「测试连接」通过时的提示：与 `.err` 同样的排版，但用成功色 */
.probe-ok {
  color: #18a058;
  margin-top: 12px;
}
.form {
  display: grid;
  grid-template-columns: 6em 1fr;
  gap: 10px 12px;
  align-items: center;
  font-size: 13px;
}
/*
 * 表单里的说明段落**跨两列**。
 *
 * 不加这条的话，`<p>` 会被当成一个普通的 grid 项塞进第一列（6em 宽），
 * 文字被挤成每行三四个字的竖排 —— 而第二列会空着，整个表单跟着错位。
 * 早先的写法是在每个 `<p>` 前面插一个空的 `<label></label>` 占位，
 * 但那样每加一段说明都要记得配一个，漏一个就是这种错位。
 * 用 CSS 收口，谁都不必记得。
 */
.form > p {
  grid-column: 1 / -1;
  margin: 0;
}
.row {
  display: flex;
  gap: 8px;
}
.path-preview {
  font-size: 12px;
  word-break: break-all;
}
/* 删除按钮：hover 才明显，别在列表里抢注意力 */
.forget {
  border: none;
  background: none;
  cursor: pointer;
  font-size: 14px;
  line-height: 1;
  padding: 0 4px;
  border-radius: 4px;
  color: inherit;
  opacity: 0.4;
  flex: none;
}
.forget:hover {
  opacity: 1;
  background: rgba(128, 128, 128, 0.18);
}
.recent-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
}
.recent-path {
  font-family: var(--mono);
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.envs {
  display: flex;
  gap: 18px;
}
.reco {
  display: flex;
  flex-direction: column;
}
.footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>