<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useHubStore } from "../../stores/hub";
import { useCmdStore } from "../../stores/cmd";
import { runGws, readTextFile } from "../../lib/gws-bridge";
import { stripAnsi } from "../../lib/ansi";
import { parseDocDir, parseDocLs, parseLs, type DocEntry } from "../../lib/parse";
import PathActions from "../PathActions.vue";
import DocViewerDialog from "../DocViewerDialog.vue";

// select「Hub 根文档」的哨兵值（工作区名不会取此形），docsWs 归属快照同样用它标记 hub 数据
const HUB_ROOT = "__hub__";

const hub = useHubStore();
const cmd = useCmdStore();
const wsFilter = ref("");
const docs = ref<DocEntry[]>([]);
const docsWs = ref(""); // 数据归属快照：doc ls 成功时随 docs/docDir 一起更新，
// 失败时不动——旧表格的工作区列与行内命令 cwd 均以它为准，保持自洽。
// hub 根文档的归属也存 HUB_ROOT 哨兵：hub 模式的 UI 差异（路径/上传/新建）全部由此派生
const newFile = ref("");
const docDir = ref("");
const err = ref("");
const submitting = ref(false);
const loading = ref(false);
const reading = ref(""); // 正在读取内容的文件名：本地读取极快，仅以此禁用该行按钮防重入
const viewer = ref<{ fileName: string; path: string; content: string } | null>(null);
const isHubData = computed(() => docsWs.value === HUB_ROOT);
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
      wss = ls.code === null || ls.code !== 0 ? [] : parseLs(ls.output);
    }
    // 默认选中（首个工作区）写回 wsFilter：select 已无「当前」跟随首项，
    // 选中值必须落成真实 option——否则首项与列表项重复显示同一工作区
    const target = wsFilter.value || wss[0]?.name;
    if (!target) {
      docs.value = [];
      docDir.value = "";
      docsWs.value = "";
      return;
    }
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
      err.value = "当前目录不是有效的工作区（gws 返回了 hub 级文档列表）";
      return;
    }
    err.value = "";
    docs.value = entries;
    // 空列表时 docDir 置空（无行渲染路径）；非空时取首行（docdir 名）
    docDir.value = entries.length ? dir : "";
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

async function create() {
  // 防双击：exec 的 IPC 往返间隙 isRunning 尚未生效（submitting 兜底）；
  // isRunning 与 push/commit 的入口守卫统一
  if (submitting.value || cmd.isRunning()) return;
  const name = newFile.value.trim(); // 纯空格输入视同空，避免把空白当文件名传给 gws
  // doc new 在 gws 侧写死当前工作区 docdir，对 hub 根文档无意义——hub 归属时入口
  // 拦截（输入框已禁用，此处防程序化点击绕过）
  if (!name || !docsWs.value || isHubData.value) return;
  if (/\s/.test(name)) {
    // gws doc new 不禁止空格名，但含空格的文件名会“已创建却列不出来”
    // （历史遗留文件也照样列出）——入口直接拒绝，输入习惯保持单个词
    err.value = "文档名不能包含空格";
    return;
  }
  submitting.value = true;
  try {
    const run = await cmd.execDialog(`gws doc new ${name}`, ["doc", "new", name], `${hub.path}/ws/${docsWs.value}`);
    // exec 返回时命令仍在跑（事件流异步），须等终态再刷新，否则拿到的是旧列表
    await cmd.waitDone(run);
    if (run.state === "done") newFile.value = ""; // 失败保留输入便于重试
  } catch (e) {
    // exec reject（如 IPC 失败）：错误进展示位（模式同 WorkspaceDetail.doCmd 的 stErr）
    err.value = String(e);
  } finally {
    submitting.value = false;
  }
  await refresh();
}

async function push(file: string) {
  if (cmd.isRunning()) return; // 按钮禁用渲染有间隙，入口再拦一道
  try {
    const run = await cmd.execDialog(`gws doc push ${file}`, ["doc", "push", file], `${hub.path}/ws/${docsWs.value}`);
    await cmd.waitDone(run);
  } catch (e) {
    // 同 create：错误进展示位，仍刷新
    err.value = String(e);
  }
  await refresh();
}

async function commit() {
  if (cmd.isRunning()) return;
  try {
    // cwd 按数据归属取：doc commit 是 hub 级文档仓库的 git add -A，ws 目录与
    // hub 根都能让 gws 定位到 docs 仓库（hub 根时不能拼 /ws/__hub__——目录不存在）
    const cwd = isHubData.value ? hub.path : `${hub.path}/ws/${docsWs.value}`;
    const run = await cmd.execDialog("gws doc commit", ["doc", "commit"], cwd);
    await cmd.waitDone(run);
  } catch (e) {
    // 同 create：错误进展示位，仍刷新
    err.value = String(e);
  }
  await refresh();
}

