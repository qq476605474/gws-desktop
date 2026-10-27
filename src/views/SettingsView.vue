<script setup lang="ts">
import { NO_AUTO } from "../lib/inputs";
import { computed, onMounted, ref, watch } from "vue";
import { NAlert, NButton, NCard, NInput, NSelect, NTag, useMessage } from "naive-ui";

import * as api from "../bridge";
import { useUpdater } from "../lib/updater";
import { useCopy } from "../lib/copy";
import { personalAccessTokensUrl } from "../lib/gitlab";
import { useAppStore } from "../stores/app";
import { withBusy } from "../busy";
import type {
  EditorOption,
  GitlabStatus,
  QuickLink,
  TerminalOption,
  ToolSyncOutcome,
  ToolVersions,
} from "../types";

const app = useAppStore();
const msg = useMessage();
const copy = useCopy();

const gl = ref<GitlabStatus | null>(null);
const token = ref("");
/** token 申请页；GitLab 地址还没读到时为 null（按钮禁用）。 */
const patUrl = computed(() => personalAccessTokensUrl(gl.value?.base_url));
/** token 文件位置（告诉用户存哪儿了，省得猜）。 */
const storePath = ref("");
/** 更新检查结果。 */
const upd = ref<import("../types").UpdateInfo | null>(null);
/** 桌面端 / CLI / Skills 三个版本（只读探测）。 */
const tools = ref<ToolVersions | null>(null);
/** 手动同步的结果（桌面端自带同版本，正常情况启动时就自动覆盖了）。 */
const syncing = ref(false);
const syncOut = ref<ToolSyncOutcome | null>(null);
// 一键更新：跟左下角那条提示共用（见 lib/updater）
const {
  installing: updInstalling,
  err: updErr,
  done: updDone,
  install: installUpdate,
} = useUpdater();
const checking = ref(false);
const terms = ref<TerminalOption[]>([{ id: "system", label: "跟随系统" }]);
/** 「用编辑器打开」的候选（后端探测本机装了哪些）。 */
const editors = ref<EditorOption[]>([{ id: "idea", label: "IntelliJ IDEA", installed: true }]);
const editorOpts = computed(() =>
  editors.value.map((e) => ({
    label: e.installed || e.id === "custom" ? e.label : `${e.label}（未安装）`,
    value: e.id,
  })),
);
const saved = ref(false);

/** 作者与项目地址（纯展示信息，点开走系统浏览器）。 */
const GITHUB_AUTHOR = "https://github.com/qq476605474";
const GITHUB_REPO = "https://github.com/qq476605474/gws-desktop";
const BILIBILI = "https://space.bilibili.com/29222259";

/** 「补绑 GitLab」表单：只在这个 hub 还没绑定时用（老 hub / 离线创建）。 */
const bindUrl = ref("");
const bindDoc = ref("");
const bindToken = ref("");
const canBind = computed(
  () => !!bindUrl.value.trim() && !!bindDoc.value.trim() && !!bindToken.value.trim(),
);

async function doBind() {
  try {
    await api.hubBindGitlab(bindUrl.value.trim(), bindDoc.value.trim(), bindToken.value.trim());
    msg.success("已绑定 GitLab");
    bindUrl.value = "";
    bindDoc.value = "";
    bindToken.value = "";
    // 绑定改了 hub 配置（地址 + 文档库），界面上的 hub 信息要跟着刷新
    await app.refreshAll();
    await refreshGl();
  } catch (e) {
    msg.error(api.errText(e));
  }
}

/** 「提交身份」草稿。
 *
 * 跟着 hub 走：保存在 hub 配置里，并写进 hub 内每个仓库的 local config ——
 * hub 里的 worktree 与主 clone 共享同一份 config，所以用户在 `ws/` 里
 * **手工 `git commit`** 用的也是这个身份。两个都留空 = 用 git 全局配置。
 */
const idName = ref("");
const idEmail = ref("");

/**
 * 只在**切换 hub** 时重置草稿。
 *
 * ⚠️ 这里原来监听的是 `app.hub?.git_name` —— 那会在**任何** hub 刷新时触发
 * （切页面、刷新 hub 信息、同步工具版本……），用户正在输入的内容被无声抹掉。
 * 实测反馈：「还没点保存，表单自己重置了」。
 *
 * 现在以 hub **根路径**为键：只有切到别的 hub 才重新填，同一个 hub 内
 * 绝不覆盖用户正在编辑的内容。
 */
