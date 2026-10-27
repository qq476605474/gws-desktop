#!/bin/sh
# Gws CLI（gws）一键安装
#
#   curl -fsSL https://raw.githubusercontent.com/qq476605474/gws-desktop/main/install.sh | sh
#
# 校验链沿用旧版 bash 的 gws 安装脚本（main 分支的 install.sh）——
# 它做得相当严谨，新版是二进制分发，但「下载的东西必须验明正身」这条不能丢。
#
# 与旧版的区别：
#   - 下的是**二进制**而不是 bash 脚本，所以校验项从「shebang + bash -n」
#     换成「可执行文件魔数 + sha256」
#   - 按平台+架构选资产，选不到就明确报错，不猜
#   - 自动把 ~/.local/bin 加进 PATH 的提示保留

set -eu

# 发布与下载都走 GitHub（本软件自己的仓库）
REPO="${GWS_REPO:-qq476605474/gws-desktop}"
API="https://api.github.com/repos/${REPO}"
DEST="${GWS_INSTALL_DIR:-$HOME/.local/bin}"

say()  { printf '%s\n' "$*"; }
fail() { printf '✗ %s\n' "$*" >&2; exit 1; }
info() { printf '▸ %s\n' "$*"; }
ok()   { printf '✓ %s\n' "$*"; }

# ── 平台探测 ──────────────────────────────────────────────────────────
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Darwin)
    case "$arch" in
      arm64) triple="aarch64-apple-darwin" ;;
      x86_64) triple="x86_64-apple-darwin" ;;
      *) fail "不支持的 macOS 架构: $arch" ;;
    esac
    ;;
  Linux)
    case "$arch" in
      x86_64) triple="x86_64-unknown-linux-gnu" ;;
      aarch64|arm64) triple="aarch64-unknown-linux-gnu" ;;
      *) fail "不支持的 Linux 架构: $arch" ;;
    esac
    ;;
  *) fail "不支持的平台: $os（Windows 请从 Releases 页下载 .exe）" ;;
esac
asset="gws-${triple}"
info "平台 $os/$arch → $asset"

for t in curl; do
  command -v "$t" >/dev/null 2>&1 || fail "需要 $t 命令"
done
# sha256 校验工具：macOS 是 shasum，多数 Linux 是 sha256sum
if command -v sha256sum >/dev/null 2>&1; then
  SHA="sha256sum"
elif command -v shasum >/dev/null 2>&1; then
  SHA="shasum -a 256"
else
  fail "需要 sha256sum 或 shasum 来校验下载内容"
fi

mkdir -p "$DEST"
work="$(mktemp -d "${TMPDIR:-/tmp}/gws-install.XXXXXX")" || fail "无法创建临时目录"
trap 'rm -rf "$work"' EXIT INT TERM

download() {
  curl -fsSL --proto '=https' --proto-redir '=https' \
    --connect-timeout 10 --max-time 300 --retry 2 --retry-delay 1 "$1" -o "$2" 2>/dev/null
}

# ── 取最新 Release ────────────────────────────────────────────────────
info "查询最新 Release"
download "${API}/releases/latest" "$work/release.json" \
  || fail "读取最新 Release 失败
     确认网络可达 api.github.com，以及仓库已发布过 Release"

# 从 JSON 里抠 tag 与资产地址。**不依赖 jq**（新机器不一定装了）——
# 用 sed/grep 取这几个字段足够了，格式由 GitLab 保证。
tag="$(sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$work/release.json" | head -1)"
[ -n "$tag" ] || fail "Release 里没有 tag_name"

version="${tag#v}"
case "$version" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) fail "Release 的 tag 不是 vX.Y.Z 形式: $tag" ;;
esac
info "最新版本 $version"

# 资产下载地址：GitHub 的 Release JSON 里，assets 是同级数组，
# 每个元素带 name 与 browser_download_url。这里取「URL 以资产名结尾」的那个。
asset_url="$(tr ',' '\n' < "$work/release.json" \
  | sed -n 's/.*"browser_download_url"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
  | grep "/${asset}$" | head -1)"
[ -n "$asset_url" ] || fail "Release $tag 里没有适配当前平台的资产 ${asset}
     可用资产见 https://github.com/${REPO}/releases/tag/${tag}
     若你是 Windows 用户，请从 Releases 页手工下载 .exe"

case "$asset_url" in
  "https://github.com/${REPO}/"*) ;;
  *) fail "资产地址不属于本仓库，已中止: $asset_url" ;;
esac

# ── 下载 ──────────────────────────────────────────────────────────────
info "下载 $asset"
download "$asset_url" "$work/gws" || fail "下载失败"

