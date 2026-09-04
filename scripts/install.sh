#!/usr/bin/env bash
# video-studio 安装脚本。
#
#   install.sh [--prefix DIR] [--version TAG] [--init BUNDLE] [--force]
#
#   --prefix DIR    安装到哪里，默认 /opt/video-studio
#   --version TAG   装哪个版本，默认最新 release
#   --init BUNDLE   顺带初始化一部作品；**默认不初始化**
#   --force         目标目录已存在时覆盖
#
# 两种用法：
#   1. 直接跑（会去 GitHub 下载）：
#        curl -fsSL https://raw.githubusercontent.com/sunsheng/video-studio/main/scripts/install.sh | bash
#   2. 手动下载 zip 解压后，在解压出来的目录里跑 ./install.sh —— 不联网，就地安装。
#
# 也可以完全不用这个脚本：把 zip 解压到任意目录直接用，程序不依赖安装位置。

set -euo pipefail

REPO="sunsheng/video-studio"
PREFIX="/opt/video-studio"
VERSION=""
INIT_PATH=""
FORCE=0

die() { printf '错误：%s\n' "$*" >&2; exit 1; }
info() { printf '%s\n' "$*"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --prefix)  PREFIX="${2:?--prefix 需要一个目录}"; shift 2 ;;
    --version) VERSION="${2:?--version 需要一个 tag}"; shift 2 ;;
    --init)    INIT_PATH="${2:?--init 需要一个作品路径}"; shift 2 ;;
    --force)   FORCE=1; shift ;;
    -h|--help) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *)         die "未知参数：$1（用 --help 看用法）" ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STAGE=""
CLEANUP=""
trap '[ -n "$CLEANUP" ] && rm -rf "$CLEANUP"' EXIT

if [ -x "$SCRIPT_DIR/studiod" ] && [ -x "$SCRIPT_DIR/studio-cli" ]; then
  # 就地安装：脚本和两个二进制在同一个目录里，说明用户已经解压好了。
  info "从 $SCRIPT_DIR 就地安装"
  STAGE="$SCRIPT_DIR"
else
  command -v curl >/dev/null 2>&1 || die "需要 curl。或者手动下载 zip 解压后在里面跑 ./install.sh。"
  command -v unzip >/dev/null 2>&1 || die "需要 unzip。或者手动下载 zip 解压后在里面跑 ./install.sh。"

  if [ -z "$VERSION" ]; then
    info "查询最新版本…"
    VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
      | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)"
    [ -n "$VERSION" ] || die "查不到最新版本。用 --version 指定，或手动下载。"
  fi

  ASSET="video-studio-${VERSION}-linux-x86_64.zip"
  URL="https://github.com/$REPO/releases/download/$VERSION/$ASSET"
  TMP="$(mktemp -d)"
  CLEANUP="$TMP"
  info "下载 $VERSION …"
  curl -fSL --progress-bar "$URL" -o "$TMP/pkg.zip" || die "下载失败：$URL"
  unzip -q "$TMP/pkg.zip" -d "$TMP/x"
  STAGE="$TMP/x/video-studio"
  [ -x "$STAGE/studiod" ] && [ -x "$STAGE/studio-cli" ] \
    || die "包里没有可执行的 studiod / studio-cli，压缩包可能损坏。"
fi

if [ -e "$PREFIX" ] && [ "$FORCE" -ne 1 ]; then
  # 升级是覆盖安装，但要用户明确同意，免得误删别的目录。
  if [ -x "$PREFIX/studiod" ]; then
    info "$PREFIX 已有一份安装，将覆盖升级。"
  else
    die "$PREFIX 已存在且不像是 video-studio 的安装目录。换个 --prefix，或加 --force。"
  fi
fi

SUDO=""
PARENT="$(dirname "$PREFIX")"
if [ ! -w "$PARENT" ] && [ "$(id -u)" -ne 0 ]; then
  command -v sudo >/dev/null 2>&1 || die "没有 $PARENT 的写权限，也没有 sudo。用 --prefix 换一个你能写的目录，例如 --prefix \$HOME/video-studio"
  SUDO="sudo"
  info "需要提权写入 $PREFIX"
fi

$SUDO mkdir -p "$PREFIX"
$SUDO cp -R "$STAGE"/. "$PREFIX"/
$SUDO chmod +x "$PREFIX/studiod" "$PREFIX/studio-cli"

# 用户自己的 .env 不能被升级覆盖掉。
if [ ! -f "$PREFIX/.env" ] && [ -f "$PREFIX/.env.example" ]; then
  info "提示：按需复制 $PREFIX/.env.example 为 $PREFIX/.env 配置 ffmpeg 路径与 ComfyUI 节点。"
fi

info ""
info "已安装到 $PREFIX"
"$PREFIX/studiod" --version
"$PREFIX/studio-cli" --version

if [ -n "$INIT_PATH" ]; then
  info ""
  "$PREFIX/studio-cli" init "$INIT_PATH"
else
  info ""
  info "默认没有初始化作品。要新建一部："
  info "  $PREFIX/studio-cli init ~/videos/我的第一部.studio"
fi

info ""
info "建议先体检一次："
info "  $PREFIX/studio-cli doctor"
