//! Conventional Commits 填写工具：只负责拼消息，不校验、不强制。
//!
//! 规范见 https://www.conventionalcommits.org/：
//!   <type>[optional scope][!]: <description>
//!
//!   [optional body]
//!
//!   [optional footer(s)]

export interface CommitType {
  value: string;
  /** 下拉里显示的名字：`feat — 新功能`。 */
  label: string;
  /** 选它时的一句解释。 */
  hint: string;
}

export const COMMIT_TYPES: CommitType[] = [
  { value: "feat", label: "feat — 新功能", hint: "给用户用的新能力" },
  { value: "fix", label: "fix — 修复", hint: "修掉一个错误行为" },
  { value: "docs", label: "docs — 文档", hint: "只改文档、注释" },
  { value: "style", label: "style — 格式", hint: "空格、排版，不影响逻辑" },
  { value: "refactor", label: "refactor — 重构", hint: "不改行为的结构调整" },
  { value: "perf", label: "perf — 性能", hint: "更快的实现" },
  { value: "test", label: "test — 测试", hint: "补测试或改测试" },
  { value: "build", label: "build — 构建", hint: "依赖、打包、构建脚本" },
  { value: "ci", label: "ci — 流水线", hint: "CI / 发布配置" },
  { value: "chore", label: "chore — 杂项", hint: "其它不影响功能的改动" },
  { value: "revert", label: "revert — 回滚", hint: "撤销之前的提交" },
];

export interface CommitParts {
  type: string;
  scope?: string | null;
  summary: string;
  body?: string;
  breaking?: boolean;
  /** 不兼容变更的具体影响（可选；勾了 breaking 才用）。 */
  breakingNote?: string;
}

/**
 * 详细说明按「一行一条」转成列表：没写前缀的补 `- `，
 * 已经是 `-` / `*` / `1.` 的保持原样，空行丢掉。
 *
 * 批量提交多个仓库时一般就是记几条改动，自动分条比让人手打符号稳。
 */
function normalizeBody(raw?: string | null): string {
  const lines = (raw ?? "").split(/\r?\n/);
  const out: string[] = [];
  for (const line of lines) {
    const t = line.trim();
    if (!t) continue;
    out.push(/^([-*]|\d+[.)])\s/.test(t) ? t : `- ${t}`);
  }
  return out.join("\n");
}

/**
 * 拼出最终提交说明。
 *
 * **描述留空返回空串** —— 调用方据此回退到后端的默认说明，
 * 而不是生成一个只有 `feat:` 的半截消息。
 */
export function buildCommitMessage(p: CommitParts): string {
  const summary = p.summary.trim().replace(/\s+/g, " ");
  if (!summary) return "";

  const scope = p.scope?.trim().replace(/\s+/g, " ");
  const head =
    `${p.type}${scope ? `(${scope})` : ""}${p.breaking ? "!" : ""}: ${summary}`;

  const parts = [head];
  const body = normalizeBody(p.body);
  if (body) parts.push(body);

  const note = p.breakingNote?.trim();
  if (p.breaking && note) parts.push(`BREAKING CHANGE: ${note}`);

  return parts.join("\n\n");
}