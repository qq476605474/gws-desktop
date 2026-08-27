import { ref } from "vue";

/** 全局数据刷新在途计数：>0 时 App.vue 渲染全屏 busy 遮罩（z-index 90）挡住其他操作。
 * 数据刷新类（st/ls/doc ls 等取数）不弹命令弹窗，但执行期间同样禁止操作界面。
 * 放 lib 层的响应式单例而非 pinia store：维护方是 lib/gws-bridge 的 runGws，
 * 与 UI 状态无关，也避免 bridge 反向依赖 store。 */
export const busyCount = ref(0);
