#!/usr/bin/env node
// 把与桌面端同版本的 gws 复制进 src-tauri/resources/，供 Tauri 打进安装包。
//
// 由 `npm run tauri build` 的 beforeBuildCommand 调用；`tauri dev` 不走这里，
// 所以日常开发不会被 CLI 的 release 编译拖慢。桌面端启动后会用它把本机
// `~/.local/bin/gws` 覆盖成同版本（见 src-tauri/src/commands/tools.rs）。

import { execFileSync } from "node:child_process";
import { chmodSync, copyFileSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { join } from "node:path";

const triple = process.env.TAURI_ENV_TARGET_TRIPLE || "";
const cargoArgs = ["build", "--release", "-p", "gws-cli"];
if (triple) cargoArgs.push("--target", triple);
execFileSync("cargo", cargoArgs, { stdio: "inherit" });

const isWindows = triple ? triple.includes("windows") : process.platform === "win32";
const exe = isWindows ? "gws.exe" : "gws";
const built = triple ? join("target", triple, "release", exe) : join("target", "release", exe);

const outDir = join("src-tauri", "resources");
mkdirSync(outDir, { recursive: true });
// 清掉上一次的产物：跨平台构建时别把 gws 和 gws.exe 一起打进去
for (const f of readdirSync(outDir)) {
  if (f !== ".gitkeep") rmSync(join(outDir, f), { force: true, recursive: true });
}

const dest = join(outDir, exe);
copyFileSync(built, dest);
if (!isWindows) chmodSync(dest, 0o755);
console.log(`[bundle-cli] ${built} → ${dest}`);
