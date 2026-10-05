#!/usr/bin/env bash
# Keeps pages on the shared components (docs/DESIGN.md, issue #46).
#
#   scripts/check-style.sh                 check the repository
#   scripts/check-style.sh --list          print every offence, allowlisted or not
#   scripts/check-style.sh --root DIR      check another tree (the script's test)
#
# Outside the shared components, a page may not
#   list    run its own uniform_list or VirtualList (logs, YAML and terminal views excepted);
#   radius  round a corner with rounded*(px(n)) other than DESIGN.md's 3, 8, 10 and 12 px (or 0);
#   title   size text 20 dp or larger with a literal, as only ui::page_title may;
#   glyph   draw a status glyph itself (a ● ◆ ▲ ○ ✓ string, or a small rounded_full dot)
#           instead of ui::status_glyph, ui::status_mark or ui::health_mark;
#   tip     give a button an icon and neither a label, a child nor a tooltip
#           (DESIGN.md "Tooltips": an icon-only control names its action).
#   scroll  scroll vertically without .restrict_scroll_to_axis(): unrestricted, GPUI
#           hands a vertical-only scroll a sideways wheel's movement too, so a
#           sideways swipe over a table inside it also scrolls the page.
#
# scripts/style-allowlist.txt names, per rule, the files that broke it when the
# check arrived. It may only shrink: the check fails when an unlisted file
# breaks a rule, and when a listed file no longer does, so the entry goes.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
list=0
while [ $# -gt 0 ]; do
  case "$1" in
    --root) root="$(cd "$2" && pwd)"; shift 2 ;;
    --list) list=1; shift ;;
    -h|--help) sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "check-style: unknown argument $1" >&2; exit 2 ;;
  esac
done
allowlist="$root/scripts/style-allowlist.txt"
cd "$root"

# Every Rust file a page could draw from. The shared components
# (crates/freshkube-ui), the domain crates and tests are not pages.
files() {
  find crates -path '*/src/*' -name '*.rs' \
    -not -path 'crates/freshkube-ui/*' \
    -not -path 'crates/freshkube-core/*' \
    -not -path 'crates/talos-rs/*' \
    -not -name 'tests.rs' \
    -not -path '*/tests/*' |
    LC_ALL=C sort
}

# Prints "rule file:line: text" for each offence.
offences() {
  files | xargs perl -CSD -0777 -ne '
    my $file = $ARGV;
    my $text = $_;
    my $line_of = sub { 1 + (substr($text, 0, $_[0]) =~ tr/\n//) };
    my $report = sub {
      my ($rule, $at) = @_;
      my $n = $line_of->($at);
      my $line = (split /\n/, $text, -1)[$n - 1];
      $line =~ s/^\s+//;
      print "$rule $file:$n: $line\n";
    };
    # Logs, YAML and the terminal measure rows of their own.
    my $own_rows = $file =~ m{^crates/freshkube-(logs|terminal)/|/resources/pane/yaml\.rs$};
    unless ($own_rows) {
      while ($text =~ /\b(uniform_list\s*\(|VirtualList\b)/g) {
        $report->("list", $-[0]);
      }
    }
    while ($text =~ /\.rounded(?:_[a-z]+)?\(\s*px\(\s*([0-9.]+)\s*\)\s*\)/g) {
      my $r = $1 + 0;
      $report->("radius", $-[0]) unless grep { $r == $_ } (0, 3, 8, 10, 12);
    }
    while ($text =~ /\.text_size\(\s*(?:dp|px)\(\s*([0-9.]+)\s*\)\s*\)/g) {
      $report->("title", $-[0]) if $1 >= 20;
    }
    while ($text =~ /\.text_(?:[2-9])?xl\(\)/g) {
      $report->("title", $-[0]);
    }
    while ($text =~ /"[^"\n]*[\x{25CF}\x{25C6}\x{25B2}\x{25B3}\x{25CB}\x{25C7}\x{2713}\x{2714}\x{2B24}][^"\n]*"/g) {
      $report->("glyph", $-[0]);
    }
    while ($text =~ /\.size\(\s*dp\(\s*([0-9.]+)\s*\)\s*\)\s*\.rounded_full\(\)/g) {
      $report->("glyph", $-[0]) if $1 <= 12;
    }
    while ($text =~ /\.overflow_y_scroll\(\)(?!\s*\.restrict_scroll_to_axis\(\))/g) {
      $report->("scroll", $-[0]);
    }
    # A button and its builder chain run until a comma, semicolon or closing
    # bracket at its own depth. A tooltip added after a wrapper call, as in
    # ui::segment(Button::new(…), …).tooltip(…), is outside that chain.
    while ($text =~ /\bButton::new\(/g) {
      my $start = $-[0];
      my ($at, $depth) = (pos($text), 1);
      while ($at < length $text) {
        my $c = substr($text, $at, 1);
        if ($c eq q(")) {
          $at++;
          while ($at < length $text && substr($text, $at, 1) ne q(")) {
            $at++ if substr($text, $at, 1) eq "\\";
            $at++;
          }
        } elsif ($c =~ /[({\[]/) {
          $depth++;
        } elsif ($c =~ /[)}\]]/) {
          last if --$depth < 0;
        } elsif ($depth == 0 && ($c eq ";" || $c eq ",")) {
          last;
        }
        $at++;
      }
      my $chain = substr($text, $start, $at - $start);
      pos($text) = $start + 1;
      next unless $chain =~ /\.icon\(/;
      $report->("tip", $start) unless $chain =~ /\.(?:label|child|children|tooltip\w*)\(/;
    }
  '
}

found="$(offences)"
if [ "$list" = 1 ]; then
  printf '%s\n' "$found" | sed '/^$/d'
  exit 0
fi

# "rule file" for each file that breaks a rule now, and for each allowed one.
now="$(printf '%s\n' "$found" | sed -n 's/^\([a-z]*\) \([^:]*\):.*/\1 \2/p' | LC_ALL=C sort -u)"
allowed=""
if [ -f "$allowlist" ]; then
  allowed="$(sed -e 's/#.*//' -e 's/[[:space:]]*$//' -e '/^$/d' "$allowlist" | LC_ALL=C sort -u)"
fi

new="$(LC_ALL=C comm -23 <(printf '%s\n' "$now" | sed '/^$/d') <(printf '%s\n' "$allowed" | sed '/^$/d'))"
clean="$(LC_ALL=C comm -13 <(printf '%s\n' "$now" | sed '/^$/d') <(printf '%s\n' "$allowed" | sed '/^$/d'))"

status=0
if [ -n "$new" ]; then
  status=1
  echo "check-style: these files break the shared-component rules (docs/DESIGN.md):"
  while read -r rule file; do
    printf '%s\n' "$found" | grep -F "$rule $file:" | sed 's/^/  /'
  done <<<"$new"
  echo "Use the shared components (ui::page_title, ui::status_glyph, the table) instead,"
  echo "give an icon-only button a tooltip and restrict a vertical scroll to its axis;"
  echo "the allowlist only shrinks, so don't add to it."
fi
if [ -n "$clean" ]; then
  status=1
  echo "check-style: these allowlist entries no longer break their rule; remove them from scripts/style-allowlist.txt:"
  printf '%s\n' "$clean" | sed 's/^/  /'
fi
if [ "$status" = 0 ]; then
  echo "check-style: ok ($(printf '%s\n' "$allowed" | sed '/^$/d' | wc -l | tr -d ' ') allowlisted)"
fi
exit "$status"
