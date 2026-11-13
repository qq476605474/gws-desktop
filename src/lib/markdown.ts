//! Markdown 预览：解析 + 消毒 + Mermaid 代码块抽取。
//!
//! 文档来自仓库（可能是同事或 AI 写的），**必须消毒后再 v-html**：
//! Markdown 允许内联 HTML，直接塞进 webview 等于开了一个 XSS 口子。

import DOMPurify from "dompurify";
import { marked } from "marked";

/** 抽取出来的 Mermaid 块：渲染时按 id 找回源码。 */
export interface MermaidBlock {
  id: string;
  code: string;
}

export interface MarkdownRender {
  html: string;
  mermaid: MermaidBlock[];
}

export function isMarkdownPath(path: string | null | undefined): boolean {
  return !!path && /\.(md|markdown)$/i.test(path);
}

/**
 * Markdown → 安全 HTML。
 *
 * Mermaid 代码块先换成占位 `div`，避免被 marked 当代码转义；
 * 真正的图由调用方在 DOM 挂载后异步渲染（失败就显示源码）。
 */
export function renderMarkdown(src: string): MarkdownRender {
  const mermaid: MermaidBlock[] = [];
  const prepared = src.replace(
    /^[ \t]*```mermaid[ \t]*\r?\n([\s\S]*?)^[ \t]*```[ \t]*$/gm,
    (_m, code: string) => {
      const id = `mb${mermaid.length}`;
      mermaid.push({ id, code: code.trim() });
      return `<div data-mermaid-id="${id}"></div>`;
    },
  );
  const raw = marked.parse(prepared, {
    gfm: true,
    breaks: false,
    async: false,
  }) as string;
  const html = DOMPurify.sanitize(raw, {
    ADD_ATTR: ["data-mermaid-id", "align"],
  });
  return { html, mermaid };
}

/**
 * 相对路径按「Markdown 文件所在目录」解析，返回相对工作目录的路径。
 * 绝对路径、空引用、越出根目录的一律返回空串（不读）。
 */
export function resolveAssetPath(filePath: string, ref: string): string {
  const clean = ref.split("#")[0].split("?")[0].trim();
  if (!clean || clean.startsWith("/") || /^[a-z]+:/i.test(clean)) return "";
  const out = filePath.split("/").slice(0, -1);
  for (const seg of clean.split("/")) {
    if (!seg || seg === ".") continue;
    if (seg === "..") {
      if (!out.length) return "";
      out.pop();
    } else {
      out.push(seg);
    }
  }
  return out.join("/");
}

/** 图片 `src` 里的百分号编码解码失败就原样用。 */
export function decodeRef(ref: string): string {
  try {
    return decodeURIComponent(ref);
  } catch {
    return ref;
  }
}

export function isExternalLink(href: string): boolean {
  return /^(https?:|mailto:)/i.test(href);
}