async function openViewer(d: DocEntry) {
  if (reading.value) return; // 双击守卫：patch 滞后窗口内的第二击由此拦截
  reading.value = d.file;
  try {
    // 路径规则与表格路径列/PathActions 一致：hub 根文档在 docs/ 第一层，
    // 工作区文档在 docs/<docdir>/ 下
    const path = isHubData.value
      ? `${hub.path}/docs/${d.file}`
      : `${hub.path}/docs/${docDir.value}/${d.file}`;
    const content = await readTextFile(path);
    viewer.value = { fileName: d.file, path, content };
  } catch (e) {
    // 读取失败（文件被移走/权限等）：错误进既有展示位，弹窗不出现
    err.value = String(e);
  } finally {
    reading.value = "";
  }
}

onMounted(refresh);
</script>

<template>
  <div>
    <div class="toolbar">
      <select v-model="wsFilter" @change="refresh">
        <option v-for="w in hub.workspaces" :key="w.name" :value="w.name">{{ w.name }}</option>
        <option :value="HUB_ROOT">Hub 根文档</option>
      </select>
      <!-- doc new 写死当前工作区 docdir：hub 根文档不可新建，输入与按钮随归属禁用 -->
      <input v-model="newFile" placeholder="新文档名.md" :disabled="isHubData || cmd.isRunning()" />
      <button class="primary" :disabled="!newFile.trim() || isHubData || cmd.isRunning() || submitting" @click="create">+ 新建文档</button>
      <!-- doc commit 实为整个 hub 文档仓库的 git add -A（与单行文件无关），收敛为工具栏统一入口；hub 根文档亦可提交 -->
      <button :disabled="cmd.isRunning()" @click="commit">commit 全部文档</button>
    </div>
    <p v-if="hub.error || err" class="error">
      {{ hub.error || err }}
      <button @click="refresh">重试</button>
    </p>
    <table v-if="docs.length">
      <thead><tr><th>文档</th><th>工作区</th><th>Confluence</th><th>路径</th><th v-if="!isHubData">操作</th></tr></thead>
      <tbody>
        <tr v-for="d in docs" :key="d.file">
          <td>
            <button class="doc-link" :disabled="reading === d.file" @click="openViewer(d)">{{ d.file }}</button>
          </td>
          <td>{{ isHubData ? "hub 根" : docsWs }}</td>
          <td>{{ d.synced ? `● 已同步 (wiki:${d.pageId})` : "○ 未上传" }}</td>
          <td>
            <code>{{ isHubData ? `docs/${d.file}` : `docs/${docDir}/${d.file}` }}</code>
            <PathActions :path="isHubData ? `${hub.path}/docs/${d.file}` : `${hub.path}/docs/${docDir}/${d.file}`" />
          </td>
          <!-- doc push 同 doc new 写死工作区 docdir：hub 根文档无上传语义，整列不渲染 -->
          <td v-if="!isHubData">
            <button class="btn-sm" :disabled="cmd.isRunning()" @click="push(d.file)">上传</button>
          </td>
        </tr>
      </tbody>
    </table>
    <!-- 对齐 WorkspaceDetail：仅在无数据且无错误时才显示加载中/空态（err 时上方错误行已给出原因与重试） -->
    <p v-else-if="!err && !hub.error && loading" class="muted">加载中…</p>
    <p v-else-if="!err && !hub.error" class="muted">{{ isHubData ? "（暂无文档）" : "（暂无文档——在当前工作区 gws doc new 创建）" }}</p>
    <p class="muted">上传依赖 GWS_DOC_UPLOADER 指向的脚本（未配置时 gws 会提示）。doc push 的输出见命令弹窗。</p>
    <DocViewerDialog v-if="viewer" :file-name="viewer.fileName" :path="viewer.path" :content="viewer.content" @close="viewer = null" />
  </div>
</template>

<style scoped>
.toolbar { display: flex; gap: 8px; margin-bottom: 12px; align-items: center; }
table { width: 100%; border-collapse: collapse; font-size: 13px; }
th, td { text-align: left; padding: 6px 10px; border-bottom: 1px solid var(--border); }
.muted { color: var(--fg-muted); font-size: 12px; }
.error { color: var(--danger-text); font-size: 13px; }
.error button { margin-left: 6px; }
/* 文件名查看入口（链接风格）：覆盖 base.css 按钮外观——无边框背景、零内边距，
   primary 色承接可点击语义，hover 下划线 */
.doc-link { border: none; background: none; padding: 0; min-height: 0; font-size: inherit; color: var(--primary); }
.doc-link:hover:not(:disabled) { text-decoration: underline; }
</style>

