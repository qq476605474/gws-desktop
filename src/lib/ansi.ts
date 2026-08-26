const ANSI = /\u001b\[([0-9;]*)m/g;

export function stripAnsi(s: string): string {
  return s.replace(ANSI, "");
}

const ESC = (c: string) =>
  c.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

// 徽标：单个非字母数字的符号字符（如 ✗ ✓ ⚠ ▸）。按码点计数以覆盖 BMP 外的符号。
const isBadge = (content: string) => {
  const chars = Array.from(content);
  return chars.length === 1 && !/\p{L}|\p{N}/u.test(chars[0]);
};

export function ansiToHtml(s: string): string {
  const parts: string[] = [];
  let last = 0;
  let open = false;
  let contentStart = 0;
  ANSI.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = ANSI.exec(s))) {
    parts.push(ESC(s.slice(last, m.index)));
    const code = m[1];
    let skip = 0;
    if (code === "0" || code === "") {
      if (open) {
        parts.push("</span>");
        open = false;
        // 仅当被闭合的是单字符徽标（✗ ✓ ⚠ ▸ 等）时，其后紧跟的 1 个空格才视为
        // 分隔符交给 CSS 间距、不进入输出；多字符内容（如 "gws drop"）后的空格
        // 是正文的一部分，必须保留，否则词与词会粘连。
        if (
          isBadge(s.slice(contentStart, m.index)) &&
          s[m.index + m[0].length] === " "
        )
          skip = 1;
      }
    } else {
      if (open) parts.push("</span>");
      parts.push(`<span class="c${code}">`);
      open = true;
      contentStart = m.index + m[0].length;
    }
    last = m.index + m[0].length + skip;
  }
  parts.push(ESC(s.slice(last)));
  if (open) parts.push("</span>");
  return parts.join("");
}
