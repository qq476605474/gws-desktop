/**
 * 所有 NInput 的统一属性：**关掉一切自动改写**。
 *
 * 实测反馈：输入英文单词后首个字母被自动大写。这是 webview 对
 * `<input type="text">` 的 `autocapitalize="sentences"` 默认行为。
 * 分支名、仓库名、环境标识这些都是**字面值**——用户填什么就该是什么，
 * 被自动改写会直接导致建出来的目录名对不上。
 *
 * 用法：`<NInput :input-props="NO_AUTO" ... />`
 */
export const NO_AUTO = {
  autocapitalize: "off",
  autocorrect: "off",
  spellcheck: "false",
  autocomplete: "off",
} as const;
