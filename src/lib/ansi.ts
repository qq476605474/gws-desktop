const ANSI = /\u001b\[([0-9;]*)m/g;

export function stripAnsi(s: string): string {
  return s.replace(ANSI, "");
}

const ESC = (c: string) =>
  c.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

export function ansiToHtml(s: string): string {
  const parts: string[] = [];
  let last = 0;
  let open = false;
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
        // 徽标后的单个分隔空格交给 CSS 间距，不进入输出
        if (s[m.index + m[0].length] === " ") skip = 1;
      }
    } else {
      if (open) parts.push("</span>");
      parts.push(`<span class="c${code}">`);
      open = true;
    }
    last = m.index + m[0].length + skip;
  }
  parts.push(ESC(s.slice(last)));
  if (open) parts.push("</span>");
  return parts.join("");
}
