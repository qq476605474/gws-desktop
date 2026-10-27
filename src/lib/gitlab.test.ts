import { describe, expect, it } from "vitest";

import { personalAccessTokensUrl } from "./gitlab";

describe("personalAccessTokensUrl", () => {
  it("拼到 GitLab 根地址下", () => {
    expect(personalAccessTokensUrl("https://gitlab.example.com")).toBe(
      "https://gitlab.example.com/-/user_settings/personal_access_tokens",
    );
  });

  it("末尾有斜杠也不会拼出双斜杠", () => {
    expect(personalAccessTokensUrl("https://gitlab.example.com/")).toBe(
      "https://gitlab.example.com/-/user_settings/personal_access_tokens",
    );
  });

  it("没配置地址 → null（按钮禁用，不跳错地方）", () => {
    expect(personalAccessTokensUrl("")).toBeNull();
    expect(personalAccessTokensUrl("   ")).toBeNull();
    expect(personalAccessTokensUrl(null)).toBeNull();
  });
});
