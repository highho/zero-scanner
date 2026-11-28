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

makensis -V2 installer/Zero.nsi
ls -la "$OUT"
