#!/usr/bin/env bash
# Exercise the fragment commands in disposable repository fixtures.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
tree="$work/tree with spaces"
failures=0

fixture() {
  rm -rf "$tree"
  mkdir -p "$tree/scripts" "$tree/changelog.d/coroot" "$tree/changelog.d/other-changes"
  cp "$here/changelog.sh" "$tree/scripts/"
  printf '%s\n' 'Fragment instructions' >"$tree/changelog.d/README.md"
  printf '%s\n' 'Coroot' >"$tree/changelog.d/coroot/title"
  printf '%s\n' 'Other changes' >"$tree/changelog.d/other-changes/title"
  cat >"$tree/CHANGELOG.md" <<'EOF'
# Changelog

## Unreleased

### Coroot

- Existing Coroot entry.

### Other changes

- Existing other entry.

## 1.0.0 (2026-01-01)

### Coroot

- Historical entry.
EOF
  cp "$tree/CHANGELOG.md" "$work/original.md"
}

# expect <exit> <description> <output substring> <arguments...>
expect() {
  local want="$1" what="$2" pattern="$3" got=0 out
  shift 3
  out="$("$tree/scripts/changelog.sh" "$@" 2>&1)" || got=$?
  if [ "$got" != "$want" ] || { [ -n "$pattern" ] && ! grep -qF -- "$pattern" <<<"$out"; }; then
    echo "FAIL: $what (exit $got, wanted $want, output containing '$pattern')"
    sed 's/^/    /' <<<"$out"
    failures=$((failures + 1))
  else
    echo "ok: $what"
  fi
}

verify() {
  local what="$1"
  shift
  if "$@"; then
    echo "ok: $what"
  else
    echo "FAIL: $what"
    failures=$((failures + 1))
  fi
}

fixture
expect 0 'an empty tree passes' 'checked 0' check
expect 0 'an empty tree passes the release guard' 'checked 0' check --empty
expect 0 'an empty collection succeeds' 'collected 0' collect
verify 'an empty collection changes no bytes' cmp -s "$work/original.md" "$tree/CHANGELOG.md"

fixture
printf '%s\n' '- **Zebra:** last in its section.' >"$tree/changelog.d/coroot/zebra.md"
printf '%s\n' '- **Alpha:** first in its section,' '  `wrapped text` stays in the same bullet.' >"$tree/changelog.d/coroot/alpha.md"
printf '%s\n' '- **Other:** follows the existing entry.' >"$tree/changelog.d/other-changes/other.md"
mkdir "$tree/changelog.d/added"
printf '%s\n' 'Added section' >"$tree/changelog.d/added/title"
printf '%s\n' '- **New:** creates a section at the end of Unreleased.' >"$tree/changelog.d/added/new.md"
chmod 640 "$tree/CHANGELOG.md"
expect 0 'single and wrapped bullets pass' 'checked 4' check
verify 'check is read-only' cmp -s "$work/original.md" "$tree/CHANGELOG.md"
expect 1 'the tag guard rejects pending notes' 'collect before tagging' check --empty
expect 0 'collect consumes the validated fragments' 'collected 4' collect
cat >"$work/expected.md" <<'EOF'
# Changelog

## Unreleased

### Coroot

- Existing Coroot entry.
- **Alpha:** first in its section,
  `wrapped text` stays in the same bullet.
- **Zebra:** last in its section.

### Other changes

- Existing other entry.
- **Other:** follows the existing entry.

### Added section

- **New:** creates a section at the end of Unreleased.

## 1.0.0 (2026-01-01)

### Coroot

- Historical entry.
EOF
verify 'append by section and filename, preserving history' cmp -s "$work/expected.md" "$tree/CHANGELOG.md"
verify 'only fragments are deleted and file mode survives' python3 - "$tree" <<'PY'
from pathlib import Path
import sys
tree = Path(sys.argv[1])
remaining = sorted(str(p.relative_to(tree / "changelog.d")) for p in (tree / "changelog.d").rglob("*") if p.is_file())
assert remaining == ["README.md", "added/title", "coroot/title", "other-changes/title"], remaining
assert tree.joinpath("CHANGELOG.md").stat().st_mode & 0o777 == 0o640
PY
expect 0 'collected notes pass the tag guard' 'checked 0' check --empty
expect 0 'collect can run twice' 'collected 0' collect
verify 'a second collect is byte-for-byte identical' cmp -s "$work/expected.md" "$tree/CHANGELOG.md"

