#!/bin/sh
# Resolves every relative link and #anchor in AGENTS.md and docs/**/*.md.
#
#   scripts/check-docs.sh              check the repository
#   scripts/check-docs.sh --root DIR   check another tree
#
# A link is `[text](target)` or a `[label]: target` line, outside code fences
# and inline code. http(s) and mailto targets are not fetched. A target is a
# path from the linking file's directory (a file or a directory that exists),
# an `#anchor`, or both; an anchor in a Markdown file must match one of its
# headings by GitHub's rules: lowercase, punctuation dropped, spaces to `-`,
# and `-1`, `-2` for a heading the file already has. A path that leaves the
# repository (docs/REFERENCES.md's sibling checkouts) is not checked or counted.
# Prints each broken link
# as `file:line target` and exits 1 if there is any. POSIX sh and awk only.
set -eu

root=.
while [ $# -gt 0 ]; do
  case "$1" in
    --root) root=$2; shift 2 ;;
    *) echo "usage: scripts/check-docs.sh [--root DIR]" >&2; exit 2 ;;
  esac
done
cd "$root"
export LC_ALL=C

# The anchors a Markdown file's headings give, one a line.
slugs() {
  awk '
    /^[ ]*(```|~~~)/ { fence = !fence; next }
    fence { next }
    /^ {0,3}#+[ \t]/ {
      s = $0
      sub(/^ *#+[ \t]+/, "", s)
      sub(/[ \t]+#+[ \t]*$/, "", s)
      while (match(s, /\]\([^)]*\)/)) s = substr(s, 1, RSTART - 1) substr(s, RSTART + RLENGTH)
      gsub(/<[^>]*>/, "", s)
      s = tolower(s)
      # Punctuation and symbols outside ASCII (Latin-1 signs, × ÷, dashes,
      # arrows, math) go as ASCII punctuation does; letters stay.
      gsub(/\302[\200-\277]|\303\227|\303\267|\342[\200-\257][\200-\277]/, "", s)
      gsub(/[^a-z0-9 _\200-\377-]/, "", s)
      gsub(/ /, "-", s)
      n = seen[s]++
      print (n ? s "-" n : s)
    }
  ' "$1"
}

# `file:line target` for each link in a file, code left out.
links() {
  awk '
    /^[ ]*(```|~~~)/ { fence = !fence; next }
    fence { next }
    {
      s = $0
      gsub(/`[^`]*`/, "", s)
      if (match(s, /^ {0,3}\[[^]]+\]: +[^ ]+/)) {
        t = substr(s, RSTART, RLENGTH); sub(/^.*\]: +/, "", t)
        print FILENAME ":" NR " " t
      }
      while (match(s, /\]\([^) ]+/)) {
        print FILENAME ":" NR " " substr(s, RSTART + 2, RLENGTH - 2)
        s = substr(s, RSTART + RLENGTH)
      }
    }
  ' "$1"
}

# Whether a path from the root climbs out of it with its `..` parts.
leaves_repo() {
  echo "$1" | awk -F/ '
    { depth = 0
      for (i = 1; i <= NF; i++) {
        if ($i == "..") depth--
        else if ($i != "." && $i != "") depth++
        if (depth < 0) exit 0 }
      exit 1 }'
}

files=$(ls AGENTS.md; find docs -name '*.md' | sort)
checked=0
broken=0
list=$(mktemp)
trap 'rm -f "$list"' EXIT

for f in $files; do
  links "$f" >> "$list"
done

while read -r where target; do
  case "$target" in
    http://*|https://*|mailto:*) continue ;;
  esac
  file=${where%%:*}
  path=${target%%#*}
  if [ -n "$path" ] && leaves_repo "$(dirname "$file")/$path"; then
    continue
  fi
  checked=$((checked + 1))
  anchor=
  case "$target" in *'#'*) anchor=${target#*#} ;; esac
  if [ -z "$path" ]; then
    dest=$file
  else
    dest=$(dirname "$file")/$path
    if [ ! -e "$dest" ]; then
      echo "$where $target"; broken=$((broken + 1)); continue
    fi
  fi
  case "$dest" in
    *.md) [ -f "$dest" ] || continue ;;
    *) continue ;;
  esac
  if [ -n "$anchor" ] && ! slugs "$dest" | grep -qxF -- "$anchor"; then
    echo "$where $target"; broken=$((broken + 1))
  fi
done < "$list"

echo "check-docs: $checked links checked, $broken broken" >&2
[ "$broken" -eq 0 ]
