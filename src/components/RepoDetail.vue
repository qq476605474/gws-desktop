<script setup lang="ts">
//! 仓库详情。
//!
//! 「这个仓库被谁用着」原来挤在列表的一列里，平时是噪声、还占宽度。
//! 收进详情面板：从**仓库**这一侧看它被哪些需求/环境引用，点一下能跳过去。

import { computed, onMounted, ref } from "vue";
import { NButton, NInput, NModal, useMessage } from "naive-ui";

import { NO_AUTO } from "../lib/inputs";
import * as api from "../bridge";
import { openInFinder, openUrl } from "../bridge";
import { useAppStore } from "../stores/app";
import { useCopy } from "../lib/copy";
import EditorButton from "./EditorButton.vue";
import LinkEditor from "./LinkEditor.vue";
import LinkList from "./LinkList.vue";
import type { Link, MatrixRow, RepoRow } from "../types";

const props = defineProps<{ repo: RepoRow }>();
const emit = defineEmits<{ (e: "close"): void; (e: "gotoWs", branch: string): void }>();

/** 「导出配置」正在生成。 */
const sharing = ref(false);

/**
 * 复制这一份仓库配置（clone 地址 + 发布地址 + 自定义信息）。
 *
 * 提示里要说清怎么用：光说「已复制」，人不知道拿它干什么。
 */
async function copyShare() {
  if (sharing.value) return;
  sharing.value = true;
  try {
    const text = await api.repoShare(props.repo.name);
    await copy(text, "仓库配置");
    msg.success("配置已复制；对方粘到「添加仓库 → 粘贴配置文本」里即可导入");
  } catch (e) {
    msg.error(api.errText(e));
  } finally {
    sharing.value = false;
  }
}

const app = useAppStore();
const copy = useCopy();
const msg = useMessage();

// ══ 自定义信息与发布地址（实测反馈 #5）══════════════════════════════════
//
// 备注 + 链接（tapd / 飞书 / 域名 / 监控），以及各环境的访问地址。
// 存 `deployments.json` —— 它是「经常批量编辑的数据」，与结构性配置分开（§2.3）。
// 这块界面以前**完全不存在**（`deploy_matrix` 命令有，没人调）。

const matrix = ref<MatrixRow | null>(null);
const notes = ref("");
const links = ref<Link[]>([]);
const addr = ref<Record<string, { url: string; domain: string }>>({});
const saving = ref(false);
const editingInfo = ref(false);
/**
 * 编辑草稿。
 *
 * ⚠️ **不要直接编 `notes` / `links`** —— 那两个是「已保存的值」，编辑时改它们
 * 的话，点「取消」改动也留在界面上，下次点「修改」看到的是上次改了一半的内容
 * （实测反馈：表单没清空重置）。草稿在点「修改」时从已保存值复制，
 * 取消就直接丢弃。
 */
const editNotes = ref("");
const editLinks = ref<Link[]>([]);

/** 进入编辑：从**已保存的**值复制一份草稿。 */
function startEditInfo() {
  editNotes.value = notes.value;
  editLinks.value = links.value.map((l) => ({ ...l }));
  editingInfo.value = true;
}

async function loadInfo() {
  try {
    const all = await api.deployMatrix();
    const row = all.find((r) => r.name === props.repo.name) ?? null;
    matrix.value = row;
    notes.value = row?.notes ?? "";
    links.value = (row?.links ?? []).map((l) => ({ ...l }));
    addr.value = Object.fromEntries(
      (row?.cells ?? []).map(([e, a]) => [e, { url: a.url, domain: a.domain }]),
    );
  } catch (e) {
    // 读不到就当没配过——发布地址本来就是可选的，不该在详情页报错
    matrix.value = null;
  }
}

async function saveInfo() {
  saving.value = true;
  try {
    await api.deploySetRepoInfo({
      repo: props.repo.name,
      notes: editNotes.value,
      links: editLinks.value.filter((l) => l.label.trim() || l.url.trim()),
    });
    msg.success("已保存");
    editingInfo.value = false;
    await loadInfo();
  } catch (e) {
    msg.error(api.errText(e));
  } finally {
    saving.value = false;
  }
}

async function saveAddr(env: string) {
  const a = addr.value[env] ?? { url: "", domain: "" };
  try {
    await api.deploySetAddress({ repo: props.repo.name, env, url: a.url, domain: a.domain });
    msg.success(`${env} 地址已保存`);
    await loadInfo();
  } catch (e) {
    msg.error(api.errText(e));
  }
}

const envIds = computed(() => (matrix.value?.cells ?? []).map(([e]) => e));

const repoPath = computed(() => `${app.hub?.root}/repos/${props.repo.name}`);

