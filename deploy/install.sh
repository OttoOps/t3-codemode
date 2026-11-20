#!/usr/bin/env bash
set -euo pipefail

REPO=OttoOps/t3-codemode
RELEASES="${T3CM_RELEASES_URL:-https://github.com/$REPO/releases}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]:-.}")" && pwd)"
CRATE="$(dirname "$HERE")"
BIN_DIR="$HOME/.local/bin"
UNIT=t3-codemode.service
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
LABEL=com.ottoops.t3-codemode
AGENT_DIR="$HOME/Library/LaunchAgents"
MAC_LOGS="$HOME/Library/Logs/t3-codemode"
MCP_NAME=t3cm
MCP_URL=http://127.0.0.1:47731/mcp
# shellcheck disable=SC2016
MCP_ADD=(claude mcp add --transport http --scope user "$MCP_NAME" "$MCP_URL" --header 'X-T3-Thread: ${T3CODE_THREAD_ID}')

usage() {
  cat <<EOF
usage: $0 [--version vX.Y.Z] [--target TRIPLE] [--files-only] [--workflow | --no-workflow]
       $0 --archive FILE --sums SHA256SUMS [--files-only] [--workflow | --no-workflow]
       $0 --from-source [--files-only] [--workflow | --no-workflow]
       $0 --uninstall [--files-only]

Installs t3cm to ~/.local/bin and its user service (a systemd user unit on
Linux, a launchd agent on macOS), then registers the MCP server with Claude
Code. By default it downloads the latest release from $REPO and checks
it against the release's SHA256SUMS, and with gh signed in, its build
provenance too. Then it asks whether to use the T3 workflow, and on yes runs
t3cm skills install, which writes the release's workflow skills and applies
their setup steps. Skills that npx skills or a copy by hand installed are
replaced (t3cm skills install --take-over) unless you say no. A rerun brings
skills t3cm installed to the new release without asking.

  --version vX.Y.Z   install that release instead of the latest
  --target TRIPLE    release build to fetch (default: musl on Linux, the Mac's own arch)
  --archive, --sums  install a release archive already on disk, checked against SHA256SUMS
  --from-source      build this checkout with cargo instead
  --files-only       install or remove the files without touching systemd or launchd
  --workflow         install, take over or update the workflow skills without asking (the default with no terminal)
  --no-workflow      leave the workflow skills alone
  --uninstall        stop and remove the service, the binary and the MCP entry; keep the store and the skills
EOF
}

die() {
  echo "install.sh: $*" >&2
  exit 1
}

case "$(uname -s)" in
  Linux) OS=linux ;;
  Darwin) OS=macos ;;
  *) die "unsupported OS $(uname -s); on Windows run deploy/install.ps1" ;;
esac

MODE=release VERSION="" TARGET="" ARCHIVE="" SUMS="" FILES_ONLY="" WORKFLOW=""
set_workflow() { if [ -n "$WORKFLOW" ] && [ "$WORKFLOW" != "$1" ]; then die "--workflow and --no-workflow cannot be combined"; fi; WORKFLOW="$1"; }
need() { if [ "$#" -lt 2 ] || [ -z "$2" ]; then die "$1 needs a value"; fi; }
set_mode() { if [ "$MODE" != release ] && [ "$MODE" != "$1" ]; then die "--from-source and --uninstall cannot be combined"; fi; MODE="$1"; }
while [ "$#" -gt 0 ]; do
  case "$1" in
    --version) need "$@"; VERSION="$2"; shift 2 ;;
    --target) need "$@"; TARGET="$2"; shift 2 ;;
    --archive) need "$@"; ARCHIVE="$2"; shift 2 ;;
    --sums) need "$@"; SUMS="$2"; shift 2 ;;
    --from-source) set_mode source; shift ;;
    --uninstall) set_mode uninstall; shift ;;
    --files-only) FILES_ONLY=1; shift ;;
    --workflow) set_workflow yes; shift ;;
    --no-workflow) set_workflow no; shift ;;
    -h | --help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done
