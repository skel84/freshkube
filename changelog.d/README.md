# Changelog fragments

Each PR adds its release-note bullet in a new file here instead of editing
`CHANGELOG.md`'s **Unreleased** section. Give the file a unique, descriptive slug
(an issue number helps), for example `other-changes/123-pod-status.md`:

```markdown
- **Pod status:** the detail pane keeps the last known status while reconnecting.
```

A fragment is exactly one `- ` bullet. It can wrap onto following lines indented
by two spaces, but has no headings, extra paragraphs or nested lists. Use UTF-8
and lowercase letters, digits and hyphens in directory and file names. Use links
relative to the repository root, where the collected changelog lives.

Choose `coroot` for Coroot changes and `other-changes` for other work. Each
section directory has a plain `title` file containing its exact changelog
heading, without `### `:

```text
Other changes
```

For a new heading, add a new directory and its one-line `title` file. A directory
without a title is an unknown section and fails validation. Section titles must
be unique. Keep only the title and fragment `.md` files in each directory.

Before pushing:

```sh
scripts/changelog.sh check
```

CI runs this check and `scripts/changelog.test.sh` in a small Ubuntu job, even
for Markdown-only changes. Neither command changes the changelog or fragments.

For a release, before renaming **Unreleased** to the version heading:

```sh
scripts/changelog.sh collect
```

Collection validates every fragment first, then appends each section's bullets
in filename order to the **end** of that section under `## Unreleased`. It adds
missing sections at the end of Unreleased in directory-name order. Existing
entries keep their text and order; earlier releases are untouched. Once the
changelog is written, it removes the collected fragments, leaving the section
titles and this README. Commit the updated changelog and deleted fragments in
the release PR. Running collect again with no fragments changes nothing.

`scripts/changelog.sh check --empty` fails if any fragments remain. The release
workflow runs it before promoting bundles, so a tag cannot silently omit notes.
See [the release steps](../docs/MACOS_PACKAGING.md#releases).
