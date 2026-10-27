//! GitLab 相关的小工具。
//!
//! 只做一件事：把配置里的 GitLab 根地址拼成「Personal Access Token 申请页」。
//! 实测反馈：让用户自己找这个页面，很多人会点到别的设置项，或者不知道怎么
//! 生成 token —— 设置页直接给个按钮最省事。

/**
 * Personal Access Token 申请页地址。
 *
 * `base` 为空（还没读到 GitLab 配置）时返回 `null`，调用方据此禁用按钮 ——
 * 不能拼出 `/-/user_settings/...` 这种没有域名的地址。
 */
export function personalAccessTokensUrl(base: string | null | undefined): string | null {
  const b = (base ?? "").trim().replace(/\/+$/, "");
  if (!b) return null;
  return `${b}/-/user_settings/personal_access_tokens`;
}
