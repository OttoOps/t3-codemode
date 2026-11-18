#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE="$(dirname "$HERE")"
BIN_DIR="$HOME/.local/bin"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
UNIT=t3-codemode.service
MCP_NAME=t3cm
MCP_URL=http://127.0.0.1:47731/mcp
# shellcheck disable=SC2016
MCP_ADD=(claude mcp add --transport http --scope user "$MCP_NAME" "$MCP_URL" --header 'X-T3-Thread: ${T3CODE_THREAD_ID}')

register_mcp() {
  if ! command -v claude >/dev/null 2>&1; then
    echo "claude is not on PATH; register the MCP server yourself:"
    printf '  %q' "${MCP_ADD[@]}"
    echo
    return
  fi
  if claude mcp get "$MCP_NAME" >/dev/null 2>&1; then
    echo "Claude Code already has an MCP server named $MCP_NAME; left as is."
  else
    "${MCP_ADD[@]}"
  fi
}

install_all() {
  (cd "$CRATE" && CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo build --release --locked)
  mkdir -p "$BIN_DIR" "$UNIT_DIR"
  install -m 0755 "$CRATE/target/release/t3cm" "$BIN_DIR/t3cm.new"
  mv -f "$BIN_DIR/t3cm.new" "$BIN_DIR/t3cm"
  install -m 0644 "$HERE/$UNIT" "$UNIT_DIR/$UNIT"
  systemctl --user daemon-reload
  systemctl --user enable "$UNIT"
  systemctl --user restart "$UNIT"
  register_mcp
  echo "t3-codemode installed. Check it with: t3cm status   Logs: journalctl --user -u t3-codemode"
}

uninstall_all() {
  systemctl --user disable --now "$UNIT" 2>/dev/null || true
  rm -f "$UNIT_DIR/$UNIT"
  systemctl --user daemon-reload
  rm -f "$BIN_DIR/t3cm"
  if command -v claude >/dev/null 2>&1; then
    claude mcp remove --scope user "$MCP_NAME" >/dev/null 2>&1 || true
  else
    echo "claude is not on PATH; remove the MCP entry with: claude mcp remove --scope user $MCP_NAME"
  fi
  echo "t3-codemode removed. Its store stays in ${XDG_STATE_HOME:-$HOME/.local/state}/t3-codemode; delete it to drop memory and links."
}

case "${1:-}" in
  "") install_all ;;
  --uninstall) uninstall_all ;;
  *) echo "usage: $0 [--uninstall]" >&2; exit 2 ;;
esac