# Every invalid fragment must fail before any valid fragment is consumed.
for kind in empty text two-bullets nested-list ordered-list heading paragraph fence; do
  fixture
  printf '%s\n' '- A valid earlier fragment.' >"$tree/changelog.d/coroot/aaa-valid.md"
  invalid="$tree/changelog.d/coroot/zzz-invalid.md"
  case "$kind" in
    empty) : >"$invalid" ;;
    text) printf '%s\n' 'Not a bullet.' >"$invalid" ;;
    two-bullets) printf '%s\n' '- First.' '- Second.' >"$invalid" ;;
    nested-list) printf '%s\n' '- First.' '  - Nested.' >"$invalid" ;;
    ordered-list) printf '%s\n' '- First.' '  1. Nested.' >"$invalid" ;;
    heading) printf '%s\n' '- First.' '  ### Heading' >"$invalid" ;;
    paragraph) printf '%s\n' '- First.' '' '  Another paragraph.' >"$invalid" ;;
    fence) printf '%s\n' '- First.' '  ```text' '  code' '  ```' >"$invalid" ;;
  esac
  expect 1 "$kind fails check" 'expected one bullet' check
  expect 1 "$kind fails collect before any writes" 'expected one bullet' collect
  verify "$kind leaves the changelog unchanged" cmp -s "$work/original.md" "$tree/CHANGELOG.md"
  verify "$kind leaves all fragments in place" test -f "$tree/changelog.d/coroot/aaa-valid.md" -a -f "$invalid"
done

fixture
mkdir "$tree/changelog.d/unknown"
printf '%s\n' '- Unknown section.' >"$tree/changelog.d/unknown/note.md"
expect 1 'a directory without a title is unknown' 'unknown section' check
expect 1 'an unknown section cannot be collected' 'unknown section' collect
verify 'unknown sections leave existing entries alone' cmp -s "$work/original.md" "$tree/CHANGELOG.md"
printf '%s\n' 'Coroot' >"$tree/changelog.d/unknown/title"
expect 1 'two directories cannot name the same section' 'duplicate section title' check
printf '%s\n' 'One' 'Two' >"$tree/changelog.d/unknown/title"
expect 1 'a title must be one line' 'expected one heading title' check

fixture
printf '%s\n' '- Outside target.' >"$work/outside.md"
ln -s "$work/outside.md" "$tree/changelog.d/coroot/link.md"
expect 1 'a fragment cannot follow a symlink' 'expected a regular file' collect
verify 'the symlink target remains' test -f "$work/outside.md"

fixture
printf '%s\n' '- Pending note.' >"$tree/changelog.d/coroot/note.md"
printf '%s\n' '## 1.0.0' >"$tree/CHANGELOG.md"
expect 1 'collect requires Unreleased' "exactly one '## Unreleased'" collect
printf '%s\n' '## Unreleased' '## Unreleased' >"$tree/CHANGELOG.md"
expect 1 'duplicate Unreleased headings are ambiguous' "exactly one '## Unreleased'" collect
printf '%s\n' '## Unreleased' '### Coroot' '### Coroot' >"$tree/CHANGELOG.md"
expect 1 'duplicate section headings are ambiguous' 'duplicate section heading' collect
verify 'ambiguous headings leave notes for correction' test -f "$tree/changelog.d/coroot/note.md"

fixture
printf '%s' '## Unreleased' >"$tree/CHANGELOG.md"
printf '%s\n' '- First new entry.' >"$tree/changelog.d/coroot/note.md"
expect 0 'a new section works at EOF without a final newline' 'collected 1' collect
printf '%s\n' '## Unreleased' '' '### Coroot' '' '- First new entry.' >"$work/expected.md"
verify 'the first section has valid Markdown spacing' cmp -s "$work/expected.md" "$tree/CHANGELOG.md"

fixture
printf '%s\n' '## Unreleased' '' '### Coroot' '' '## 1.0.0' >"$tree/CHANGELOG.md"
printf '%s\n' '- First entry in an empty section.' >"$tree/changelog.d/coroot/note.md"
expect 0 'an empty existing section accepts notes' 'collected 1' collect
printf '%s\n' '## Unreleased' '' '### Coroot' '' '- First entry in an empty section.' '' '## 1.0.0' >"$work/expected.md"
verify 'an empty section retains its separator' cmp -s "$work/expected.md" "$tree/CHANGELOG.md"

if [ "$failures" -gt 0 ]; then
  echo "changelog.test: $failures failed"
  exit 1
fi
echo 'changelog.test: all passed'
