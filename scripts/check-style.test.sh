#!/usr/bin/env bash
# Runs scripts/check-style.sh on small broken samples and checks its verdicts.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
failures=0

tree() {
  rm -rf "$work/tree"
  mkdir -p "$work/tree/scripts" "$work/tree/crates/freshkube-desktop/src/logs" \
    "$work/tree/crates/freshkube-ui/src" "$work/tree/crates/freshkube-logs/src" \
    "$work/tree/crates/freshkube-terminal/src"
  cp "$here/check-style.sh" "$work/tree/scripts/"
  : >"$work/tree/scripts/style-allowlist.txt"
}

page() {
  cat >"$work/tree/crates/freshkube-desktop/src/$1"
}

# expect <0|1> <description> [pattern the output must contain]
expect() {
  local want="$1" what="$2" pattern="${3:-}" got=0 out
  out="$("$work/tree/scripts/check-style.sh" --root "$work/tree" 2>&1)" || got=$?
  if [ "$got" != "$want" ] || { [ -n "$pattern" ] && ! grep -qF -- "$pattern" <<<"$out"; }; then
    echo "FAIL: $what (exit $got, wanted $want${pattern:+, output containing \"$pattern\"})"
    sed 's/^/    /' <<<"$out"
    failures=$((failures + 1))
  else
    echo "ok: $what"
  fi
}

tree
page clean.rs <<'EOF'
fn card() -> Div {
    div().rounded(px(12.)).text_size(dp(12.5)).child(ui::page_title("Pods"))
        .child(ui::status_glyph(ui::Tone::Good, cx))
}
EOF
expect 0 "a page on the shared components passes"

tree
page broken.rs <<'EOF'
fn page() -> Div {
    let rows = uniform_list("rows", 10, |_, _, _| Vec::new());
    let card = div()
        .rounded(px(6.))
        .child(div().text_size(dp(26.)).child("Nodes"));
    let dot = div()
        .size(dp(8.))
        .rounded_full()
        .bg(p.good);
    let legend = "● healthy";
    div().child(rows).child(card).child(dot).child(legend)
}
EOF
expect 1 "an unlisted page running its own list fails" "list crates/freshkube-desktop/src/broken.rs:2:"
expect 1 "an off-scale radius fails" "radius crates/freshkube-desktop/src/broken.rs:4:"
expect 1 "a literal title size fails" "title crates/freshkube-desktop/src/broken.rs:5:"
expect 1 "a hand-drawn dot fails" "glyph crates/freshkube-desktop/src/broken.rs:7:"
expect 1 "a glyph in a string fails" "glyph crates/freshkube-desktop/src/broken.rs:10:"

printf '%s\n' "# not yet migrated" \
  "list crates/freshkube-desktop/src/broken.rs" \
  "radius crates/freshkube-desktop/src/broken.rs" \
  "title crates/freshkube-desktop/src/broken.rs" \
  "glyph crates/freshkube-desktop/src/broken.rs" >"$work/tree/scripts/style-allowlist.txt"
expect 0 "an allowlisted page passes"

grep -v '^title' "$work/tree/scripts/style-allowlist.txt" >"$work/tree/scripts/allow.tmp"
mv "$work/tree/scripts/allow.tmp" "$work/tree/scripts/style-allowlist.txt"
expect 1 "an allowlist entry covers one rule, not the file" "title crates/freshkube-desktop/src/broken.rs:5:"

tree
page fixed.rs <<'EOF'
fn page() -> Div { div().rounded(px(8.)) }
EOF
echo "radius crates/freshkube-desktop/src/fixed.rs" >"$work/tree/scripts/style-allowlist.txt"
expect 1 "a listed file that is clean fails until its entry goes" "radius crates/freshkube-desktop/src/fixed.rs"

tree
cat >"$work/tree/crates/freshkube-logs/src/view.rs" <<'EOF'
fn rows() { let list = VirtualList::new(); uniform_list("log", 1, f); }
EOF
cat >"$work/tree/crates/freshkube-terminal/src/paint.rs" <<'EOF'
fn rows() { uniform_list("grid", 1, f); }
EOF
cat >"$work/tree/crates/freshkube-ui/src/table.rs" <<'EOF'
fn table() { uniform_list("rows", 1, f); div().rounded(px(6.)).text_size(dp(20.)); }
EOF
page tests.rs <<'EOF'
fn sample() { uniform_list("rows", 1, f); }
EOF
expect 0 "the log and terminal crates, freshkube-ui and tests are exempt"

