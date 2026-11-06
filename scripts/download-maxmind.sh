#!/usr/bin/env bash

# Download GeoLite2-City into revscope/ so the backend can embed it at build time.
# A HEAD request checks Last-Modified first; that does not count against
# MaxMind's daily download limit.
#
# Credentials come from the command line or the environment. Command-line
# values override the environment. Nothing is hardcoded.
#
#   MAXMIND_ACCOUNT_ID, MAXMIND_LICENSE_KEY
#   --account-id, --license-key
#   download-maxmind.sh ACCOUNT_ID LICENSE_KEY
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"
mmdb_path="${repo_root}/revscope/GeoLite2-City.mmdb"
maxmind_url="https://download.maxmind.com/geoip/databases/GeoLite2-City/download?suffix=tar.gz"
account_id="${MAXMIND_ACCOUNT_ID:-}"
license_key="${MAXMIND_LICENSE_KEY:-}"
force=0

usage() {
  cat <<'EOF'
Usage: download-maxmind.sh [options] [ACCOUNT_ID LICENSE_KEY]

Download the MaxMind GeoLite2-City database used by the revscope backend.
Skips the download when the local file is already as new as MaxMind's copy.

Credentials (command line overrides environment):
  --account-id ID     or MAXMIND_ACCOUNT_ID, or the first positional argument
  --license-key KEY   or MAXMIND_LICENSE_KEY, or the second positional argument

Options:
  --output PATH       Where to write GeoLite2-City.mmdb
                      (default: revscope/GeoLite2-City.mmdb next to this repo)
  --force             Download even if the local file is up to date
  -h, --help          Show this help
EOF
}

require_value() {
  local flag="$1"
  local value="$2"
  if [[ -z "${value}" ]]; then
    echo "${flag} requires a value" >&2
    exit 1
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --account-id)
      [[ $# -ge 2 ]] || { echo "--account-id requires a value" >&2; exit 1; }
      account_id="$2"
      shift 2
      ;;
    --account-id=*)
      account_id="${1#*=}"
      require_value "--account-id" "${account_id}"
      shift
      ;;
    --license-key)
      [[ $# -ge 2 ]] || { echo "--license-key requires a value" >&2; exit 1; }
      license_key="$2"
      shift 2
      ;;
    --license-key=*)
      license_key="${1#*=}"
      require_value "--license-key" "${license_key}"
      shift
      ;;
    --output|-o)
      [[ $# -ge 2 ]] || { echo "--output requires a value" >&2; exit 1; }
      mmdb_path="$2"
      shift 2
      ;;
    --output=*)
      mmdb_path="${1#*=}"
      require_value "--output" "${mmdb_path}"
      shift
      ;;
    --force)
      force=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      break
      ;;
    -*)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
    *)
      break
      ;;
  esac
done

if [[ $# -ge 1 ]]; then
  account_id="$1"
  shift
fi
if [[ $# -ge 1 ]]; then
  license_key="$1"
  shift
fi
if [[ $# -gt 0 ]]; then
  echo "Unexpected argument: $1" >&2
  usage >&2
  exit 1
fi

if [[ -z "${account_id}" || -z "${license_key}" ]]; then
  echo "MaxMind account ID and license key are required." >&2
  echo "Pass --account-id and --license-key, positional arguments, or set MAXMIND_ACCOUNT_ID and MAXMIND_LICENSE_KEY." >&2
  exit 1
fi

echo "==> Checking MaxMind GeoLite2-City database"
# HEAD does not count against MaxMind's daily download limit.
remote_last_modified="$(
  curl -sSI -L -f \
    -u "${account_id}:${license_key}" \
    "${maxmind_url}" \
    | awk -F': ' 'tolower($1) == "last-modified" { gsub(/\r/, "", $2); print $2 }' \
    | tail -n1
)" || {
  echo "Failed to contact MaxMind. Check the account ID and license key." >&2
  exit 1
}
if [[ -z "${remote_last_modified}" ]]; then
  echo "Failed to read Last-Modified from MaxMind" >&2
  exit 1
fi

needs_update=1
if (( force == 0 )) && [[ -f "${mmdb_path}" ]]; then
  local_epoch="$(stat -c %Y "${mmdb_path}")"
  remote_epoch="$(date -u -d "${remote_last_modified}" +%s)"
  if (( local_epoch >= remote_epoch )); then
    needs_update=0
  fi
fi

if (( needs_update == 0 )); then
  echo "GeoLite2-City.mmdb is up to date (${remote_last_modified})"
  exit 0
fi

echo "==> Downloading GeoLite2-City (${remote_last_modified})"
tmp="$(mktemp -d)"
cleanup() {
  rm -rf "${tmp}"
}
trap cleanup EXIT

if ! curl -sS -L -f \
  -u "${account_id}:${license_key}" \
  -o "${tmp}/GeoLite2-City.tar.gz" \
  "${maxmind_url}"; then
  echo "MaxMind download failed" >&2
  exit 1
fi

tar -xzf "${tmp}/GeoLite2-City.tar.gz" -C "${tmp}"
extracted_mmdb="$(find "${tmp}" -type f -name 'GeoLite2-City.mmdb' -print -quit)"
if [[ -z "${extracted_mmdb}" ]]; then
  echo "GeoLite2-City.mmdb not found in downloaded archive" >&2
  exit 1
fi

mkdir -p "$(dirname "${mmdb_path}")"
mv "${extracted_mmdb}" "${mmdb_path}"
touch -d "${remote_last_modified}" "${mmdb_path}"
echo "Updated ${mmdb_path}"
