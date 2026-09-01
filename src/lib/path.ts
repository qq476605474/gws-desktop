/** hub 路径规范化：连续/混合分隔符压成一个，风格跟随原路径（含 \ 即 Windows 风格）。
 *  用户输入与目录选择器可能给到 C:\a\/b 这类混合形态——显示难看、复制出去
 *  也别扭，且部分 Windows 程序（explorer）直接解析失败。UNC 开头 \\ 保留。 */
export function normalizePath(p: string): string {
  const trimmed = p.trim();
  if (!trimmed) return "";
  // Windows 形态判定：含反斜杠、盘符开头（C:/ 全正斜杠也是）、或 // / \\ 开头（UNC）
  const isWin =
    trimmed.includes("\\") ||
    /^[A-Za-z]:[\\/]/.test(trimmed) ||
    trimmed.startsWith("//") ||
    trimmed.startsWith("\\\\");
  if (!isWin) return trimmed.replace(/\/+$/, "");
  const unc = trimmed.startsWith("//") || trimmed.startsWith("\\\\");
  const flat = trimmed.replace(/[\\/]+/g, "\\").replace(/\\+$/, "");
  return unc ? `\\${flat}` : flat;
}

/** 路径拼接：分隔符跟随 base 风格（base 含 \ 用 \，否则 /），base 尾部与
 *  各段首尾的多余分隔符先清掉。段为空串时跳过。 */
export function joinPath(base: string, ...parts: string[]): string {
  const sep = base.includes("\\") ? "\\" : "/";
  const b = base.replace(/[\\/]+$/, "");
  const segs = [b, ...parts.map((p) => p.replace(/^[\\/]+/, "").replace(/[\\/]+$/, ""))].filter(Boolean);
  return segs.join(sep);
}
