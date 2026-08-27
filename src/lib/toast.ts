import { reactive } from "vue";

export interface Toast {
  id: number;
  message: string;
}

/** 全局 toast 队列：模块级 reactive（非 Pinia store）——无跨 store 依赖、
 *  任意组件/工具函数可直接 import 调用，ToastList 挂 App.vue 消费 */
export const toasts = reactive<Toast[]>([]);
let nextId = 1;

/** 显示一条轻提示（如「已复制路径」），2.5s 后自动消失。
 *  失败信息仍走各组件既有 err 展示位——toast 只做成功反馈，不抢错误通道。 */
export function toast(message: string) {
  const id = nextId++;
  toasts.push({ id, message });
  setTimeout(() => {
    const i = toasts.findIndex((t) => t.id === id);
    if (i >= 0) toasts.splice(i, 1);
  }, 2500);
}
