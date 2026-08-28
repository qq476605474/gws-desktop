// script setup 不允许 ES 导出，测试需要的哨兵常量放这里
<script lang="ts">
// select「Hub 根文档」的哨兵值（工作区名不会取此形），docsWs 归属快照同样用它标记 hub 数据
export const HUB_ROOT = "__hub__";
</script>

<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { confirmBox } from "../../lib/confirm";
import { useHubStore } from "../../stores/hub";
import { useCmdStore } from "../../stores/cmd";
import { runGws, openPath, copyText } from "../../lib/gws-bridge";
import { toast } from "../../lib/toast";
import { stripAnsi } from "../../lib/ansi";
import { parseDocDir, parseDocLs, parseLs, type DocEntry } from "../../lib/parse";
import PathActions from "../PathActions.vue";
import AddDocDialog from "../AddDocDialog.vue";

const hub = useHubStore();
const cmd = useCmdStore();
const wsFilter = ref(HUB_ROOT); // 默认根文档（用户反馈 #12）；选中值随数据归属写回
const docs = ref<DocEntry[]>([]);
const docsWs = ref(""); // 数据归属快照：doc ls 成功时随 docs/docDir 一起更新，
// 失败时不动——行内命令 cwd 与目录行/文件路径均以它为准，保持自洽。
// hub 根文档的归属也存 HUB_ROOT 哨兵：hub 模式的 UI 差异（目录路径/上传/新建）全部由此派生
const showNew = ref(false);
const docDir = ref("");
const err = ref("");
const submitting = ref(false);
const loading = ref(false);
const opening = ref(""); // 正在交给系统打开的文件名：读取期间禁用所有 doc-link 防重入/跨行点击
const isHubData = computed(() => docsWs.value === HUB_ROOT);
/** 文档目录（目录级访达/终端/复制的落点）：ws 模式 docs/<docdir>，hub 模式 docs/。
 *  空串 = 无数据归属（未加载/无工作区），目录行不渲染 */
const dirPath = computed(() => {
  if (isHubData.value) return `${hub.path}/docs`;
  return docDir.value ? `${hub.path}/docs/${docDir.value}` : "";
});
const dirRel = computed(() => (isHubData.value ? "docs" : `docs/${docDir.value}`));
/** 文件全路径：hub 根文档在 docs/ 第一层，工作区文档在 docs/<docdir>/ 下 */
function filePath(file: string) {
  return isHubData.value ? `${hub.path}/docs/${file}` : `${hub.path}/docs/${docDir.value}/${file}`;
}
// 并发守卫：refresh 可能重叠（切换工作区、命令结束刷新、重试），只接受最新一次的结果，
// 防止旧响应迟到覆盖新数据（模式同 WorkspaceDetail.refresh）
let seq = 0;

async function refresh() {
  const n = ++seq;
  loading.value = true;
  try {
    let wss = hub.workspaces;
    if (!wss.length) {
      // hub 数据未就绪（如直入本 Tab）：ls 兜底解析
      const ls = await runGws(["ls"], hub.path);
      if (n !== seq) return; // 旧请求迟到：丢弃
      // 兜底失败（含 spawn 失败）安静降级为"无工作区"空态，不额外报错——
      // doc ls 主流程的 err 机制已够
      if (ls.code === 0) {
        wss = parseLs(ls.output);
        // 写回 store：select 选项渲染 hub.workspaces，兜底解析的工作区也须可选
        hub.workspaces = wss;
      }
    }
    // 根文档为默认与兜底：选中值恒有效（HUB_ROOT 永远可选）；仅当存量选中值
    // 不在最新工作区列表（如他处已删除）时回落到根文档
    let target = wsFilter.value;
    if (target !== HUB_ROOT && !wss.some((w) => w.name === target)) target = HUB_ROOT;
    wsFilter.value = target;
    // hub 模式在 hub 根跑 doc ls：gws 无工作区上下文时退化为列 docs/ 第一层（hub 级文档）
    const cwd = target === HUB_ROOT ? hub.path : `${hub.path}/ws/${target}`;
    const r = await runGws(["doc", "ls"], cwd);
    if (n !== seq) return; // 旧请求迟到：丢弃本次结果
    if (r.code === null || r.code !== 0) {
      // spawn 失败（code null，output 即错误信息）或非零退出：错误输出不能喂
      // parseDocLs——会渲染成“成功”的表格
      const tail = stripAnsi(r.output).trim().split("\n").filter((l) => l).pop();
      err.value = tail ?? (r.code === null ? "gws doc ls 启动失败" : `gws doc ls 失败（退出码 ${r.code}）`);
      return; // 不更新 docs/docDir/docsWs，保留旧数据（若有）
    }
    const entries = parseDocLs(r.output);
    const dir = parseDocDir(r.output);
    // gws 上游怪癖防御：cwd 所在工作区异常（如 .workspace.json 缺 docs 键）时，
    // doc ls 以 exit 0 返回 hub 级文档列表，首行 docdir 退化为 basename "docs"。
    // ws 模式合法 docdir 恒为 <日期>-<名> 格式，命中哨兵视为异常、不更新数据；
    // hub 模式（cwd 就在 hub 根）首行恒为 "docs"，属正常态，不在此拦截
    if (target !== HUB_ROOT && dir === "docs") {
      err.value = "当前目录不是有效的需求（gws 返回了 hub 级文档列表）";
      return;
    }
    err.value = "";
    docs.value = entries;
    // docdir 取自首行（剥 ANSI），空列表也保留：目录本身存在（gws new 创建），
    // 上方文档目录操作行需要它定位
    docDir.value = dir;
    docsWs.value = target;
  } catch (e) {
    // runGws reject（如 IPC 失败）：错误进同一展示位，不崩、不死加载
    if (n !== seq) return;
    err.value = String(e);
  } finally {
    // seq 守卫：被丢弃的旧请求不得清掉在途新请求的 loading
    if (n === seq) loading.value = false;
  }
}

