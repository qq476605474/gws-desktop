//! 一键更新：**左下角那条提示**与**设置页的版本检查**共用这一份。
//!
//! 两处都要做同一串事：调后端（下载 → 校验 → 装）→ 按结果决定是重启还是
//! 把包交给用户（Linux 的 deb 要 sudo）。抄两遍的话，一处改了另一处就漂。
//!
//! 抽成 composable 而不是"把逻辑塞进某个组件"，是因为这两个入口在
//! **不同的组件树位置**（侧边栏 vs 设置页），谁也不该 import 谁。

import { ref } from "vue";

import * as api from "../bridge";
import type { UpdateInstallOutcome } from "../types";

export function useUpdater() {
  const installing = ref(false);
  const err = ref("");
  const done = ref<UpdateInstallOutcome | null>(null);

  /**
   * 下载并安装。
   *
   * 返回结果（失败返回 `null`，原因在 `err` 里）—— 调用方自己决定怎么提示：
   * 左下角弹消息、设置页显示在页面上。
   */
  async function install(): Promise<UpdateInstallOutcome | null> {
    if (installing.value) return null;
    installing.value = true;
    err.value = "";
    try {
      const out = await api.updateInstall();
      done.value = out;
      if (out.next === "restart") {
        // 装好了：**等一拍再退出**，好让"正在重启"那句话先显示出来。
        //
        // 退出这一步**不能悄悄失败**：Windows 上要等本进程真退干净，安装程序才肯装
        // （NSIS 会用退出码 2 拒绝"应用还在跑"）。所以这里把失败也接住并说出来。
        // （助手那边还有重试兜底，最多等 60 秒。）
        setTimeout(() => {
          void api.restartApp().catch((e: unknown) => {
            err.value = `自动重启失败，请手动关闭本窗口再重新打开：${e}`;
          });
        }, 800);
      }
      return out;
    } catch (e) {
      err.value = api.errText(e);
      return null;
    } finally {
      installing.value = false;
    }
  }

  return { installing, err, done, install };
}