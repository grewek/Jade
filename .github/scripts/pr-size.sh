#!/usr/bin/env bash
# Counts the changed lines of production code between the merge base of <base>
# and <head>, and warns above the limit (CLAUDE.md, "Reviewable steps"). Never
# fails because of the size.
#
# Production code is crates/*/src/**.rs and build.rs, without test files and
# top-level `#[cfg(test)]` modules. Added and removed lines both count; a moved
# file counts only what changed in it.
#
# Usage: .github/scripts/pr-size.sh <base> [head]
set -euo pipefail

limit=500
base=${1:?usage: pr-size.sh <base> [head]}
head=${2:-HEAD}
merge_base=$(git merge-base "$base" "$head")

is_production() {
  [[ $1 =~ ^crates/[^/]+/(src/.*|build)\.rs$ ]] &&
    ! [[ $1 =~ /(tests/|(tests|[^/]*_tests|test_[^/]*)\.rs$) ]]
}

# "first-last" of each top-level test module of <commit>:<path>.
test_spans() {
  git show "$1:$2" 2>/dev/null | awk '
    inside { if (/^}/) { printf "%d-%d ", first, NR; inside = 0 }; next }
    cfg && /^mod [A-Za-z0-9_]+ \{/ { first = NR - 1; inside = 1; cfg = 0; next }
    { cfg = /^#\[cfg\(test\)\]/ }'
}

total=0
# --name-status -z: a status, then one path, or two for a rename or copy.
while IFS= read -r -d '' status; do
  IFS= read -r -d '' old
  new=$old
  if [[ $status == [RC]* ]]; then IFS= read -r -d '' new; fi
  is_production "$new" || is_production "$old" || continue
  paths=("$old")
  [[ $new == "$old" ]] || paths+=("$new")
  count=$(git diff -M -U0 "$merge_base" "$head" -- "${paths[@]}" |
    awk -v old_spans="$(test_spans "$merge_base" "$old")" \
      -v new_spans="$(test_spans "$head" "$new")" '
      function tested(line, spans,   n, i, s, r) {
        n = split(spans, s, " ")
        for (i = 1; i <= n; i++) {
          split(s[i], r, "-")
          if (line >= r[1] + 0 && line <= r[2] + 0) return 1
        }
        return 0
      }
      /^@@/ {
        split(substr($2, 2), a, ","); split(substr($3, 2), b, ",")
        removed = a[1]; added = b[1]; hunk = 1; next
      }
      hunk && /^-/ { if (!tested(removed++, old_spans)) count++ }
      hunk && /^\+/ { if (!tested(added++, new_spans)) count++ }
      END { print count + 0 }')
  if ((count > 0)); then
    printf '%6d  %s\n' "$count" "$new"
    total=$((total + count))
  fi
done < <(git diff -M --name-status -z "$merge_base" "$head")

echo "Changed lines of production code: $total (limit about $limit)"
if ((total > limit)); then
  echo "::warning title=Large pull request::$total changed lines of production code, over the limit of about $limit. Cut it into reviewable steps."
fi