watch(
  () => app.hub?.root,
  () => {
    idName.value = app.hub?.git_name ?? "";
    idEmail.value = app.hub?.git_email ?? "";
  },
  { immediate: true },
);

async function saveIdentity() {
  try {
    const info = await api.hubSetIdentity(idName.value.trim(), idEmail.value.trim());
    app.setHub(info);
    const set = !!(idName.value.trim() && idEmail.value.trim());
    msg.success(
      set ? "提交身份已保存，并同步到 hub 内各仓库" : "提交身份已清除，之后用 git 全局配置",
    );
  } catch (e) {
    msg.error(api.errText(e));
  }
}

/**
 * 「快捷入口」草稿。
 *
 * 这些是**用户自己配的**发布页快捷入口（存在 hub 的 `gws.json`）——开源版
 * 没有内置发布系统。**一条都不配，界面就不显示快捷入口**。
 */
const quickDraft = ref<QuickLink[]>([]);

/** 同「提交身份」：**只在切换 hub 时**重置，不覆盖用户正在输入的内容。 */
watch(
  () => app.hub?.root,
  () => {
    quickDraft.value = (app.hub?.quick_links ?? []).map((l) => ({ ...l }));
  },
  { immediate: true },
);

function addQuickLink() {
  quickDraft.value.push({ label: "", url: "" });
}

async function saveQuickLinks() {
  try {
    // 名称或地址没填全的行直接丢掉，别写进配置让校验去报错
    const links = quickDraft.value
      .map((l) => ({ label: l.label.trim(), url: l.url.trim() }))
      .filter((l) => l.label && l.url);
    const info = await api.hubSetQuickLinks(links);
    app.setHub(info);
    msg.success(links.length ? "快捷入口已保存" : "快捷入口已清空，界面将不再显示");
  } catch (e) {
    msg.error(api.errText(e));
  }
}
/**
 * 当前版本号。
 *
 * **从 Tauri 读，不写死** —— 原来这里是硬编码的 `1.0.0`，升版本时必然忘记改，
 * 界面上会一直显示旧版本（而这个数字正是人判断"我装的是哪版"的依据）。
 */
const version = ref("…");
async function loadVersion() {
  try {
    const { getVersion } = await import("@tauri-apps/api/app");
    version.value = await getVersion();
  } catch {
    // 拿不到就留个占位，不编造数字
    version.value = "未知";
  }
}

async function loadTools() {
  try {
    tools.value = await api.toolVersions();
  } catch {
    // 探测失败（比如没有权限跑 --version）不该影响别的设置
  }
}

/** 把桌面端自带的 CLI / Skills 覆盖到本机。改过的 skill 会先备份成 *.gws-backup。 */
async function runSyncTools() {
  syncing.value = true;
  try {
    const out = await api.syncTools();
    syncOut.value = out;
    if (out.changed) msg.success("CLI 与 Skills 已同步");
    else msg.info("已经是最新版本");
    await loadTools();
  } catch (e) {
    msg.error(api.errText(e));
  } finally {
    syncing.value = false;
  }
}

onMounted(async () => {
  void refreshGl();
  void loadVersion();
  void loadTools();
  try {
    terms.value = await api.terminalOptions();
    if (!terms.value.some((t) => t.id === app.terminal)) app.terminal = "system";
  } catch {
    // IPC 失败保留 system 项，下拉仍可用
  }
  try {
    editors.value = await api.editorOptions();
    if (!editors.value.some((e) => e.id === app.editor)) app.editor = "idea";
  } catch {
    // 探测失败保留 IDEA，按钮点击时后端仍会给出明确原因
  }
});

async function refreshGl() {
  try {
    gl.value = await api.gitlabStatus();
    storePath.value = gl.value.store_path;
  } catch (e) {
    gl.value = {
      base_url: "",
      bound: false,
      has_hub: false,
      source: "",
      has_stored_token: false,
      store_path: "",
      user: null,
      error: api.errText(e),
    };
  }
}





/** 保存**不验证**：验证要打网络，网络一慢就卡在「正在验证」好几分钟，
 *  用户以为死了（实测反馈 #13）。要验证点「重新检测」。 */
