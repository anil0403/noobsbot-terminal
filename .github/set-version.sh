#!/usr/bin/env bash
# Sets the version (0.6.1, or a beta like 0.6.1-beta.2) in both crates'
# Cargo.toml and Cargo.lock. Works with macOS's and Linux's tools alike.
set -euo pipefail
v=${1#v}
[[ $v =~ ^[0-9]+\.[0-9]+\.[0-9]+(-beta\.[0-9]+)?$ ]] || { echo "not a version: $1" >&2; exit 1; }
cd "$(dirname "$0")/.."
for d in . macos; do
    V=$v perl -0pi -e 's/^version = ".*?"/version = "$ENV{V}"/m' "$d/Cargo.toml"
    V=$v perl -0pi -e 's/(\nname = "noobsbot"\nversion = )".*?"/$1"$ENV{V}"/' "$d/Cargo.lock"
done
