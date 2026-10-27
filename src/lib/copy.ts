import { useMessage } from "naive-ui";

import { copyText } from "../bridge";

/**
 * 复制到剪贴板并**给出反馈**。
 *
 * 实测反馈：点了复制没有任何提示，不知道到底成没成。
 * 复制是「没有可见结果」的动作，不给反馈就等于没做——必须弹一条。
 *
 * `what` 是给用户看的目标说明，如「目录」「仓库地址」。
 * 复制目录类按钮统一叫「复制目录」、统一传 `'目录'`（提示「已复制目录」），
 * 不要再按场景写成「本地路径 / 文档路径 / 期次目录」——同一个动作，名字得对得上。
 */
export function useCopy() {
  const msg = useMessage();
  return async (text: string, what = "内容") => {
    if (!text) {
      msg.warning(`没有可复制的${what}`);
      return;
    }
    try {
      await copyText(text);
      msg.success(`已复制${what}`);
    } catch (e) {
      msg.error(`复制失败：${String((e as { message?: string })?.message ?? e)}`);
    }
  };
}