/**
 * 「被哪些需求用着」与「在哪些环境里有检出」两类。
 *
 * 它们分别来自 `RepoRow` 的两个字段（后端有意分开的：需求引用会拦住删除，
 * 环境检出不会）。这里别再按前缀过滤 `usages` —— 那个字段**只含需求**了。
 */
const inWorkspaces = computed(() => props.repo.usages);
const inEnvs = computed(() =>
  props.repo.env_usages.map((u) => u.replace(/^envs\//, "")),
);

/**
 * 把「需求引用」那一条还原成**可跳转的分支名**。
 *
 * ⚠️ `usages` 里放的是**给人看的显示名**（后端 `workspace_usages()` 拼的
 * 「需求名（分支名）」，没起中文名时才是裸分支名）。第一版直接把它当分支名
 * 发给 `gotoWs` → 真去跳的时候 `Workspace::open` 会报「分支不存在」
 * （实测反馈里那类「点了报错」就是这么来的）。
 *
 * 所以这里从括号里取出候选，再**拿本地需求列表核对一遍** —— 只有确实是某个需求的
 * 分支才允许跳，标题里恰好带括号也不会误判。核对不上就不给跳（只当文本看）。
 */
function branchOf(usage: string): string | null {
  const m = usage.match(/（([^（）]+)）\s*$/);
  const candidates = [m?.[1], usage].filter((x): x is string => !!x).map((x) => x.trim());
  return candidates.find((c) => app.workspaces.some((w) => w.branch === c)) ?? null;
}

const gitlabUrl = computed(() => {
  const m = props.repo.remote_url.match(/^(?:https?:\/\/|git@)([^/:]+)[/:](.+?)(?:\.git)?$/);
  return m ? `https://${m[1]}/${m[2]}` : null;
});

/**
 * 该仓库在各环境里实际检出的分支。
 *
 * **先取好存进 map**，不在模板里直接调异步函数——模板里调会渲染成
 * `[object Promise]`，是个很容易踩的坑。
 */
const envBranches = ref<Record<string, string>>({});

onMounted(async () => {
  void loadInfo();
  const pairs = await Promise.all(
    inEnvs.value.map(async (e) => {
      try {
        return [e, await api.envBranchOf(e, props.repo.name)] as const;
      } catch {
        return [e, "?"] as const;
      }
    }),
  );
  envBranches.value = Object.fromEntries(pairs);
});
</script>

<template>
  <NModal :show="true" @update:show="(v: boolean) => !v && emit('close')"
    preset="card"
    :title="`仓库详情 · ${repo.name}`"
    style="width: 880px"
    @close="emit('close')"
  >
    <div class="kv"><span class="k">仓库名</span><span class="mono">{{ repo.name }}</span></div>
    <div class="kv"><span class="k">主干分支</span><span class="mono">{{ repo.main_branch }}</span></div>
    <div class="kv">
      <span class="k">本地路径</span>
      <span class="mono path">{{ repoPath }}</span>
    </div>
    <div class="kv"><span class="k">远程地址</span><span class="mono path">{{ repo.remote_url }}</span></div>

    <div class="actions">
      <NButton size="small" @click="openInFinder(repoPath)">打开目录</NButton>
      <NButton size="small" @click="copy(repoPath, '目录')">复制目录</NButton>
      <EditorButton :path="repoPath" :secondary="false" />
      <NButton v-if="gitlabUrl" size="small" type="primary" @click="openUrl(gitlabUrl)">
        打开 GitLab
      </NButton>
      <!-- 把「clone 地址 + 发布地址矩阵 + 自定义信息」整份导出去，
           对方粘一下就有了 —— 不用再听你念 jenkins 地址（实测反馈）。 -->
      <NButton size="small" :loading="sharing" @click="copyShare">导出配置</NButton>
    </div>

    <div class="section">
      <div class="sec-title">
        自定义信息
        <span class="muted small">（飞书 / tapd / 域名 / 监控 …… 想放什么放什么）</span>
      </div>
      <template v-if="!editingInfo">
        <div class="info-view">
          <template v-if="links.length">
            <span class="muted small">链接</span>
            <LinkList :links="links" />
          </template>
          <template v-if="notes">
            <span class="muted small">备注</span>
            <p class="muted small notes">{{ notes }}</p>
          </template>
          <p v-if="!links.length && !notes" class="muted small">未设置</p>
        </div>
        <NButton size="tiny" class="mt6" @click="startEditInfo">修改</NButton>
      </template>
      <template v-else>
        <div class="info-edit">
          <label class="muted small">备注</label>
          <NInput v-model:value="editNotes" type="textarea" size="small" :rows="3" :input-props="NO_AUTO" />
          <label class="muted small">链接</label>
          <LinkEditor v-model="editLinks" :placeholders="['如 项目域名 / tapd', 'https://…']" />
          <div class="edit-actions">
            <NButton size="tiny" @click="editingInfo = false">取消</NButton>
            <NButton size="tiny" type="primary" :loading="saving" @click="saveInfo">保存</NButton>
          </div>
        </div>
      </template>
    </div>

    <div v-if="envIds.length" class="section">
      <div class="sec-title">
        各环境访问地址
        <span class="muted small">（点地址打开站点，点「复制域名」贴给测试/前端）</span>
      </div>
      <table class="tbl">
        <thead>
          <tr><th>环境</th><th>地址</th><th>域名</th><th class="acts">操作</th></tr>
        </thead>
        <tbody>
          <tr v-for="e in envIds" :key="e">
            <td class="mono">{{ e }}</td>
            <td>
              <NInput
                v-model:value="addr[e].url"
                size="tiny"
                placeholder="https://…"
                :input-props="NO_AUTO"
              />
            </td>
            <td>
              <NInput
                v-model:value="addr[e].domain"
                size="tiny"
                placeholder="域名"
                :input-props="NO_AUTO"
              />
            </td>
            <td class="acts">
              <div class="btns">
                <NButton
                  v-if="addr[e].url"
                  size="tiny"
                  type="primary"
                  @click="openUrl(addr[e].url)"
                >
                  打开
                </NButton>
                <NButton v-if="addr[e].domain" size="tiny" @click="copy(addr[e].domain, '域名')">
                  复制域名
                </NButton>
                <NButton size="tiny" @click="saveAddr(e)">保存</NButton>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <div class="section">
      <div class="sec-title">
        被哪些需求使用
        <span class="muted small">（{{ inWorkspaces.length }} 个需求检出了这个仓库）</span>
      </div>
      <div v-if="inWorkspaces.length" class="chips">
        <!-- 能跳的才可点：分支名核对不上时（见 `branchOf`）只当文本显示，
             不给一个点了必然报错的按钮 -->
        <NButton
          v-for="b in inWorkspaces"
          :key="b"
          size="tiny"
          secondary
          class="chip"
          :disabled="!branchOf(b)"
          @click="branchOf(b) && emit('gotoWs', branchOf(b)!)"
        >
          {{ b }}
        </NButton>
      </div>
      <p v-else class="muted small">没有需求在用 —— 可以安全移除</p>
    </div>

    <div class="section">
      <div class="sec-title">
        被哪些环境使用
        <span class="muted small">（环境标识 ↔ 分支名是解耦的，这里看各自实际检出的分支）</span>
      </div>
      <table v-if="inEnvs.length" class="tbl">
        <thead>
          <tr><th>环境</th><th>该仓库在此环境的分支</th></tr>
        </thead>
        <tbody>
          <tr v-for="e in inEnvs" :key="e">
            <td class="mono">{{ e }}</td>
            <td class="mono">{{ envBranches[e] ?? "…" }}</td>
          </tr>
        </tbody>
      </table>
      <p v-else class="muted small">没有环境在用</p>
    </div>

    <p class="muted small note">
      删除仓库时若还被某个需求用着会被拒绝（先删掉那个需求）；环境里的检出会跟着一起清掉。
    </p>

    <template #footer>
      <div class="footer">
        <NButton @click="emit('close')">关闭</NButton>
      </div>
    </template>
  </NModal>
</template>

<style scoped>
.kv {
  display: grid;
  grid-template-columns: 6em 1fr;
  gap: 8px;
  padding: 3px 0;
  font-size: 13px;
  align-items: baseline;
}
.k {
  color: #888;
}
.path {
  word-break: break-all;
}
.actions {
  display: flex;
  gap: 8px;
  margin: 12px 0 4px;
}
.section {
  margin-top: 14px;
}
.sec-title {
  font-size: 13px;
  font-weight: 500;
  margin-bottom: 6px;
}
.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.tbl {
  width: 100%;
  border-collapse: separate;
  border-spacing: 0;
  font-size: 12px;
  border: 1px solid var(--xg-border);
  border-radius: 6px;
  overflow: hidden;
}
.tbl th,
.tbl td {
  text-align: left;
  padding: 4px 10px;
  border-bottom: 1px solid var(--xg-border);
}
.tbl tbody tr:last-child td {
  border-bottom: none;
}
.tbl th {
  color: #888;
  font-weight: 500;
  background: rgba(128, 128, 128, 0.06);
}
.muted {
  color: #888;
}
.small {
  font-size: 12px;
}
.note {
  margin-top: 16px;
}
.footer {
  display: flex;
  justify-content: flex-end;
}
.info-view {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.notes {
  margin: 0;
  white-space: pre-wrap;
}
.info-edit {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.edit-actions {
  display: flex;
  gap: 6px;
  justify-content: flex-end;
}
.mt6 {
  margin-top: 6px;
}
.acts {
  white-space: nowrap;
  width: 1%;
}
.btns {
  display: inline-flex;
  gap: 4px;
  align-items: center;
}
</style>