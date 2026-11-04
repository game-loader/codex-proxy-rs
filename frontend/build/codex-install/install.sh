#!/bin/sh
# Codex Proxy RS 一键配置：升级 Codex CLI 到最新版，并按网关模板写入用户配置
# 由前端构建生成，配置模板来自 src/utils/codex-config.ts
set -eu

CODEX_RELEASES_URL="https://releases.openai.com/codex"
CODEX_INSTALLER_URL="https://chatgpt.com/codex/install.sh"

base_url=""
api_key=""
websocket_enabled="false"
temporary_file=""
temporary_config=""

step() {
  printf '==> %s\n' "$1"
}

warn() {
  printf 'WARNING: %s\n' "$1" >&2
}

fail() {
  printf 'ERROR: %s\n' "$1" >&2
  exit 1
}

cleanup() {
  rm -f ${temporary_file:+"$temporary_file"} ${temporary_config:+"$temporary_config"}
}
trap cleanup EXIT
trap 'exit 130' INT TERM

usage() {
  cat <<'USAGE'
Usage: sh install.sh --base-url <gateway>/v1 --key <client-api-key> [--websocket]

Upgrades Codex CLI to the latest release and writes the gateway configuration
to $CODEX_HOME/config.toml (default ~/.codex/config.toml). An existing file is
backed up next to it before it is replaced.
USAGE
}

while [ $# -gt 0 ]; do
  case "$1" in
    --base-url)
      [ $# -ge 2 ] || fail "--base-url requires a value."
      base_url="$2"
      shift 2
      ;;
    --key)
      [ $# -ge 2 ] || fail "--key requires a value."
      api_key="$2"
      shift 2
      ;;
    --websocket)
      websocket_enabled="true"
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      fail "Unknown option: $1. Run with --help for usage."
      ;;
  esac
done

case "$base_url" in
  http://?* | https://?*) ;;
  *) fail "--base-url must be an http:// or https:// gateway address." ;;
esac
while :; do
  case "$base_url" in
    */) base_url="${base_url%/}" ;;
    *) break ;;
  esac
done
[ -n "$api_key" ] || fail "--key is required."

fetch() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO- "$1"
  else
    return 1
  fi
}

# 写入 TOML 基本字符串；控制字符无法安全写入单行值，直接拒绝
toml_string() {
  if [ "$(printf '%s' "$1" | LC_ALL=C tr -d '[:cntrl:]')" != "$1" ]; then
    return 1
  fi
  printf '"%s"' "$(printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g')"
}

codex_version() {
  "$1" --version 2>/dev/null | head -n 1 | awk '{ print $NF }'
}

latest_codex_version() {
  fetch "$CODEX_RELEASES_URL/channels/latest" 2>/dev/null |
    sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"rust-v\([^"]*\)".*/\1/p' |
    head -n 1
}

# 按实际执行的 codex 判断来源：Node 入口归 npm／bun，Homebrew 前缀下归 brew，其余交给官方独立安装
codex_manager() {
  if [ -f "$1" ] && head -n 1 "$1" 2>/dev/null | grep -q node; then
    case "$1" in
      *"/.bun/"*) echo bun ;;
      *) echo npm ;;
    esac
    return
  fi
  if command -v brew >/dev/null 2>&1; then
    brew_prefix="$(brew --prefix 2>/dev/null || true)"
    case "$1" in
      "$brew_prefix"/*)
        if brew list --cask codex >/dev/null 2>&1; then
          echo brew-cask
          return
        elif brew list --formula codex >/dev/null 2>&1; then
          echo brew-formula
          return
        fi
        ;;
    esac
  fi
  echo standalone
}

run_official_installer() {
  temporary_file="$(mktemp "${TMPDIR:-/tmp}/codex-install.XXXXXX")" || return 1
  fetch "$CODEX_INSTALLER_URL" >"$temporary_file" || return 1
  CODEX_NON_INTERACTIVE=1 sh "$temporary_file" || return 1
}

# 在 if 条件中调用时 set -e 不生效，每一步都显式返回失败
upgrade_codex() {
  case "$1" in
    npm)
      step "Running: npm install -g @openai/codex@latest"
      npm install -g @openai/codex@latest || return 1
      ;;
    bun)
      step "Running: bun add -g @openai/codex@latest"
      bun add -g @openai/codex@latest || return 1
      ;;
    brew-cask)
      step "Running: brew upgrade --cask codex"
      brew upgrade --cask codex || return 1
      ;;
    brew-formula)
      step "Running: brew upgrade codex"
      brew upgrade codex || return 1
      ;;
    *)
      step "Running the official Codex installer"
      run_official_installer || return 1
      ;;
  esac
}

resolve_codex() {
  standalone_path="${CODEX_INSTALL_DIR:-$HOME/.local/bin}/codex"
  if command -v codex >/dev/null 2>&1; then
    command -v codex
  elif [ -x "$standalone_path" ]; then
    printf '%s\n' "$standalone_path"
  fi
}

base_url_toml="$(toml_string "$base_url")" || fail "--base-url contains control characters."
model_catalog_url_toml="$(toml_string "$base_url/models")" || fail "--base-url contains control characters."
api_key_toml="$(toml_string "$api_key")" || fail "--key contains control characters."

latest_version="$(latest_codex_version || true)"
codex_path="$(resolve_codex)"
if [ -z "$codex_path" ]; then
  step "Codex CLI not found"
  manager="standalone"
  current_version=""
else
  manager="$(codex_manager "$codex_path")"
  current_version="$(codex_version "$codex_path" || true)"
  step "Found Codex ${current_version:-of unknown version} at $codex_path ($manager)"
fi

if [ -n "$latest_version" ] && [ "$current_version" = "$latest_version" ]; then
  step "Codex $current_version is already the latest release"
else
  if [ -n "$latest_version" ]; then
    step "Updating Codex to $latest_version"
  else
    warn "Could not read the latest Codex release. Updating with $manager anyway."
  fi
  if ! upgrade_codex "$manager"; then
    if [ -z "$codex_path" ]; then
      fail "Codex installation failed. Install it with: curl -fsSL $CODEX_INSTALLER_URL | sh"
    fi
    warn "Codex update failed. Continuing with the installed version."
  fi
  codex_path="$(resolve_codex)"
  [ -n "$codex_path" ] || fail "Codex was installed but could not be found. Open a new terminal and run this command again."
  current_version="$(codex_version "$codex_path" || true)"
  # 多份安装并存时 PATH 顺序决定实际运行的版本
  if [ -n "$latest_version" ] && [ "$current_version" != "$latest_version" ]; then
    warn "$codex_path still reports ${current_version:-an unknown version}. Check PATH for another Codex install."
  fi
fi

codex_home="${CODEX_HOME:-$HOME/.codex}"
config_path="$codex_home/config.toml"
mkdir -p "$codex_home"
if [ -e "$config_path" ]; then
  backup_path="$config_path.bak.$(date +%Y%m%d%H%M%S)"
  cp -p "$config_path" "$backup_path"
  step "Backed up the existing config to $backup_path"
fi

temporary_config="$config_path.tmp.$$"
# 模板的静态部分不含 $、反引号或反斜杠，heredoc 只展开下方已编码的 TOML 值
(
  umask 077
  cat >"$temporary_config" <<CPR_CODEX_CONFIG
@@CODEX_CONFIG@@
CPR_CODEX_CONFIG
)
chmod 600 "$temporary_config"
mv -f "$temporary_config" "$config_path"
temporary_config=""

step "Wrote $config_path"
step "Codex ${current_version:-(unknown version)} now uses $base_url"
step "Quit running Codex sessions, then start a new one: codex"
