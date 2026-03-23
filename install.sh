#!/usr/bin/env bash

set -euo pipefail

REPO_SLUG="liquidos-ai/odyssey"
BIN_NAME="odyssey-rs"
INSTALL_DIR="${ODYSSEY_RS_INSTALL_DIR:-${HOME}/.local/bin}"
VERSION="${ODYSSEY_RS_VERSION:-}"
TMP_DIR=""

has_command() {
  command -v "$1" >/dev/null 2>&1
}

print_info() {
  printf '%s\n' "$*"
}

print_error() {
  printf 'error: %s\n' "$*" >&2
}

cleanup() {
  if [ -n "${TMP_DIR}" ] && [ -d "${TMP_DIR}" ]; then
    rm -rf "${TMP_DIR}"
  fi
}

require_command() {
  if ! has_command "$1"; then
    print_error "required command not found: $1"
    exit 1
  fi
}

resolve_version() {
  if [ -n "${VERSION}" ]; then
    printf '%s\n' "${VERSION#v}"
    return
  fi

  local latest_url
  latest_url="$(
    curl -fsSLI -o /dev/null -w '%{url_effective}' \
      "https://github.com/${REPO_SLUG}/releases/latest"
  )"

  local tag
  tag="${latest_url##*/}"
  tag="${tag#v}"

  if [ -z "${tag}" ]; then
    print_error "failed to resolve the latest release version"
    exit 1
  fi

  printf '%s\n' "${tag}"
}

detect_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "${os}:${arch}" in
    Linux:x86_64)
      printf '%s\n' "x86_64-unknown-linux-gnu"
      ;;
    Darwin:x86_64)
      printf '%s\n' "x86_64-apple-darwin"
      ;;
    Darwin:arm64 | Darwin:aarch64)
      printf '%s\n' "aarch64-apple-darwin"
      ;;
    *)
      print_error "unsupported platform: ${os} ${arch}"
      print_error "supported targets: Linux x86_64, macOS x86_64, macOS arm64"
      exit 1
      ;;
  esac
}

verify_sha256() {
  local checksum_file archive_file
  checksum_file="$1"
  archive_file="$2"

  if has_command sha256sum; then
    (
      cd "$(dirname "${archive_file}")"
      sha256sum -c "$(basename "${checksum_file}")"
    )
    return
  fi

  if has_command shasum; then
    (
      cd "$(dirname "${archive_file}")"
      shasum -a 256 -c "$(basename "${checksum_file}")"
    )
    return
  fi

  if has_command openssl; then
    local expected actual
    expected="$(awk '{print $1}' "${checksum_file}")"
    actual="$(openssl dgst -sha256 "${archive_file}" | awk '{print $NF}')"
    if [ "${expected}" = "${actual}" ]; then
      return
    fi

    print_error "checksum verification failed"
    exit 1
  fi

  print_error "no SHA-256 verifier found; install sha256sum, shasum, or openssl"
  exit 1
}

main() {
  require_command curl
  require_command tar
  require_command mktemp

  local version target archive_name checksum_name base_url archive_url checksum_url
  version="$(resolve_version)"
  target="$(detect_target)"
  archive_name="${BIN_NAME}-${version}-${target}.tar.gz"
  checksum_name="${archive_name}.sha256"
  base_url="https://github.com/${REPO_SLUG}/releases/download/v${version}"
  archive_url="${base_url}/${archive_name}"
  checksum_url="${base_url}/${checksum_name}"

  TMP_DIR="$(mktemp -d)"
  trap cleanup EXIT

  print_info "Installing ${BIN_NAME} ${version} for ${target}"
  print_info "Downloading release archive from GitHub Releases"

  curl -fsSL "${archive_url}" -o "${TMP_DIR}/${archive_name}"
  curl -fsSL "${checksum_url}" -o "${TMP_DIR}/${checksum_name}"
  verify_sha256 "${TMP_DIR}/${checksum_name}" "${TMP_DIR}/${archive_name}"

  mkdir -p "${TMP_DIR}/unpack"
  tar -xzf "${TMP_DIR}/${archive_name}" -C "${TMP_DIR}/unpack"

  if [ ! -f "${TMP_DIR}/unpack/${BIN_NAME}" ]; then
    print_error "archive did not contain ${BIN_NAME}"
    exit 1
  fi

  mkdir -p "${INSTALL_DIR}"
  cp "${TMP_DIR}/unpack/${BIN_NAME}" "${INSTALL_DIR}/${BIN_NAME}"
  chmod +x "${INSTALL_DIR}/${BIN_NAME}"

  print_info "Installed to ${INSTALL_DIR}/${BIN_NAME}"

  case ":${PATH}:" in
    *:"${INSTALL_DIR}":*)
      print_info "Try: ${BIN_NAME} --help"
      ;;
    *)
      print_info "Add ${INSTALL_DIR} to PATH, then run: ${BIN_NAME} --help"
      ;;
  esac
}

main "$@"