/** 新建文档弹窗关闭即刷新：创建成功/失败/取消统一走此路径（模式同 ReposTab.onAddClose） */
async function onNewClose() {
  showNew.value = false;
  await refresh();
}

async function push(file: string) {
  // 防双击：同 create，submitting 兜底 exec 的 IPC 往返间隙
  if (submitting.value || cmd.isRunning()) return;
  // doc push 上传 Confluence 是写远程操作：先确认再执行（模式同 EnvsTab.rmEnv）
  let ok = false;
  try {
    ok = await confirmBox(`上传 ${file} 到 Confluence？`);
  } catch {
    return; // 理论上不 reject；万一异常按取消处理
  }
  if (!ok) return;
  if (cmd.isRunning()) return; // confirm 弹窗打开期间用户可能已从另一入口启动命令
  submitting.value = true;
  try {
    const run = await cmd.execDialog(`gws doc push ${file}`, ["doc", "push", file], `${hub.path}/ws/${docsWs.value}`);
    await cmd.waitDone(run);
  } catch (e) {
    // 同 create：错误进展示位，仍刷新
    err.value = String(e);
  } finally {
    submitting.value = false;
  }
  await refresh();
}

async function commit() {
  // 防双击：同 create，submitting 兜底 exec 的 IPC 往返间隙
  if (submitting.value || cmd.isRunning()) return;
  submitting.value = true;
  try {
    // cwd 按数据归属取：doc commit 是 hub 级文档仓库的 git add -A，ws 目录与
    // hub 根都能让 gws 定位到 docs 仓库（hub 根时不能拼 /ws/__hub__——目录不存在）
    const cwd = isHubData.value ? hub.path : `${hub.path}/ws/${docsWs.value}`;
    const run = await cmd.execDialog("gws doc commit", ["doc", "commit"], cwd);
    await cmd.waitDone(run);
  } catch (e) {
    // 同 create：错误进展示位，仍刷新
    err.value = String(e);
  } finally {
    submitting.value = false;
  }
  await refresh();
}

/** 文件级操作只留“复制路径”：📂 对文件是打开文件（非预期）、💻 终端对文件无意义；
 *  访达/终端是目录级操作，收敛到上方文档目录行（用户反馈 #10） */
async function copyFile(file: string) {
  await copyText(filePath(file));
  toast("已复制路径");
}

/** 点文档名 → 系统默认应用打开（如 Typora/VS Code）：自带查看器界面简陋，弃用 */
async function openDoc(d: DocEntry) {
  if (opening.value) return; // 双击守卫：patch 滞后窗口内的第二击由此拦截
  opening.value = d.file;
  try {
    await openPath(filePath(d.file));
  } catch (e) {
    err.value = String(e);
  } finally {
    opening.value = "";
  }
}

onMounted(refresh);
</script>

