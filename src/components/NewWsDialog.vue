<script setup lang="ts">
import { NO_AUTO } from "../lib/inputs";
import { computed, ref } from "vue";
import { NAlert, NButton, NCheckbox, NInput, NModal, NSelect, NTag, useMessage } from "naive-ui";

import * as api from "../bridge";
import {
  canCreate as canCreateRule,
  composeBranch,
  needsSuffix,
  resolveBranch,
  resolveTitle,
  stripLeadingDates,
  TITLE_PLACEHOLDER,
  type BranchMode,
  type NewWsForm,
} from "../lib/newWsForm";
import { useAppStore } from "../stores/app";
import { moduleRepos } from "../types";
import BaseEditor from "./BaseEditor.vue";
import type { Link } from "../types";
import LinkEditor from "./LinkEditor.vue";
import type { PrecheckFailure } from "../types";

const emit = defineEmits<{ (e: "close"): void }>();
const app = useAppStore();
const msg = useMessage();

/**
 * 分支后缀。
 *
 * **它不是需求的中文名**（实测反馈：第一栏叫「名称」，填的人以为写的是中文名，
 * 其实是分支的后半段）。中文名在下面那栏 [`title`]。
 * 选「完整分支名」时这一栏根本不显示，也就不参与校验。
 */
const suffix = ref("");
/** 中文名（给人看的）。留空时按模式兜底，见 `lib/newWsForm.ts`。 */
const title = ref("");
// 用 hub 的真实配置，而不是写死 `feature` + 8 位 —— 预览必须和提交结果一致
const prefix = ref(app.hub?.branch_prefix || "feature");
const branchMode = ref<BranchMode>("compose");
const fullBranch = ref("");
/** 基线链（有序列表，顺序即优先级）。 */
const baseList = ref<string[]>([]);
const baseEditor = ref<InstanceType<typeof BaseEditor> | null>(null);
const docFamily = ref("");
const notes = ref("");
const links = ref<Link[]>([]);
/**
 * 选中的模块。
 *
 * **默认只勾文档库**：它一般都要（AI 写的文档得有地方放），但代码模块要人
 * 明确选 —— 默认全勾的话一不小心就给 22 个仓库都建了 worktree（实测反馈）。
 */
const picked = ref<string[]>(moduleRepos(app.repos).filter((r) => r.is_doc).map((r) => r.name));
const err = ref("");
const precheck = ref<PrecheckFailure | null>(null);
const submitting = ref(false);

const PREFIXES = ["feature", "hotfix", "bugfix", "release", "refactor", "chore"];

/** hub 配的日期位数（6/8）。 */
const dateDigits = computed(() => (app.hub?.branch_date_digits === 6 ? 6 : 8));

function today(): string {
  const d = new Date();
  const yyyy = String(d.getFullYear());
  const year = dateDigits.value === 6 ? yyyy.slice(-2) : yyyy;
  return `${year}${String(d.getMonth() + 1).padStart(2, "0")}${String(d.getDate()).padStart(2, "0")}`;
}

/** 预览分支名 —— 提交前就能看到，避免建完才发现拼错。 */
const composedBranch = computed(() => composeBranch(prefix.value, today(), suffix.value));

/** 表单规则的输入（纯函数在 `lib/newWsForm.ts`，那几条规则单测钉着）。 */
const formState = computed<NewWsForm>(() => ({
  mode: branchMode.value,
  suffix: suffix.value,
  prefix: prefix.value,
  date: today(),
  fullBranch: fullBranch.value,
  title: title.value,
  moduleCount: picked.value.length,
}));

/** 「分支后缀」这一栏要不要显示 —— 选了完整分支名就不显示，也不再必填。 */
const showSuffix = computed(() => needsSuffix(branchMode.value));

/** 后缀里已经写了日期：会被自动去掉，预览下方要说一声。 */
const suffixHadDate = computed(
  () => showSuffix.value && stripLeadingDates(suffix.value).stripped,
);

const canCreate = computed(() => canCreateRule(formState.value));

function selectAllRepos() {
  picked.value = moduleRepos(app.repos).map((r) => r.name);
}

