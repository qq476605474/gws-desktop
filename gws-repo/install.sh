#!/bin/sh
# gws 一键安装/更新：curl -fsSL https://raw.githubusercontent.com/qq476605474/gws/main/install.sh | sh
set -e
dest="${GWS_INSTALL_DIR:-$HOME/.local/bin}"
url="https://raw.githubusercontent.com/qq476605474/gws/main/gws"
mkdir -p "$dest"
tmp="$dest/.gws.tmp.$$"
trap 'rm -f "$tmp"' EXIT
if ! curl -fsSL --max-time 60 "$url" -o "$tmp" 2>/dev/null; then
  echo "✗ 下载失败: $url（请检查网络）" >&2; exit 1
fi
chmod +x "$tmp"
mv "$tmp" "$dest/gws"
echo "✓ gws 已安装: $dest/gws"
case ":$PATH:" in
  *":$dest:"*) ;;
  *)
    echo
    echo "  $dest 不在 PATH，加一行到 ~/.zshrc（或对应 shell 配置）:"
    echo '  echo '\''export PATH="'$dest':$PATH"'\'' >> ~/.zshrc && source ~/.zshrc'
    ;;
esac
echo
echo "  验证:  gws --help"
echo "  更新:  gws update"