<template>
  <div>
    <div class="toolbar">
      <select v-model="wsFilter" @change="refresh">
        <!-- 根文档置于首位且默认选中（用户反馈 #12）；过滤同名工作区：
             万一真有叫 __hub__ 的工作区，哨兵会静默错位到 hub 根 -->
        <option :value="HUB_ROOT">根文档</option>
        <option v-for="w in hub.workspaces.filter((w) => w.name !== HUB_ROOT)" :key="w.name" :value="w.name">{{ w.name }}</option>
      </select>
      <!-- doc new 写死当前工作区 docdir：hub 根文档不可新建，按钮随归属禁用（弹窗内再兜底） -->
      <button class="primary" :disabled="isHubData || cmd.isRunning()" @click="showNew = true">+ 新建文档</button>
      <!-- doc commit 实为整个 hub 文档仓库的 git add -A（与单行文件无关），收敛为工具栏统一入口；hub 根文档亦可提交 -->
      <button :disabled="cmd.isRunning()" @click="commit">commit 全部文档</button>
    </div>
    <p v-if="hub.error || err" class="error">
      {{ hub.error || err }}
      <button @click="refresh">重试</button>
    </p>
    <!-- 目录级操作（访达/终端/复制路径）落点为文档目录本身；表格不再重复工作区/路径列（用户反馈 #10） -->
    <div v-if="dirPath" class="group-row">📁 <code>{{ dirRel }}</code> <PathActions :path="dirPath" /></div>
    <table v-if="docs.length">
      <thead><tr><th>文档</th><th>Confluence</th><th>操作</th></tr></thead>
      <tbody>
        <tr v-for="d in docs" :key="d.file">
          <td>
            <!-- 在途打开禁全部行：系统调起窗口无上界（网络盘/大文件），不止被点的那行 -->
            <button class="doc-link" title="用系统默认应用打开" :disabled="!!opening" @click="openDoc(d)">{{ d.file }}</button>
          </td>
          <td>{{ d.synced ? `● 已同步 (wiki:${d.pageId})` : "○ 未上传" }}</td>
          <td>
            <button class="btn-sm" title="复制文件路径" @click="copyFile(d.file)">📋</button>
            <!-- doc push 同 doc new 写死当前工作区 docdir：hub 根文档无上传语义 -->
            <button v-if="!isHubData" class="btn-sm" :disabled="cmd.isRunning()" @click="push(d.file)">上传</button>
          </td>
        </tr>
      </tbody>
    </table>
    <!-- 对齐 WorkspaceDetail：仅在无数据且无错误时才显示加载中/空态（err 时上方错误行已给出原因与重试） -->
    <p v-else-if="!err && !hub.error && loading" class="muted">加载中…</p>
    <p v-else-if="!err && !hub.error" class="muted">{{ isHubData ? "（暂无文档）" : "（暂无文档——在当前需求 gws doc new 创建）" }}</p>
    <p class="muted">上传依赖 GWS_DOC_UPLOADER 指向的脚本（未配置时 gws 会提示）。doc push 的输出见命令弹窗。</p>
    <!-- doc new 写死当前工作区 docdir：hub 归属不该开弹窗，ws 由 docsWs 快照锁定数据归属 -->
    <AddDocDialog v-if="showNew && !isHubData && docsWs" :ws="docsWs" @close="onNewClose" />
  </div>
</template>

<style scoped>
.toolbar { display: flex; flex-wrap: wrap; gap: 8px; margin-bottom: 12px; align-items: center; }
/* 需求筛选 select 与相邻控件等高（用户反馈 #12）：全局 width:100%/max-width:320px
   在 flex 行里宽度漂移，显式 flex-basis 接管 */
.toolbar select { flex: 0 0 220px; height: var(--control-h); }
.group-row { display: flex; align-items: center; gap: 8px; margin-bottom: 12px; }
/* table/th/td 基础样式全局化（base.css） */
.muted { color: var(--fg-muted); font-size: 12px; }
.error { color: var(--danger-text); font-size: 13px; }
.error button { margin-left: 6px; }
/* 文件名查看入口（链接风格）：覆盖 base.css 按钮外观——无边框背景、零内边距，
   primary 色承接可点击语义，hover 下划线 */
.doc-link { border: none; background: none; padding: 0; min-height: 0; font-size: inherit; color: var(--primary); }
.doc-link:hover:not(:disabled) { text-decoration: underline; }
</style>