async function saveToken() {
  if (!token.value.trim()) return;
  try {
    await api.gitlabSaveToken(token.value.trim());
    token.value = "";
    saved.value = true;
    msg.success("token 已保存");
    await refreshGl();
  } catch (e) {
    msg.error(api.errText(e));
  }
}

/** 「重新检测」才做真正的连接验证。 */
async function testConnection() {
  try {
    await withBusy("正在连接 GitLab", refreshGl);
    if (gl.value?.user) msg.success(`连接成功：${gl.value.user[0]}`);
    else msg.warning("连接失败，见下方错误");
  } catch (e) {
    msg.error(api.errText(e));
  }
}

async function doCheckUpdate() {
  checking.value = true;
  try {
    upd.value = await api.checkUpdate();
  } catch (e) {
    msg.error(api.errText(e));
    upd.value = null;
  } finally {
    checking.value = false;
  }
}

/** 一键更新（下载 → 校验 → 装 → 重启），设置页与左下角提示同一套。 */
function copyCmd(cmd: string) {
  void copy(cmd, "命令");
}

async function runUpdate() {
  const out = await installUpdate();
  if (out?.next === "restart") msg.success(`已更新到 ${out.version}，正在重启…`);
  else if (out) msg.info(out.message);
}

async function clearToken() {
  try {
    gl.value = await api.gitlabLogout();
    msg.success("已删除本机保存的 token");
  } catch (e) {
    msg.error(api.errText(e));
  }
}
</script>

