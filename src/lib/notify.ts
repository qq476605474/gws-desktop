//! 全局提示出口 —— 给**不在组件里**的代码（bridge、工具模块）一个弹提示的地方。
//!
//! Naive UI 的 `useMessage()` 只能在 `NMessageProvider` **内部**的组件里调用，
//! 而 `bridge.ts` 是普通模块，调不到。于是「打开目录失败」这类错误**没有地方弹**，
//! 只能被丢掉 —— 表现为点了按钮界面上一点痕迹都没有（实测反馈：环境页点
//! 「文件管理」没反应）。
//!
//! 这里放一个「装一次」的出口：`components/NotifierBridge.vue` 在 Provider 内部
//! 挂载时把 message 实例装进来。没装时退化成 `console.error` ——
//! **绝不因为「没地方弹」就把错误吞掉**。

export interface Notifier {
  error: (text: string) => void;
  warning: (text: string) => void;
  success: (text: string) => void;
}

let impl: Notifier | null = null;

export function installNotifier(n: Notifier): void {
  impl = n;
}

/** 弹一条错误。没有出口时写控制台，不静默。 */
export function notifyError(text: string): void {
  if (impl) impl.error(text);
  else console.error(text);
}

/** 弹一条提醒（不阻断操作，例如「还没配置常用编辑器」）。 */
export function notifyWarning(text: string): void {
  if (impl) impl.warning(text);
  else console.warn(text);
}