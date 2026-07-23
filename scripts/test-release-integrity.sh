#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
installers=(
  "$root_dir/website/public/install.sh"
  "$root_dir/website/public/install-crostini.sh"
)

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

run_case() (
  local installer="$1"
  local case_name="$2"
  local case_dir="$3"
  local asset_name="test-release-asset"
  local download_path="$case_dir/downloaded-asset"

  curl() {
    local url=""
    local output=""

    while [ "$#" -gt 0 ]; do
      case "$1" in
        -o)
          output="$2"
          shift 2
          ;;
        -*)
          shift
          ;;
        *)
          url="$1"
          shift
          ;;
      esac
    done

    local source_file="$case_dir/${url##*/}"
    [ -f "$source_file" ] || return 22
    cp "$source_file" "$output"
  }

  export JSTORRENT_INSTALLER_LIB_ONLY=1
  # shellcheck source=/dev/null
  source "$installer"
  unset JSTORRENT_INSTALLER_LIB_ONLY

  TMP_DIR="$case_dir/tmp"
  BASE_URL="https://example.invalid/release"
  CHECKSUMS_FALLBACK_BASE_URL="https://example.invalid/checksums"
  TAG="v-test"
  mkdir -p "$TMP_DIR"

  case "$case_name" in
    success)
      printf 'trusted release bytes\n' > "$case_dir/$asset_name"
      (
        cd "$case_dir"
        sha256sum "$asset_name" > SHA256SUMS
      )
      download_verified_asset "$asset_name" "$download_path"
      cmp "$case_dir/$asset_name" "$download_path"
      ;;
    mismatch)
      printf 'trusted release bytes\n' > "$case_dir/$asset_name"
      (
        cd "$case_dir"
        sha256sum "$asset_name" > SHA256SUMS
      )
      printf 'tampered release bytes\n' > "$case_dir/$asset_name"
      if download_verified_asset "$asset_name" "$download_path"; then
        fail "$(basename "$installer") accepted a checksum mismatch"
      fi
      [ ! -e "$download_path" ] ||
        fail "$(basename "$installer") retained a mismatched download"
      ;;
    missing-manifest)
      printf 'unverified release bytes\n' > "$case_dir/$asset_name"
      if download_verified_asset "$asset_name" "$download_path"; then
        fail "$(basename "$installer") accepted a missing manifest"
      fi
      [ ! -e "$download_path" ] ||
        fail "$(basename "$installer") downloaded before finding a manifest"
      ;;
    fallback-manifest)
      printf 'trusted release bytes\n' > "$case_dir/$asset_name"
      (
        cd "$case_dir"
        sha256sum "$asset_name" > "tauri-app-${TAG}-SHA256SUMS"
      )
      download_verified_asset "$asset_name" "$download_path"
      cmp "$case_dir/$asset_name" "$download_path"
      ;;
    missing-entry)
      printf 'release bytes\n' > "$case_dir/$asset_name"
      printf '%064d  another-asset\n' 0 > "$case_dir/SHA256SUMS"
      if download_verified_asset "$asset_name" "$download_path"; then
        fail "$(basename "$installer") accepted a missing checksum entry"
      fi
      [ ! -e "$download_path" ] ||
        fail "$(basename "$installer") downloaded without a checksum entry"
      ;;
    *)
      fail "unknown test case: $case_name"
      ;;
  esac
)

test_root="$(mktemp -d)"
trap 'rm -rf "$test_root"' EXIT

for installer in "${installers[@]}"; do
  for case_name in \
    success \
    mismatch \
    missing-manifest \
    fallback-manifest \
    missing-entry
  do
    case_dir="$test_root/$(basename "$installer")-$case_name"
    mkdir -p "$case_dir"
    run_case "$installer" "$case_name" "$case_dir"
  done
done

echo "Release integrity tests passed for both public installers."