if [ -n "$ARCHIVE$SUMS" ]; then
  if [ -z "$ARCHIVE" ] || [ -z "$SUMS" ]; then die "--archive and --sums go together"; fi
  if [ "$MODE" != release ] || [ -n "$VERSION$TARGET" ]; then die "--archive cannot be combined with --version, --target, --from-source or --uninstall"; fi
fi
if [ "$MODE" != release ] && [ -n "$VERSION$TARGET" ]; then
  die "--version and --target pick a release; they do not apply to --from-source or --uninstall"
fi
if [ "$MODE" = uninstall ] && [ -n "$WORKFLOW" ]; then
  die "--workflow and --no-workflow do not apply to --uninstall"
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

detect_target() {
  local arch
  case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) die "no release build for $(uname -m); use --from-source" ;;
  esac
  case "$OS" in
    linux) echo "$arch-unknown-linux-musl" ;;
    macos)
      if [ "$arch" = x86_64 ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null)" = 1 ]; then
        arch=aarch64
      fi
      echo "$arch-apple-darwin"
      ;;
  esac
}

fetch() {
  curl -fsSL --proto-redir "=${RELEASES%%:*}" -o "$2" "$1" || die "download failed: $1"
}

latest_tag() {
  local url
  url="$(curl -fsSLI --proto-redir "=${RELEASES%%:*}" -o /dev/null -w '%{url_effective}' "$RELEASES/latest")" \
    || die "could not reach $RELEASES/latest"
  case "${url##*/}" in
    v[0-9]*) echo "${url##*/}" ;;
    *) die "no published release at $RELEASES yet; use --from-source" ;;
  esac
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

verify_sum() {
  local name want got
  name="$(basename "$1")"
  want="$(awk -v n="$name" '{ f = $2; sub(/^\*/, "", f) } f == n { print $1; exit }' "$2")"
  [ -n "$want" ] || die "$name is not listed in $2"
  got="$(sha256 "$1")"
  [ "$got" = "$want" ] || die "checksum mismatch for $name: SHA256SUMS says $want, the file is $got"
  echo "Checksum OK: $name"
}

verify_provenance() {
  if ! command -v gh >/dev/null 2>&1 || ! gh auth status >/dev/null 2>&1; then
    echo "gh is missing or not signed in; skipped the provenance check (gh attestation verify)."
    return
  fi
  gh attestation verify "$1" -R "$REPO" >/dev/null || die "gh attestation verify failed for $(basename "$1")"
  echo "Provenance OK: $(basename "$1") has a build attestation from $REPO"
}

has_user_systemd() {
  command -v systemctl >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1
}

plist_value() {
  case "$1" in
    *[\&\<\>\|\\]*) die "the path $1 has a character the launchd plist cannot carry (& < > | \\)" ;;
  esac
}

render_plist() {
  plist_value "$HOME"
  sed -e "s|@BIN@|$BIN_DIR/t3cm|g" -e "s|@LOGS@|$MAC_LOGS|g" -e "s|@HOME@|$HOME|g" "$1" >"$2"
  plutil -lint "$2" >/dev/null || die "the rendered plist $2 does not lint"
  chmod 0644 "$2"
}

launchd_loaded() {
  launchctl print "gui/$(id -u)/$LABEL" >/dev/null 2>&1
}

launchd_unload() {
  launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
  local i
  for i in $(seq 50); do
    launchd_loaded || return 0
    sleep 0.2
  done
  die "launchd still has $LABEL loaded after bootout (tried $i times)"
}

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

skills_state() {
  "$BIN_DIR/t3cm" skills status --json 2>/dev/null || true
}

