#!/bin/sh
# Fail when any version field in server.json, or the OCI identifier's tag,
# differs from Cargo.toml's [package] version. With --tag, also require the
# release tag to match. npm/package.json is checked only when it exists.
#
# Usage:
#   scripts/check-version-sync.sh [--tag vX.Y.Z] [cargo_toml] [server_json] [npm_package_json]
set -eu

root="$(cd "$(dirname "$0")/.." && pwd)"

tag=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --tag) tag="${2:?--tag needs a value}"; shift 2;;
    --tag=*) tag="${1#*=}"; shift;;
    -h|--help)
      echo "usage: check-version-sync.sh [--tag vX.Y.Z] [cargo_toml] [server_json] [npm_package_json]"
      exit 0
      ;;
    --) shift; break;;
    -*) echo "check-version-sync: unknown argument: $1" >&2; exit 2;;
    *) break;;
  esac
done

cargo_toml="${1:-$root/Cargo.toml}"; [ "$#" -gt 0 ] && shift
server_json="${1:-$root/server.json}"; [ "$#" -gt 0 ] && shift
# The launcher manifest defaults to npm/ beside the Cargo.toml being checked.
npm_package_json="${1:-$(dirname "$cargo_toml")/npm/package.json}"

# First `version = "..."` inside the [package] table.
cargo_version="$(awk '
  /^\[/ { in_pkg = ($0 == "[package]") }
  in_pkg && /^version[[:space:]]*=/ { gsub(/.*=[[:space:]]*"|".*/, ""); print; exit }
' "$cargo_toml")"
[ -n "$cargo_version" ] || { echo "could not read version from $cargo_toml" >&2; exit 2; }

status=0

if [ -n "$tag" ]; then
  printf '%s' "$tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$' \
    || { echo "release tag must look like vMAJOR.MINOR.PATCH: $tag" >&2; status=1; }
  [ "${tag#v}" = "$cargo_version" ] \
    || { echo "tag $tag does not match Cargo.toml version $cargo_version" >&2; status=1; }
fi

# Top-level version and every non-OCI packages[].version; a missing field is a
# failure. The MCP registry rejects a `version` on OCI packages: their version
# is the identifier's tag, checked below.
versions="$(jq -r '([.version] + [.packages[]? | select(.registryType != "oci") | .version]) | map(if . == null then "<missing>" else . end) | .[]' "$server_json")"
[ -n "$versions" ] || { echo "no version fields found in $server_json" >&2; exit 2; }
# OCI package identifiers carry the tag (registry/name:tag); it must match too.
tags="$(jq -r '.packages[]? | select(.registryType == "oci") | (.identifier | split(":") | if length > 1 then .[-1] else "<missing>" end)' "$server_json")"

oci_with_version="$(jq -r '[.packages[]? | select(.registryType == "oci" and has("version"))] | length' "$server_json")"
if [ "$oci_with_version" != 0 ]; then
  echo "server.json OCI packages must not have a version field (the MCP registry rejects it); put the version in the identifier tag" >&2
  status=1
fi
for v in $versions $tags; do
  if [ "$v" != "$cargo_version" ]; then
    echo "server.json version $v != Cargo.toml version $cargo_version" >&2
    status=1
  fi
done

# npm/package.json is only checked when it is present (the npm launcher is not
# part of the release yet).
if [ -f "$npm_package_json" ]; then
  npm_version="$(jq -r '.version // "<missing>"' "$npm_package_json")"
  if [ "$npm_version" != "$cargo_version" ]; then
    echo "npm/package.json version $npm_version != Cargo.toml version $cargo_version" >&2
    status=1
  fi
fi

[ "$status" -eq 0 ] && echo "version sync ok: $cargo_version"
exit "$status"