<template>
  <div class="page">
    <h2 class="title">设置</h2>

    <NCard size="small" class="card" title="外观与终端">
      <div class="form">
        <label>外观主题</label>
        <NSelect
          v-model:value="app.theme"
          :options="[
            { label: '浅色', value: 'light' },
            { label: '深色', value: 'dark' },
          ]"
        />
        <label>打开终端</label>
        <NSelect v-model:value="app.terminal" :options="terms.map((t) => ({ label: t.label, value: t.id }))" />
      </div>
    </NCard>

    <NCard size="small" class="card" title="用编辑器打开">
      <div class="form">
        <label>默认编辑器</label>
        <NSelect v-model:value="app.editor" :options="editorOpts" />
        <template v-if="app.editor === 'custom'">
          <label>自定义命令</label>
          <NInput
            v-model:value="app.editorCommand"
            size="small"
            placeholder="例如：idea {path}"
            :input-props="NO_AUTO"
          />
        </template>
      </div>
      <p class="muted small">
        模块/仓库旁边的「用编辑器打开」用它，默认 IntelliJ IDEA；编辑器已开着会切到对应窗口。
        自定义命令里的 <code>{path}</code> 会替换成目录（没写就追加在末尾），由系统 shell 执行。
      </p>
    </NCard>

    <NCard size="small" class="card" title="快捷入口">
      <p class="muted small note">
        配好后，总览页和需求详情里会出现对应的跳转按钮；留空则不显示。
      </p>
      <div v-for="(l, i) in quickDraft" :key="i" class="quick-row">
        <NInput
          v-model:value="l.label"
          size="small"
          placeholder="名称（如「打开发布页」）"
          :input-props="NO_AUTO"
        />
        <NInput
          v-model:value="l.url"
          size="small"
          placeholder="https://…"
          :input-props="NO_AUTO"
        />
        <NButton size="small" @click="quickDraft.splice(i, 1)">删除</NButton>
      </div>
      <div class="rel-actions">
        <NButton size="small" @click="addQuickLink">添加一条</NButton>
        <NButton size="small" type="primary" :disabled="!app.hub" @click="saveQuickLinks">
          保存
        </NButton>
      </div>
      <p v-if="!app.hub" class="muted small note">先打开一个 hub 才能配置。</p>
    </NCard>

    <NCard size="small" class="card" title="GitLab 与提交身份">
      <!-- 凭据**跟着 hub 走**：地址在建 hub 时固定、不可改；token 随时可换。
           没打开 hub 时不显示表单 —— 那会让人以为在配一份「全局设置」。 -->
      <p v-if="!gl?.has_hub" class="muted small note">
        还没有打开 hub。先在启动页选择或新建一个 hub ——
        GitLab 地址与 token 都是<strong>跟着 hub</strong> 的，没有全局配置。
      </p>

      <template v-else>
        <div class="kv">
          <span class="k">地址</span>
          <span class="mono">{{ gl.base_url || "（未绑定）" }}</span>
          <NTag v-if="gl.bound" size="tiny" :bordered="false">随 hub 绑定，不可改</NTag>
        </div>

        <!-- 未绑定（老 hub / 离线创建）：给一个补绑入口 -->
        <template v-if="!gl.bound">
          <p class="muted small note">
            这个 hub 还没绑定 GitLab。补绑之后地址同样<strong>不可再改</strong>。
          </p>
          <div class="bind-row">
            <NInput
              v-model:value="bindUrl"
              size="small"
              placeholder="GitLab 地址，如 https://gitlab.example.com"
              :input-props="NO_AUTO"
            />
          </div>
          <div class="bind-row">
            <NInput
              v-model:value="bindDoc"
              size="small"
              placeholder="文档库仓库地址（…/文档库.git）"
              :input-props="NO_AUTO"
            />
          </div>
          <div class="bind-row">
            <NInput
              v-model:value="bindToken"
              size="small"
              type="password"
              show-password-on="click"
              placeholder="Personal Access Token（需要 api 权限）"
              :input-props="NO_AUTO"
            />
            <NButton size="small" type="primary" :disabled="!canBind" @click="doBind">绑定</NButton>
          </div>
        </template>

        <template v-else>
          <div class="gl-status">
            <div class="kv">
              <span class="k">凭据来源</span>
              <span>
                {{ gl.source }}
                <NTag v-if="gl.has_stored_token" size="tiny" :bordered="false" class="ml">已保存</NTag>
              </span>
            </div>
            <div class="kv">
              <span class="k">连接</span>
              <span v-if="gl.user">
                <NTag size="tiny" type="success" :bordered="false">
                  {{ gl.user[0] }}（{{ gl.user[1] }}）
                </NTag>
              </span>
              <span v-else class="err">{{ gl.error }}</span>
            </div>
          </div>

          <div class="token-row">
            <NInput
              v-model:value="token"
              type="password"
              show-password-on="click"
              size="small"
              placeholder="换一个 Personal Access Token（需要 api 权限）"
             :input-props="NO_AUTO"/>
            <NButton size="small" type="primary" :disabled="!token.trim()" @click="saveToken">保存</NButton>
            <NButton size="small" @click="testConnection">重新检测</NButton>
            <NButton v-if="gl.has_stored_token" size="small" @click="clearToken">清除</NButton>
          </div>

          <!-- 实测反馈：不知道 token 去哪儿申请 —— 直接给入口，别让人自己翻设置 -->
          <div class="pat-row">
            <span class="muted small">还没有令牌？在 GitLab 里创建一个，勾选 <code>api</code> 权限即可。</span>
            <NButton
              size="small"
              secondary
              type="primary"
              :disabled="!patUrl"
              @click="patUrl && api.openUrl(patUrl)"
            >
              去申请 Personal Access Token
            </NButton>
          </div>

          <p class="muted small note">
            token 存在<strong>这个 hub</strong> 的凭据文件 <code>{{ storePath }}</code>
            （Unix 上权限 600，仅你可读写）。<br />
            若你配过环境变量 <code>GWS_GITLAB_TOKEN</code> / <code>GITLAB_TOKEN</code>，
            它会优先生效——想改用这里保存的，就把 shell 配置里那一行删掉。
          </p>

          <!-- ── 提交身份：跟 GitLab 配在同一张卡里 ──
               两者都是「这个 hub 以谁的身份与实例打交道」，平时一起改，
               分两张卡反而要来回找。它和 token 一样**随时可改**（地址才锁定）。 -->
          <div class="sub-sec">
            <div class="sub-title">提交身份</div>
            <p class="muted small note">
              在这个 hub 里提交代码时用的 <code>user.name</code> / <code>user.email</code>。<br />
              <strong>跟 git 全局配置解耦</strong> —— 同一台机器上有多套身份
              （个人的 / 工作的 / 不同 GitLab 账号）时不会串。两个都留空则用 git 全局配置。
            </p>
            <div class="bind-row">
              <NInput
                v-model:value="idName"
                size="small"
                placeholder="名字（如 张三）"
                :input-props="NO_AUTO"
              />
              <NInput
                v-model:value="idEmail"
                size="small"
                placeholder="邮箱（如 zhangsan@example.com）"
                :input-props="NO_AUTO"
              />
              <NButton size="small" type="primary" @click="saveIdentity">保存</NButton>
            </div>
          </div>
        </template>
      </template>
    </NCard>

    <NCard size="small" class="card" title="版本与更新">
      <div class="kv"><span class="k">当前版本</span><span class="mono">{{ version }}</span></div>
      <!-- 三个东西各自装在不同位置，落后没落后要能一眼看出来（#7） -->
      <div class="kv">
        <span class="k">桌面端</span>
        <span class="mono">{{ tools?.desktop ?? version }}</span>
      </div>
      <div class="kv">
        <span class="k">命令行</span>
        <span class="mono">{{ tools?.cli ?? "未安装" }}</span>
        <span v-if="tools?.cli_path" class="muted small">{{ tools.cli_path }}</span>
      </div>
      <div class="kv">
        <span class="k">Skills</span>
        <span class="mono">
          {{ tools?.skill_targets?.length ? `${tools.skill_targets.length} 个 AI CLI` : "未检测到 AI CLI" }}
        </span>
      </div>
      <div
        v-for="t in tools?.skill_targets ?? []"
        :key="t.dir"
        class="kv skill-row"
      >
        <span class="k">{{ t.label }}</span>
        <span class="mono">
          {{ t.version ?? (t.installed ? "已安装（未记录版本）" : "未安装") }}
        </span>
        <span class="muted small">{{ t.dir }}</span>
      </div>
      <div v-if="upd" class="kv">
        <span class="k">最新版本</span>
        <span>
          <template v-if="upd.state === 'up_to_date'">
            <NTag size="tiny" type="success" :bordered="false">已是最新</NTag>
          </template>
          <template v-else-if="upd.state === 'available'">
            <NTag size="tiny" type="warning" :bordered="false">有新版 {{ upd.latest }}</NTag>
          </template>
          <template v-else>
            <NTag size="tiny" :bordered="false">
              远程 {{ upd.latest }}（低于当前，不降级）
            </NTag>
          </template>
        </span>
      </div>
      <div class="actions">
        <!-- 桌面端自带同版本的 CLI 与 skill：启动时自动覆盖，这里留一个手动入口 -->
        <NButton size="small" :loading="syncing" @click="runSyncTools">同步 CLI 与 Skills</NButton>
        <NButton size="small" :loading="checking" @click="doCheckUpdate">检查更新</NButton>
        <NButton size="small" @click="api.openReleases()">打开 Releases 页</NButton>
        <!-- 有新版就**直接能装**，不用跳到发布页自己下（实测反馈：
             设置里检查出版本很低，却没有能动手的地方） -->
        <NButton
          v-if="upd?.state === 'available'"
          size="small"
          type="primary"
          :loading="updInstalling"
          @click="runUpdate"
        >
          立即更新
        </NButton>
        <NButton v-if="upd?.release_url" size="small" @click="api.openUrl(upd.release_url)">
          看这一版
        </NButton>
      </div>

      <NAlert v-if="updErr" type="error" :bordered="false" class="mt">{{ updErr }}</NAlert>
      <NAlert
        v-if="syncOut"
        :type="syncOut.changed ? 'success' : 'info'"
        :bordered="false"
        class="mt"
      >
        <div>{{ syncOut.cli }}</div>
        <div>{{ syncOut.skills }}</div>
      </NAlert>
      <NAlert
        v-if="updDone"
        :type="updDone.next === 'restart' ? 'success' : 'info'"
        :bordered="false"
        class="mt"
      >
        <div>{{ updDone.message }}</div>
        <template v-if="updDone.command">
          <div class="cmd mono">{{ updDone.command }}</div>
          <NButton size="tiny" type="primary" class="mt" @click="copyCmd(updDone.command)">
            复制命令
          </NButton>
        </template>
      </NAlert>
      <p class="muted small note">
        桌面端的「立即更新」会下载对应安装包、**对照 release 里的 sha256 校验**、
        再装上去并自动重启；任何一步失败都不会动你现在的安装（最坏是还是旧版）。
        Linux 的 deb 需要 sudo，所以那边只把包下好、命令给出来，人工执行。
      </p>
      <p class="muted small note">
        桌面端自带<strong>同版本</strong>的命令行（<code>gws</code>）与 <code>gws</code> skill。
        安装或升级桌面端后，启动时会自动把它们覆盖到本机（CLI 放进已在用的位置，
        一个都没有就装到 <code>~/.local/bin</code>）。skills 会写进本机<strong>已安装</strong>的
        AI CLI 各自的目录：Claude Code <code>~/.claude/skills</code>、OpenCode
        <code>~/.config/opencode/skills</code>、Qoder CLI <code>~/.qoder/skills</code>、
        Codex <code>~/.codex/skills</code>；没有的目录不新建，也可用
        <code>GWS_SKILLS_DIR</code> 指定。被改过的 skill 会先备份成
        <code>SKILL.md.gws-backup</code>；本机版本比桌面端新时不会被降级。
        <code>GWS_NO_SKILLS=1</code> 可跳过 skills。<br />
        另外，<code>install.sh</code> 与 <code>gws update --apply</code> 也仍然可用，
        适合没有桌面端的机器。
      </p>
    </NCard>

    <NCard size="small" class="card" title="关于">
      <div class="kv">
        <span class="k">作者</span>
        <span class="mono">qq476605474</span>
      </div>
      <p class="muted small note">
        用着有问题、或者想要什么功能，欢迎到 GitHub 提 issue；也欢迎来 B 站看看。
      </p>
      <div class="about-links">
        <NButton size="small" secondary type="primary" @click="api.openUrl(GITHUB_REPO)">
          GitHub 项目主页
        </NButton>
        <NButton size="small" secondary @click="api.openUrl(GITHUB_AUTHOR)">
          作者 GitHub
        </NButton>
        <NButton size="small" secondary @click="api.openUrl(BILIBILI)">
          B 站
        </NButton>
      </div>
    </NCard>

  </div>
