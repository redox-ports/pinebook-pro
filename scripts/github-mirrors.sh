#!/usr/bin/env bash
# Point the Redox build tree at the official GitHub mirrors instead of
# gitlab.redox-os.org.
#
# The Redox GitLab instance is often slow or unreachable, which stalls
# `repo cook` mid-build (it auto-updates recipe sources via `git fetch`).
# github.com/redox-os/* are official mirrors sharing the same commit SHAs,
# so switching remotes does not fork anything and keeps the "clone upstream,
# inject our files" model intact.
#
# Repositories without a GitHub mirror (base, ca-certificates, ...) stay on
# GitLab: their clones already exist in the build tree and rarely change.
#
# Idempotent: safe to run repeatedly (bootstrap-redox.sh calls it).
#
# Usage: scripts/github-mirrors.sh [--dir DIR]

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REDOX_DIR="${PBP_REDOX_DIR:-$REPO_ROOT/build/redox}"

die() {
    echo "error: $*" >&2
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        --dir) REDOX_DIR="${2:?--dir needs a value}"; shift ;;
        -h|--help) echo "usage: scripts/github-mirrors.sh [--dir DIR]"; exit 0 ;;
        *) die "unknown argument: $1" ;;
    esac
    shift
done

[ -d "$REDOX_DIR/recipes" ] || die "no recipes dir at $REDOX_DIR (run scripts/bootstrap-redox.sh first)"

# Repositories verified to exist as GitHub mirrors (verify new entries with:
# git ls-remote --heads https://github.com/redox-os/<repo>.git master).
MIRRORED="bootloader coreutils extrautils installer ion kernel netdb netutils pkgutils redoxfs relibc userutils"

changed=0

# 1) Recipe manifests.
for toml in "$REDOX_DIR"/recipes/*/recipe.toml "$REDOX_DIR"/recipes/*/*/recipe.toml; do
    [ -f "$toml" ] || continue
    for repo in $MIRRORED; do
        if grep -q "gitlab.redox-os.org/redox-os/$repo" "$toml"; then
            sed -i "s|https://gitlab.redox-os.org/redox-os/$repo|https://github.com/redox-os/$repo|g" "$toml"
            echo "recipe: $toml ($repo -> github)"
            changed=$((changed + 1))
        fi
    done
done

# 2) Existing source clones (keep their history, just retarget origin).
for src in "$REDOX_DIR"/recipes/*/source "$REDOX_DIR"/recipes/*/*/source; do
    [ -d "$src/.git" ] || continue
    url="$(git -C "$src" remote get-url origin 2>/dev/null || true)"
    case "$url" in
        https://gitlab.redox-os.org/redox-os/*)
            repo="${url#https://gitlab.redox-os.org/redox-os/}"
            repo="${repo%.git}"
            case " $MIRRORED " in
                *" $repo "*)
                    git -C "$src" remote set-url origin "https://github.com/redox-os/$repo.git"
                    echo "clone:  $src ($repo -> github)"
                    changed=$((changed + 1))
                    ;;
            esac
            ;;
    esac
done

echo "github-mirrors: $changed change(s); base and ca-certificates stay on GitLab (no mirror)"
