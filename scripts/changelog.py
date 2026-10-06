"""Validate PR fragments or collect them into CHANGELOG.md for a release.

Run through scripts/changelog.sh, which passes the repository root first."""
import os
from pathlib import Path
import re
import sys
import tempfile


def fail(message):
    raise ValueError(message)


def read_file(path):
    if path.is_symlink() or not path.is_file():
        fail(f"{path}: expected a regular file")
    return path.read_bytes().decode("utf-8")


def read_bullet(path):
    lines = read_file(path).rstrip("\r\n").splitlines()
    if not lines or not re.fullmatch(r"- \S.*", lines[0]):
        fail(f"{path}: expected one bullet starting with '- '")
    for line in lines[1:]:
        # A wrapped bullet is fine; a second block or nested list is not.
        if not line.startswith("  ") or not line.strip() or re.match(
            r"(?:[-+*](?:\s|$)|\d+[.)](?:\s|$)|#{1,6}(?:\s|$)|>|```|~~~)", line.lstrip()
        ):
            fail(f"{path}: expected one bullet; indent wrapped text by two spaces")
    return "\n".join(line.rstrip() for line in lines)


def read_fragments(directory):
    if directory.is_symlink() or not directory.is_dir():
        fail(f"{directory}: expected the changelog.d directory")
    sections = []
    titles = set()
    slug = r"[a-z0-9]+(?:-[a-z0-9]+)*"
    for section in sorted(directory.iterdir()):
        if section.name == "README.md":
            read_file(section)
            continue
        if section.is_symlink() or not section.is_dir() or not re.fullmatch(slug, section.name):
            fail(f"{section}: expected a section directory with a title file")
        title_file = section / "title"
        if not title_file.exists():
            fail(f"{section}: unknown section; add a one-line title file")
        raw_title = read_file(title_file)
        title = raw_title.strip()
        if len(raw_title.splitlines()) != 1 or not title or title.startswith("#"):
            fail(f"{title_file}: expected one heading title, without '### '")
        if title in titles:
            fail(f"{title_file}: duplicate section title {title!r}")
        titles.add(title)
        fragments = []
        for path in sorted(section.iterdir()):
            if path.name == "title":
                continue
            if not re.fullmatch(slug + r"\.md", path.name):
                fail(f"{path}: expected a <slug>.md fragment")
            fragments.append((path, read_bullet(path)))
        sections.append((title, fragments))
    return sections


def unreleased_bounds(lines):
    starts = [i for i, line in enumerate(lines) if line.rstrip() == "## Unreleased"]
    if len(starts) != 1:
        fail("CHANGELOG.md: expected exactly one '## Unreleased' heading")
    start = starts[0] + 1
    end = next((i for i in range(start, len(lines)) if lines[i].startswith("## ")), len(lines))
    return start, end


def collect(changelog, sections):
    original = read_file(changelog)
    lines = original.splitlines(keepends=True)
    start, end = unreleased_bounds(lines)
    headings = [line.rstrip() for line in lines[start:end] if line.startswith("### ")]
    if len(headings) != len(set(headings)):
        fail("CHANGELOG.md: duplicate section heading under Unreleased")
    for title, fragments in sections:
        if not fragments:
            continue
        start, end = unreleased_bounds(lines)
        heading = f"### {title}"
        section = next((i for i in range(start, end) if lines[i].rstrip() == heading), None)
        if section is None:
            insert = end
        else:
            insert = next((i for i in range(section + 1, end) if lines[i].startswith("### ")), end)
        # Keep the existing text and the blank separator after the section.
        while insert > start and not lines[insert - 1].strip():
            insert -= 1
        prefix = "" if insert == 0 or lines[insert - 1].endswith("\n") else "\n"
        if section is None:
            prefix += f"\n{heading}\n\n"
        elif insert == section + 1:
            prefix += "\n"
        block = prefix + "\n".join(bullet for _, bullet in fragments) + "\n"
        if insert < len(lines) and lines[insert].strip():
            block += "\n"
        lines[insert:insert] = block.splitlines(keepends=True)
    updated = "".join(lines)
    if updated == original:
        return
    # Replace the changelog before deleting anything. Invalid input or a write
    # failure leaves all fragments available for correction and another try.
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=changelog.parent, prefix=".changelog-", delete=False) as output:
            temporary = Path(output.name)
            output.write(updated.encode("utf-8"))
            os.fchmod(output.fileno(), changelog.stat().st_mode & 0o777)
        os.replace(temporary, changelog)
    finally:
        if temporary is not None and temporary.exists():
            temporary.unlink()
    for _, fragments in sections:
        for path, _ in fragments:
            path.unlink()


def main():
    root = Path(sys.argv[1])
    args = sys.argv[2:]
    if args not in (["check"], ["check", "--empty"], ["collect"]):
        fail("usage: scripts/changelog.sh check [--empty] | collect")
    sections = read_fragments(root / "changelog.d")
    count = sum(len(fragments) for _, fragments in sections)
    if args == ["check", "--empty"] and count:
        fail(f"{count} uncollected fragment(s); run scripts/changelog.sh collect before tagging a release")
    if args == ["collect"] and count:
        collect(root / "CHANGELOG.md", sections)
    verb = "collected" if args == ["collect"] else "checked"
    print(f"changelog: {verb} {count} fragment(s)")


try:
    main()
except (OSError, ValueError) as error:
    sys.exit(f"changelog: {error}")