tree
page logs/talos.rs <<'EOF'
fn rows() { uniform_list("services", 1, f); }
EOF
expect 1 "desktop's log sources are pages like any other" "list crates/freshkube-desktop/src/logs/talos.rs:1:"

mkdir -p "$work/tree/crates/freshkube-desktop/src/resources/screen"
page resources/screen/view.rs <<'EOF'
fn table() { uniform_list("rows", 1, f); }
EOF
expect 1 "the Resources screen is a page like any other" "list crates/freshkube-desktop/src/resources/screen/view.rs:1:"

tree
page buttons.rs <<'EOF'
fn controls() -> Div {
    div()
        .child(Button::new("refresh").ghost().icon(IconName::Refresh).tooltip("Refresh pods"))
        .child(Button::new("next").icon(IconName::ChevronDown).tooltip_with_action("Next", &Next, None))
        .child(Button::new("copy").icon(IconName::Copy).label("Copy"))
        .child(Button::new("open").on_click(|_, _, _| open(Button::new("x"))).child(icon()))
        .child(Button::new("plain").label("Plain"))
        .child(
            Button::new("close")
                .small()
                .icon(IconName::X)
                .on_click(cx.listener(|view, _, window, cx| view.close(window, cx))),
        )
}
EOF
expect 1 "an icon-only button without a tooltip fails" "tip crates/freshkube-desktop/src/buttons.rs:9:"
out="$("$work/tree/scripts/check-style.sh" --root "$work/tree" --list)"
if [ "$(grep -c '^tip ' <<<"$out")" = 1 ]; then
  echo "ok: an icon button with a tooltip, label or child passes"
else
  echo "FAIL: an icon button with a tooltip, label or child passes"
  sed 's/^/    /' <<<"$out"
  failures=$((failures + 1))
fi

tree
page scroll.rs <<'EOF'
fn page() -> Div {
    let list = div().id("list").overflow_y_scroll().restrict_scroll_to_axis();
    let pane = div()
        .id("pane")
        .overflow_y_scroll()
        .restrict_scroll_to_axis();
    div().id("page").size_full().overflow_y_scroll().child(list).child(pane)
}
EOF
expect 1 "a vertical scroll that takes sideways movement fails" "scroll crates/freshkube-desktop/src/scroll.rs:7:"
out="$("$work/tree/scripts/check-style.sh" --root "$work/tree" --list)"
if [ "$(grep -c '^scroll ' <<<"$out")" = 1 ]; then
  echo "ok: a vertical scroll restricted to its axis passes"
else
  echo "FAIL: a vertical scroll restricted to its axis passes"
  sed 's/^/    /' <<<"$out"
  failures=$((failures + 1))
fi

tree
mkdir -p "$work/tree/crates/freshkube-ui/src/motion" "$work/tree/crates/freshkube-ui/src/table"
cat >"$work/tree/crates/freshkube-ui/src/motion/mod.rs" <<'EOF'
fn fade(el: Div) -> AnimationElement<Div> { el.with_animation("fade", Animation::new(FADE), f) }
EOF
cat >"$work/tree/crates/freshkube-ui/src/table/flash.rs" <<'EOF'
fn tint(el: Div) -> AnimationElement<Div> { motion::fade_out("tint", el) }
EOF
page spinner.rs <<'EOF'
fn spinner() -> impl IntoElement { motion::pulse("dot", div()) }
EOF
expect 0 "animations through ui::motion pass"

cat >"$work/tree/crates/freshkube-ui/src/table/loading.rs" <<'EOF'
fn bar() -> impl IntoElement {
    div().with_animation("bar", Animation::new(PULSE), f)
}
EOF
page tests.rs <<'EOF'
fn sample() { div().with_spring("slide", spring, f); }
EOF
expect 1 "an animation outside ui::motion fails, in freshkube-ui too" "motion crates/freshkube-ui/src/table/loading.rs:2:"
expect 1 "tests are not exempt from the motion rule" "motion crates/freshkube-desktop/src/tests.rs:1:"

if [ "$failures" -gt 0 ]; then
  echo "check-style.test: $failures failed"
  exit 1
fi
echo "check-style.test: all passed"
