#!/usr/bin/env bash
# Apply the Pinebook Pro port's changes to the upstream checkouts.
#
# The port does not fork upstream repositories: everything we add or change
# lives in this repository and is injected into the Redox build tree.
#
# Layout of patches/<repo>/ (repo = the recipe name under recipes/<category>):
#   * a path that does NOT end in .patch is a NEW file, copied verbatim with its
#     directory tree preserved (e.g. a driver that mainline Redox does not have);
#   * "<path>.patch" is a unified diff against the upstream file "<path>" in the
#     same repository, applied with `git apply` from the repository root.
#
# Running the script twice is safe: files that already match are skipped and
# patches that are already applied are reported instead of failing.
#
# Usage:
#   scripts/apply-patches.sh [--check] [--list] [repo...]
#
#   --check  report what would be done and verify every diff applies (or is
#            already applied); touch nothing
#   --list   list the files that belong to each repository and exit
#   repo...  restrict to these recipes (default: every patches/<dir> that maps
#            to an existing source tree)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REDOX_DIR="${PBP_REDOX_DIR:-$REPO_ROOT/build/redox}"
PATCH_DIR="$REPO_ROOT/patches"
CHECK=0
LIST=0
WANTED=()

die() {
    echo "error: $*" >&2
    exit 1
}

usage() {
    sed -n '2,24p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit 0
}

while [ $# -gt 0 ]; do
    case "$1" in
        --check) CHECK=1 ;;
        --list) LIST=1 ;;
        -h|--help) usage ;;
        -*) die "unknown argument: $1 (see --help)" ;;
        *) WANTED+=("$1") ;;
    esac
    shift
done

[ -d "$PATCH_DIR" ] || die "no patches directory: $PATCH_DIR"
[ -d "$REDOX_DIR" ] || die "no Redox tree at $REDOX_DIR (run scripts/bootstrap-redox.sh first)"

# Recipes known to the build tree, in a stable order.
if [ ${#WANTED[@]} -eq 0 ]; then
    for dir in "$PATCH_DIR"/*/; do
        WANTED+=("$(basename "$dir")")
    done
fi

copied=0
applied=0
skipped=0
missing=()
hand=()

for repo in "${WANTED[@]}"; do
    pdir="$PATCH_DIR/$repo"
    # The recipe category is not fixed (core/gui/libs/...), so find the
    # checkout by recipe name instead of assuming recipes/core.
    src=""
    for cand in "$REDOX_DIR"/recipes/*/"$repo"/source; do
        if [ -d "$cand/.git" ] || [ -f "$cand/.git" ]; then
            src="$cand"
            break
        fi
    done
    # Keep a predictable path for the error messages when nothing matched.
    [ -n "$src" ] || src="$REDOX_DIR/recipes/core/$repo/source"

    [ -d "$pdir" ] || die "no such patch set: $pdir"
    if [ ! -d "$src/.git" ] && [ ! -f "$src/.git" ]; then
        # `patches/debug/` is instrumentation that is applied by hand, not part
        # of the injected tree; a real recipe without a checkout simply has not
        # been fetched yet (scripts/apply-patches.sh runs before the build).
        if [ "$repo" = debug ]; then
            hand+=("$repo")
        elif [ ${#WANTED[@]} -eq 1 ]; then
            die "no source checkout for '$repo' at $src"
        else
            missing+=("$repo")
        fi
        continue
    fi

    echo "==> $repo ($src)"
    mapfile -t entries < <(cd "$pdir" && find . -type f -printf '%P\n' | sort)

    if [ "$LIST" -eq 1 ]; then
        for rel in "${entries[@]}"; do
            [ "$rel" = README.md ] && continue
            case "$rel" in
                *.patch) printf '  diff   %s\n' "${rel%.patch}" ;;
                *) printf '  new    %s\n' "$rel" ;;
            esac
        done
        continue
    fi

    for rel in "${entries[@]}"; do
        # A README.md at the root of a patch set documents that set; it is ours
        # and must never be copied into the upstream checkout.
        [ "$rel" = README.md ] && continue
        case "$rel" in
            *.patch)
                target="${rel%.patch}"
                if git -C "$src" apply --check --whitespace=nowarn "$pdir/$rel" 2>/dev/null; then
                    if [ "$CHECK" -eq 1 ]; then
                        printf '  would apply  %s\n' "$target"
                    else
                        git -C "$src" apply --whitespace=nowarn "$pdir/$rel"
                        printf '  applied      %s\n' "$target"
                    fi
                    applied=$((applied + 1))
                elif git -C "$src" apply --reverse --check --whitespace=nowarn \
                    "$pdir/$rel" 2>/dev/null; then
                    printf '  up to date   %s\n' "$target"
                    skipped=$((skipped + 1))
                else
                    die "$repo: $rel does not apply to $src/$target
       (the checkout is at $(git -C "$src" rev-parse --short HEAD); see
        patches/$repo/README.md for the base commit)"
                fi
                ;;
            *)
                dst="$src/$rel"
                if [ -f "$dst" ] && cmp -s "$pdir/$rel" "$dst"; then
                    skipped=$((skipped + 1))
                    continue
                fi
                if [ "$CHECK" -eq 1 ]; then
                    printf '  would copy   %s\n' "$rel"
                else
                    install -D -m "$(stat -c %a "$pdir/$rel")" "$pdir/$rel" "$dst"
                    printf '  copied       %s\n' "$rel"
                fi
                copied=$((copied + 1))
                ;;
        esac
    done
done

if [ ${#hand[@]} -gt 0 ]; then
    echo
    echo "==> not injected (applied by hand): ${hand[*]}"
    echo "    see patches/${hand[*]}/README.md"
fi

if [ ${#missing[@]} -gt 0 ]; then
    echo
    echo "==> no source checkout yet: ${missing[*]}"
    echo "    nothing to do for them; run scripts/bootstrap-redox.sh to fetch the sources"
fi

if [ "$LIST" -eq 1 ]; then
    exit 0
fi

echo
if [ "$CHECK" -eq 1 ]; then
    echo "==> check only: $applied diff(s) applyable, $copied file(s) would be added, $skipped up to date"
else
    echo "==> done: $applied diff(s) applied, $copied file(s) copied, $skipped already up to date"
fi
