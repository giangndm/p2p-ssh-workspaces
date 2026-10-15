#!/usr/bin/env bash
set -euo pipefail

REPO="${SSHP_REPO:-giangndm/ssh-over-iroh}"
BIN_NAME="sshp"
INSTALL_DIR="${SSHP_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${SSHP_VERSION:-latest}"

log() {
  printf "==> %s\n" "$*" >&2
}

fail() {
  printf "error: %s\n" "$*" >&2
  exit 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

detect_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Darwin)
      case "$arch" in
        arm64|aarch64) printf "aarch64-apple-darwin" ;;
        *) fail "unsupported macOS architecture: $arch (supported: arm64)" ;;
      esac
      ;;
    Linux)
      case "$arch" in
        x86_64|amd64) printf "x86_64-unknown-linux-musl" ;;
        *) fail "unsupported Linux architecture: $arch (supported: x86_64)" ;;
      esac
      ;;
    *)
      fail "unsupported operating system: $os (supported: macOS, Linux)"
      ;;
  esac
}

verify_sha256() {
  local dir="$1"
  local checksum_file="$2"

  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$dir" && sha256sum -c "$checksum_file" >/dev/null)
  elif command -v shasum >/dev/null 2>&1; then
    (cd "$dir" && shasum -a 256 -c "$checksum_file" >/dev/null)
  else
    fail "neither sha256sum nor shasum is available to verify checksum"
  fi
}

main() {
  require_cmd curl
  require_cmd tar
  require_cmd uname
  require_cmd mktemp

  local target archive checksum base_url
  target="$(detect_target)"
  archive="${BIN_NAME}-${target}.tar.gz"
  checksum="${archive}.sha256"

  if [ "$VERSION" = "latest" ]; then
    base_url="https://github.com/${REPO}/releases/latest/download"
  else
    case "$VERSION" in
      v*) base_url="https://github.com/${REPO}/releases/download/${VERSION}" ;;
      *) base_url="https://github.com/${REPO}/releases/download/v${VERSION}" ;;
    esac
  fi

  TMP_DIR="$(mktemp -d)"
  trap 'rm -rf "${TMP_DIR:-}"' EXIT

  log "Downloading ${archive} (${VERSION}) from ${REPO}..."
  curl -fsSL "${base_url}/${archive}" -o "${TMP_DIR}/${archive}"
  curl -fsSL "${base_url}/${checksum}" -o "${TMP_DIR}/${checksum}"

  log "Verifying SHA-256 checksum..."
  verify_sha256 "$TMP_DIR" "$checksum"

  log "Extracting ${BIN_NAME}..."
  tar -xzf "${TMP_DIR}/${archive}" -C "$TMP_DIR"

  mkdir -p "$INSTALL_DIR"
  install -m 0755 "${TMP_DIR}/${BIN_NAME}" "${INSTALL_DIR}/${BIN_NAME}"

  log "Installed ${BIN_NAME} to ${INSTALL_DIR}/${BIN_NAME}"
  "${INSTALL_DIR}/${BIN_NAME}" --version >&2 || true

  case ":${PATH}:" in
    *":${INSTALL_DIR}:"*) ;;
    *)
      log "Note: ${INSTALL_DIR} is not in your PATH. Add it with:"
      log "  export PATH=\"${INSTALL_DIR}:\$PATH\""
      ;;
  esac
}

main "$@"