# ① 内容必须真的是可执行文件（防「地址配错拿回一个登录页」）
head_bytes="$(dd if="$work/gws" bs=4 count=1 2>/dev/null | od -An -tx1 | tr -d ' \n')"
case "$head_bytes" in
  7f454c46*|cffaedfe*|cefaedfe*|cafebabe*) ;;  # ELF / Mach-O
  *) fail "下载内容不是可执行文件（可能是登录页或错误页），已中止" ;;
esac
[ "$(wc -c < "$work/gws" | tr -d ' ')" -gt 100000 ] || fail "下载内容过小，不像是完整二进制"

# ② sha256 校验（Release 附了 sha256sums.txt 才做）
sums_url="$(tr ',' '\n' < "$work/release.json" \
  | sed -n 's/.*"browser_download_url"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
  | grep "/sha256sums.txt$" | head -1)"
if [ -n "$sums_url" ] && download "$sums_url" "$work/sums"; then
  want="$(grep "[[:space:]]\**${asset}$" "$work/sums" | head -1 | cut -d' ' -f1)"
  if [ -n "$want" ]; then
    got="$($SHA "$work/gws" | cut -d' ' -f1)"
    [ "$got" = "$want" ] || fail "sha256 校验失败，已中止
     期望 $want
     实际 $got"
    ok "sha256 校验通过"
  else
    info "校验文件里没有 $asset，跳过校验"
  fi
else
  info "Release 未附校验文件，跳过校验"
fi

chmod 755 "$work/gws"
mv -f "$work/gws" "$DEST/gws"
ok "gws $version 已安装: $DEST/gws"

# ── Skills 同步 ───────────────────────────────────────────────────────
#
# 技能文件随仓库走，从 **Release 对应的 tag** 取 raw 文件（不再另传资产，
# 避免「二进制更新了、技能还是旧的」）。只覆盖 gws 自己的技能目录；
# 每个目录写 .gws-version，设置页据此显示技能版本。
#
# 失败**不影响 CLI 安装**（技能是增强，不是运行前提），只提示。
# 各 AI CLI 读 skills 的目录：只写**已经存在**的，没有就不新建。
# 显式设置 GWS_SKILLS_DIR 时只写那一个（测试 / 自定义用）。
if [ -n "${GWS_SKILLS_DIR:-}" ]; then
  SKILLS_DIRS="$GWS_SKILLS_DIR"
else
  SKILLS_DIRS=""
  for d in \
    "$HOME/.claude/skills" \
    "$HOME/.config/opencode/skills" \
    "$HOME/.qoder/skills" \
    "$HOME/.codex/skills"; do
    [ -d "$d" ] || continue
    if [ -z "$SKILLS_DIRS" ]; then
      SKILLS_DIRS="$d"
    else
      SKILLS_DIRS="$SKILLS_DIRS
$d"
    fi
  done
fi

if [ "${GWS_NO_SKILLS:-0}" = "1" ]; then
  info "已按 GWS_NO_SKILLS=1 跳过 skills"
elif [ -z "$SKILLS_DIRS" ]; then
  info "没有找到已安装的 AI CLI skills 目录，跳过 skills（可用 GWS_SKILLS_DIR 指定）"
else
  # 技能文件随仓库走，从**这个 tag** 的原始文件取（不另传资产，
  # 避免「二进制更新了、技能还是旧的」）
  raw="https://raw.githubusercontent.com/${REPO}/${tag}/skills"
  skills_ok=0
  skills_fail=""
  for skill in gws; do
    tmp="$work/skill-$skill"
    if download "$raw/$skill/SKILL.md" "$tmp" && [ -s "$tmp" ] \
      && head -1 "$tmp" | grep -q '^---$'; then
      while IFS= read -r dir; do
        [ -n "$dir" ] || continue
        mkdir -p "$dir/$skill"
        cp -f "$tmp" "$dir/$skill/SKILL.md"
        printf '%s\n' "$version" > "$dir/$skill/.gws-version"
        skills_ok=$((skills_ok + 1))
      done <<EOF
$SKILLS_DIRS
EOF
    else
      skills_fail="$skills_fail $skill"
    fi
  done
  dirs_count=$(printf '%s\n' "$SKILLS_DIRS" | grep -c .)
  if [ -z "$skills_fail" ]; then
    ok "skills 已同步到 $dirs_count 个 AI CLI（$skills_ok 个文件）"
  elif [ "$skills_ok" -gt 0 ]; then
    info "skills 部分同步（$skills_ok 个文件），失败:$skills_fail"
  else
    info "skills 同步失败（不影响 CLI 使用），稍后可在设置页查看状态"
  fi
fi

# ── PATH 提示 ─────────────────────────────────────────────────────────
case ":$PATH:" in
  *":$DEST:"*) ;;
  *)
    say ""
    say "  $DEST 不在 PATH，加一行到 shell 配置："
    say "    echo 'export PATH=\"$DEST:\$PATH\"' >> ~/.zshrc && source ~/.zshrc"
    ;;
esac

say ""
say "  验证:  gws --version"
say "  更新:  gws update"