ask() {
  local answer
  if [ ! -t 0 ]; then
    echo "$1 Yes: there is no terminal to ask on (--no-workflow says no)."
    return 0
  fi
  read -r -p "$1 [Y/n] " answer || answer=""
  case "$answer" in
    [nN]*) return 1 ;;
    *) return 0 ;;
  esac
}

install_workflow() {
  local state question
  local -a cmd
  state="$(skills_state)"
  case "$state" in
    *'"installed": "'*) cmd=(update) ;;
    *'"foreign": {'*)
      cmd=(install --take-over)
      question="npx skills or a copy by hand installed the T3 workflow skills here. Replace them with this release's, and have t3cm keep them current?"
      ;;
    *)
      cmd=(install)
      question="Use the T3 workflow?"
      ;;
  esac
  case "$WORKFLOW:${cmd[0]}" in
    no:*)
      echo "Left the T3 workflow skills alone; to have t3cm manage them, run: t3cm skills ${cmd[*]}"
      return
      ;;
    yes:*) ;;
    :update) echo "t3cm installed the T3 workflow skills here, so this install brings them to its release (--no-workflow skips that)." ;;
    *)
      if ! ask "$question"; then
        echo "Left the T3 workflow skills alone; to have t3cm manage them, run: t3cm skills ${cmd[*]}"
        return
      fi
      ;;
  esac
  if ! "$BIN_DIR/t3cm" skills "${cmd[@]}"; then
    SKILLS_FAILED=1
  fi
}

install_files() {
  local bin="$1" svc="$2"
  case "$OS" in
    linux) install -m 0644 "$svc/$UNIT" "$WORK/$UNIT" ;;
    macos) render_plist "$svc/$LABEL.plist" "$WORK/$LABEL.plist" ;;
  esac
  mkdir -p "$BIN_DIR"
  install -m 0755 "$bin" "$BIN_DIR/t3cm.new"
  mv -f "$BIN_DIR/t3cm.new" "$BIN_DIR/t3cm"
  case "$OS" in
    linux)
      mkdir -p "$UNIT_DIR"
      mv -f "$WORK/$UNIT" "$UNIT_DIR/$UNIT"
      if [ -n "$FILES_ONLY" ]; then
        echo "Installed $UNIT_DIR/$UNIT; start it with: systemctl --user daemon-reload && systemctl --user enable --now $UNIT"
      elif has_user_systemd; then
        systemctl --user daemon-reload
        systemctl --user enable "$UNIT"
        systemctl --user restart "$UNIT"
      else
        echo "No systemd user session here; installed $UNIT_DIR/$UNIT but did not start it. Run t3cm serve yourself."
      fi
      LOGS_HINT="journalctl --user -u t3-codemode"
      ;;
    macos)
      mkdir -p "$AGENT_DIR" "$MAC_LOGS"
      mv -f "$WORK/$LABEL.plist" "$AGENT_DIR/$LABEL.plist"
      if [ -n "$FILES_ONLY" ]; then
        # shellcheck disable=SC2016
        printf 'Installed %s; start it with: launchctl enable gui/$(id -u)/%s && launchctl bootstrap gui/$(id -u) %q\n' "$AGENT_DIR/$LABEL.plist" "$LABEL" "$AGENT_DIR/$LABEL.plist"
      else
        if launchd_loaded; then
          launchd_unload
        fi
        launchctl enable "gui/$(id -u)/$LABEL"
        launchctl bootstrap "gui/$(id -u)" "$AGENT_DIR/$LABEL.plist"
      fi
      LOGS_HINT="$MAC_LOGS/t3cm.log"
      ;;
  esac
  install_workflow
  register_mcp
  echo "t3-codemode $("$BIN_DIR/t3cm" --version | awk '{print $2}') installed. Check it with: t3cm status   Logs: $LOGS_HINT"
  case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "$BIN_DIR is not on your PATH; add it to run t3cm by name." ;;
  esac
  if [ -n "${SKILLS_FAILED:-}" ]; then
    die "t3cm is installed, but its workflow skills are not; fix what t3cm skills install printed, then run it again"
  fi
}