async function create() {
  if (submitting.value || !canCreate.value) return;
  const bad = baseEditor.value?.firstError ?? "";
  if (bad) {
    err.value = `基线来源有问题：${bad}`;
    return;
  }
  submitting.value = true;
  err.value = "";
  precheck.value = null;
  try {
    // **两种模式都直接传完整分支名**：预览看到的、提交的就是同一个字符串，
    // 不会再出现「界面预览一种、后端又拼一次」的偏差（那也是重复日期的来源之一）。
    const branch = resolveBranch(formState.value);
    const out = await api.wsNew({
      name: null,
      branch,
      title: resolveTitle(title.value, branch) || null,
      modules: picked.value,
      base: baseList.value.map((s) => s.trim()).filter(Boolean),
      prefix: prefix.value,
      doc_family: docFamily.value.trim() || null,
      notes: notes.value,
      links: links.value.filter((l) => l.label.trim() || l.url.trim()),
    });
    msg.success(`需求已创建：${out.branch}`);
    if (out.doc?.attached) msg.success(`文档目录已挂上：${out.doc.relative_dir}`);
    else if (out.doc_error) msg.warning(`文档目录未挂上：${out.doc_error}`);
    emit("close");
  } catch (e) {
    // 预检失败是**分类明细**，要按类别展示并给不同的下一步，不是一句错误文本
    const pc = api.precheckOf(e);
    if (pc?.precheck) precheck.value = pc.precheck;
    else err.value = api.errText(e);
  } finally {
    submitting.value = false;
  }
}
</script>

