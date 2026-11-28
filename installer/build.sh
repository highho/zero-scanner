#!/usr/bin/env bash
# 构建 Windows 安装包（NSIS）。
# 依赖：NSIS 3.x —— 只需要 makensis 在 PATH 上（https://nsis.sourceforge.io/Download）
#       Linux 上也可以：apt install nsis
# 用法：bash installer/build.sh
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^!define APP_VERSION "\(.*\)"$/\1/p' installer/Zero.nsi)
OUT="dist/zero-scanner-${VERSION}-setup-x64.exe"

test -f dist/zero-scanner.exe || {
  echo "缺少 dist/zero-scanner.exe：先 cargo build --release，再把产物复制到 dist/" >&2
  exit 1
}
command -v makensis >/dev/null 2>&1 || {
  echo "找不到 makensis：装 NSIS 3.x，并把它的目录加进 PATH" >&2
  exit 1
}

# NSIS 只有在文件带 UTF-8 BOM 时才按 UTF-8 解析；缺了会把中文读成乱码并直接报
# "Bad text encoding"（编辑器/脚本改过这个文件很容易把 BOM 弄丢），这里自动补上。
if [ "$(head -c 3 installer/Zero.nsi | od -An -tx1 | tr -d ' \n')" != "efbbbf" ]; then
  echo "installer/Zero.nsi 缺少 UTF-8 BOM，自动补上"
  { printf '\xef\xbb\xbf'; cat installer/Zero.nsi; } > installer/Zero.nsi.tmp
  mv installer/Zero.nsi.tmp installer/Zero.nsi
fi

makensis -V2 installer/Zero.nsi
ls -la "$OUT"