install_release() {
  local archive sums name dir tag
  if [ -n "$ARCHIVE" ]; then
    archive="$ARCHIVE" sums="$SUMS"
  else
    if [[ ! "$RELEASES" =~ ^(https://|http://(127\.0\.0\.1|localhost)(:[0-9]+)?(/|$)) ]]; then
      die "T3CM_RELEASES_URL must be https (plain http only on 127.0.0.1 or localhost): $RELEASES"
    fi
    tag="${VERSION:-$(latest_tag)}"
    case "$tag" in v*) ;; *) tag="v$tag" ;; esac
    name="t3cm-${tag#v}-${TARGET:-$(detect_target)}.tar.gz"
    fetch "$RELEASES/download/$tag/$name" "$WORK/$name"
    fetch "$RELEASES/download/$tag/SHA256SUMS" "$WORK/SHA256SUMS"
    archive="$WORK/$name" sums="$WORK/SHA256SUMS"
  fi
  verify_sum "$archive" "$sums"
  if [ -n "$ARCHIVE" ]; then
    echo "Local archive: provenance not checked; run gh attestation verify $archive -R $REPO for a release download."
  else
    verify_provenance "$archive"
  fi
  name="$(basename "$archive")"
  case "$name" in
    t3cm-*.tar.gz) ;;
    *) die "$name is not a t3cm release archive (t3cm-<version>-<target>.tar.gz)" ;;
  esac
  mkdir "$WORK/x"
  tar -xzf "$archive" -C "$WORK/x"
  dir="$WORK/x/${name%.tar.gz}"
  [ -f "$dir/t3cm" ] || die "$name has no ${name%.tar.gz}/t3cm"
  "$dir/t3cm" --version >/dev/null 2>&1 || die "the t3cm in $name does not run on this machine; is it the right target?"
  install_files "$dir/t3cm" "$dir"
}

install_source() {
  if [ ! -f "$CRATE/Cargo.toml" ] || [ ! -f "$HERE/$UNIT" ]; then die "--from-source runs from a t3-codemode checkout"; fi
  (cd "$CRATE" && CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-8}" cargo build --release --locked)
  install_files "$CRATE/target/release/t3cm" "$HERE"
}

uninstall_all() {
  local store
  case "$OS" in
    linux)
      if [ -z "$FILES_ONLY" ] && has_user_systemd; then
        systemctl --user disable "$UNIT" 2>/dev/null || true
        if systemctl --user cat "$UNIT" >/dev/null 2>&1; then
          systemctl --user stop "$UNIT" || die "could not stop $UNIT; nothing removed, run --uninstall again"
        fi
      fi
      rm -f "$UNIT_DIR/$UNIT"
      if [ -z "$FILES_ONLY" ] && has_user_systemd; then
        systemctl --user daemon-reload
      fi
      store="${XDG_STATE_HOME:-$HOME/.local/state}/t3-codemode"
      ;;
    macos)
      if [ -z "$FILES_ONLY" ] && launchd_loaded; then
        launchd_unload
      fi
      rm -f "$AGENT_DIR/$LABEL.plist"
      store="$HOME/Library/Application Support/t3-codemode (logs: $MAC_LOGS)"
      ;;
  esac
  rm -f "$BIN_DIR/t3cm"
  if command -v claude >/dev/null 2>&1; then
    claude mcp remove --scope user "$MCP_NAME" >/dev/null 2>&1 || true
  else
    echo "claude is not on PATH; remove the MCP entry with: claude mcp remove --scope user $MCP_NAME"
  fi
  echo "t3-codemode removed. Its store stays in $store; delete it to drop memory and links."
}

case "$MODE" in
  release) install_release ;;
  source) install_source ;;
  uninstall) uninstall_all ;;
esac
