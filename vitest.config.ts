// 单独一份配置：vitest 4 自带的 vite 与本项目的 vite 5 不是同一份，
// 在 vite.config.ts 里 import "vitest/config" 会把 rolldown 版 vite 拉进来，
// 而它要求 Node ≥ 20.12（用了 node:util 的 styleText）。分开就互不干扰。
//
// 注意：本机 Node 是 20.11.1，vitest 4 要求 ≥ 20.19。要跑前端单测得先升级 Node。
import { defineConfig } from "vitest/config";
import vue from "@vitejs/plugin-vue";

export default defineConfig({
  plugins: [vue()],
  test: {
    environment: "happy-dom",
    include: ["src/**/*.test.ts"],
  },
});