<template>
  <NModal :show="true" @update:show="(v: boolean) => !v && emit('close')"
    preset="card"
    title="新建需求"
    style="width: 620px"
    :mask-closable="!submitting"
    @close="emit('close')"
  >
    <div class="form">
      <!-- 中文名放最前面：它是人在列表里看到的那个名字。
           以前第一栏叫「名称」，填的人以为写的是中文名，其实是分支后缀。 -->
      <label>需求名称</label>
      <NInput
        v-model:value="title"
        :placeholder="`给人看的名字（${TITLE_PLACEHOLDER}）`"
        :input-props="NO_AUTO"
      />

      <label>分支方式</label>
      <div class="radios">
        <NCheckbox
          :checked="branchMode === 'compose'"
          @update:checked="() => (branchMode = 'compose')"
        >
          前缀组合
        </NCheckbox>
        <NCheckbox
          :checked="branchMode === 'full'"
          @update:checked="() => (branchMode = 'full')"
        >
          完整分支名
        </NCheckbox>
      </div>

      <!-- 选了完整分支名，这一栏整栏不出现（分支名已经逐字给全了，
           再要一个后缀既没处用又会把人卡住） -->
      <template v-if="showSuffix">
        <!-- 先定分支前缀，再写后缀：拼出来的分支名就是从前往后读的顺序（实测反馈） -->
        <label>分支前缀</label>
        <NSelect v-model:value="prefix" :options="PREFIXES.map((p) => ({ label: p, value: p }))" />

        <label>分支后缀</label>
        <NInput
          v-model:value="suffix"
          placeholder="只写后半段，如 checkout-revamp"
          :input-props="NO_AUTO"
        />

        <label></label>
        <div class="preview">
          分支：<code>{{ composedBranch }}</code>
          <div v-if="suffixHadDate" class="date-warn">
            后缀里已经写了日期，拼接时已自动去掉（避免出现两个日期）。
          </div>
        </div>
      </template>
      <template v-else>
        <label>完整分支名</label>
        <NInput
          v-model:value="fullBranch"
          placeholder="如 feature-20261010-checkout-revamp"
          :input-props="NO_AUTO"
        />
      </template>

      <label>基线来源</label>
      <BaseEditor ref="baseEditor" v-model="baseList" />
      <label></label>
      <div class="muted small">
        分阶段需求（第二期基于第一期）就写 <code>ws:上一期的分支名</code>。
        上游没有该分支的模块会自动降级到主干。
      </div>

      <label>需求分类</label>
      <NInput v-model:value="docFamily" placeholder="留空按年月归组（如 2026-10）"  :input-props="NO_AUTO"/>
      <label></label>
      <div class="muted small">
        留空时按年月归组；V1 / V2 / bugfix 要放在一起，就填与上一期相同的分类。
      </div>

      <label>备注</label>
      <NInput v-model:value="notes" type="textarea" size="small" :rows="2" :input-props="NO_AUTO" />
      <label>自定义信息</label>
      <LinkEditor v-model="links" :placeholders="['如 tapd / 飞书文档', 'https://…']" />
      <label></label>
      <div class="muted small">
        建的时候就能填，不用建完再进「需求设置」补。都是可选的。
      </div>

      <label>模块</label>
      <div class="mods">
        <div class="mods-bar">
          <NButton size="tiny" @click="selectAllRepos">全选</NButton>
          <NButton size="tiny" @click="picked = []">全不选</NButton>
          <span class="muted small">已选 {{ picked.length }} / {{ moduleRepos(app.repos).length }}</span>
        </div>
        <div class="mods-list">
          <NCheckbox
            v-for="r in moduleRepos(app.repos)"
            :key="r.name"
            :checked="picked.includes(r.name)"
            @update:checked="
              (c: boolean) => (picked = c ? [...picked, r.name] : picked.filter((x) => x !== r.name))
            "
          >
            <span class="mono">{{ r.name }}</span>
            <!-- 文档库置顶 + 标出来：一眼看到「这次选没选它」 -->
            <NTag v-if="r.is_doc" size="tiny" type="info" :bordered="false" class="tag">文档</NTag>
          </NCheckbox>
          <p v-if="!app.repos.length" class="muted small">还没有仓库，先去「仓库」页添加</p>
        </div>
      </div>
    </div>

    <NAlert v-if="err" type="error" :bordered="false" class="mt">{{ err }}</NAlert>

    <!-- 预检失败：分类展示 + 每类给不同的下一步 -->
    <div v-if="precheck" class="precheck">
      <div class="precheck-title">预检未通过，工作区未创建（没有留下任何残留）</div>

      <div v-if="precheck.missing_repos.length" class="pc-group">
        <div class="pc-label">以下仓库还没添加进 hub</div>
        <div v-for="m in precheck.missing_repos" :key="m" class="mono small">✗ {{ m }}</div>
        <div class="muted small">到「仓库」页添加它们。</div>
      </div>

      <div v-if="precheck.remote_branch_exists.length" class="pc-group">
        <div class="pc-label">分支已存在于远程，可能已有需求在用同名分支</div>
        <div v-for="m in precheck.remote_branch_exists" :key="m" class="mono small">✗ {{ m }}</div>
        <div class="muted small">
          若这是已经建好的需求，改用「导入需求」；若要另建，换个分支名。
        </div>
      </div>

      <div v-if="precheck.local_branch_exists.length" class="pc-group">
        <div class="pc-label">分支已存在于本地（多半是此前删目录没清干净）</div>
        <div v-for="m in precheck.local_branch_exists" :key="m" class="mono small">✗ {{ m }}</div>
        <div class="muted small">到「需求」页点「清理残留」。</div>
      </div>

      <div v-if="precheck.base_missing.length" class="pc-group">
        <div class="pc-label">基线不存在</div>
        <div v-for="[m, b] in precheck.base_missing" :key="m" class="mono small">✗ {{ m }}: {{ b }}</div>
      </div>
    </div>

    <template #footer>
      <div class="footer">
        <NButton :disabled="submitting" @click="emit('close')">取消</NButton>
        <NButton type="primary" :loading="submitting" :disabled="!canCreate" @click="create">
          创建
        </NButton>
      </div>
    </template>
  </NModal>
</template>

<style scoped>
.form {
  display: grid;
  grid-template-columns: 6em 1fr;
  gap: 10px 12px;
  align-items: center;
  font-size: 13px;
}
.radios {
  display: flex;
  gap: 16px;
}
.preview {
  font-size: 12px;
  color: #888;
}
.date-warn {
  margin-top: 2px;
  color: #b36b00;
  font-size: 12px;
}
.mods {
  grid-column: 2;
}
.mods-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 4px;
}
.tag {
  margin-left: 6px;
}
.mods-list {
  max-height: 160px;
  overflow: auto;
  border: 1px solid var(--xg-border);
  border-radius: 6px;
  padding: 6px 10px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.muted {
  color: #888;
}
.small {
  font-size: 12px;
}
.mt {
  margin-top: 12px;
}
.precheck {
  margin-top: 12px;
  padding: 12px;
  border-radius: 6px;
  background: rgba(208, 48, 80, 0.06);
}
.precheck-title {
  font-weight: 500;
  margin-bottom: 8px;
  color: #d03050;
}
.pc-group {
  margin-bottom: 10px;
}
.pc-label {
  font-size: 12px;
  font-weight: 500;
}
.footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>