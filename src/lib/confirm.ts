import { reactive } from "vue";

interface PendingConfirm {
  message: string;
  resolve: (ok: boolean) => void;
}

const state = reactive<{ pending: PendingConfirm | null }>({ pending: null });

/** GUI 级确认框：替代插件原生 confirm——原生对话框在 WKWebView 下取消按钮无响应，
 *  且按钮文案跟随系统英文（Cancel/OK）；自绘弹窗按钮恒中文、行为可控 */
export function confirmBox(message: string): Promise<boolean> {
  // 并发防御：组件侧已有 confirming 守卫，这里兜底按取消，避免覆盖后首问永挂
  if (state.pending) return Promise.resolve(false);
  return new Promise((resolve) => {
    state.pending = { message, resolve };
  });
}

export function answerConfirm(ok: boolean) {
  const p = state.pending;
  state.pending = null;
  p?.resolve(ok);
}

export const confirmState = state;