</template>

<style scoped>
.page {
  padding: 20px 24px 40px;
  max-width: 860px;
}
.title {
  margin: 0 0 16px;
  font-size: 18px;
  font-weight: 600;
}
.card {
  margin-bottom: 14px;
}
.form {
  display: grid;
  grid-template-columns: 6em 1fr;
  gap: 10px 12px;
  align-items: center;
  font-size: 13px;
  max-width: 420px;
}
.kv {
  display: grid;
  grid-template-columns: 6em 1fr;
  gap: 8px;
  padding: 3px 0;
  font-size: 13px;
  align-items: center;
}
.k {
  color: #888;
}
/* 每个 AI CLI 一行：名字 / 版本 / 目录 */
.skill-row {
  padding-left: 10px;
  grid-template-columns: 6em auto 1fr;
}
.skill-row .small {
  overflow-wrap: anywhere;
}
.ml {
  margin-left: 6px;
}
.err {
  color: #d03050;
  font-size: 12px;
  white-space: pre-wrap;
}
.gl-url {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
}
.url-input {
  max-width: 360px;
}
.gl-status {
  margin-top: 10px;
}
.token-row {
  display: flex;
  gap: 8px;
  margin-top: 12px;
}
.quick-row {
  display: flex;
  gap: 8px;
  margin-top: 10px;
}
/* 卡片内的次级分节（如 GitLab 卡里的「提交身份」） */
.sub-sec {
  margin-top: 16px;
  padding-top: 14px;
  border-top: 1px solid rgba(128, 128, 128, 0.18);
}
.sub-title {
  font-weight: 600;
  font-size: 13px;
  margin-bottom: 6px;
}
/* 「补绑 GitLab」的三行输入 */
.bind-row {
  display: flex;
  gap: 8px;
  margin-top: 8px;
}
/* 「关于」里的几个外链按钮 */
.about-links {
  display: flex;
  gap: 8px;
  margin-top: 12px;
  flex-wrap: wrap;
}
.rel-actions {
  display: flex;
  gap: 8px;
  margin-top: 12px;
}
.pat-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-top: 10px;
  flex-wrap: wrap;
}
.muted {
  color: #888;
}
.small {
  font-size: 12px;
}
.note {
  margin-top: 10px;
}
.actions {
  margin-top: 10px;
}
/* 同步 / 更新结果与按钮拉开距离，否则提示条贴着按钮 */
.mt {
  margin-top: 12px;
}
/* 要人敲的那条命令：等宽 + 可整条选中 */
.cmd {
  margin-top: 8px;
  padding: 6px 8px;
  border-radius: 6px;
  background: rgba(128, 128, 128, 0.12);
  font-size: 12px;
  user-select: all;
  word-break: break-all;
}
</style>