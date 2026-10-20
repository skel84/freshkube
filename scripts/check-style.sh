#!/usr/bin/env bash
# Keeps pages on the shared components (docs/DESIGN.md, issue #46).
#
#   scripts/check-style.sh                 check the repository
#   scripts/check-style.sh --list          print every offence, allowlisted or not
#   scripts/check-style.sh --root DIR      check another tree (the script's test)
#   scripts/check-style.sh --pending FILE  read the pending list from FILE (the script's test)
#
# Outside the shared components, a page may not
#   list    run its own uniform_list or VirtualList (logs, YAML and terminal views excepted);
#   radius  round a corner with rounded*(px(n)) other than DESIGN.md's 3, 8, 10 and 12 px (or 0);
#   title   size text 20 dp or larger with a literal: PageHeader draws a page's title;
#   glyph   draw a status glyph itself (a ● ◆ ▲ ○ ✓ string, or a small rounded_full dot)
#           instead of ui::status_glyph, ui::status_mark or ui::health_mark;
#   tip     give a button an icon and neither a label, a child nor a tooltip
#           (DESIGN.md "Tooltips": an icon-only control names its action).
#   scroll  scroll vertically without .restrict_scroll_to_axis(): unrestricted, GPUI
#           hands a vertical-only scroll a sideways wheel's movement too, so a
#           sideways swipe over a table inside it also scrolls the page.
#
# Anywhere outside freshkube-ui's motion module, the shared components and
# tests included, nothing may
#   motion  animate with with_animation(s) or with_spring: animations take their
#           timing from ui::motion's tokens and draw through its helpers.
#
# Outside the domain crates (core, talos-rs) and tests, the shared components
# included, nothing but freshkube-ui's platform module may
#   platform  name the target (target_os, target_family, target_vendor,
#             consts::OS, consts::FAMILY) or put windows in a cfg, cfg! or
#             cfg_attr, on any line: ask freshkube_ui::platform. Three files
#             read the OS themselves and are exempt: motion/system.rs (reduced
#             motion), desktop's stress.rs (process counters) and src/main.rs
#             (a release build's Windows console). Tests spell out each
#             platform's keys, as an independent check of the module. The
#             domain crates (core, talos-rs) and cfg(unix) file modes and
#             paths are the OS's, not the app's look, and stay where they are.
#
# Outside freshkube-ui's form module and tests, the shared components included,
# nothing may
#   label    draw a field label of its own: a fixed text (a literal or a bare
#            name) at 12.5 semibold. form::row draws a field's label.
#   buttons  set out a group's or a dialog's buttons itself: a Button labelled
#            Revert or Discard, or one labelled Cancel that closes a dialog.
#            form::actions puts the pair in the platform's order
#            (DESIGN.md "The Settings page").
# These two arrived with #697 while the Settings code still broke them. The
# pending list below names each such file with the #697 slice that rewrites
# it; a pending file passes its rule, and fails once it no longer breaks it,
# so that slice removes its line. Any other file breaks the rule at once. The
# rules are fully on when the list is empty.
#
# scripts/style-allowlist.txt names, per rule, the files that broke it when the
# check arrived. It may only shrink: the check fails when an unlisted file
# breaks a rule, and when a listed file no longer does, so the entry goes.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
list=0
pending_file=""
while [ $# -gt 0 ]; do
  case "$1" in
    --root) root="$(cd "$2" && pwd)"; shift 2 ;;
    --list) list=1; shift ;;
    --pending) pending_file="$2"; shift 2 ;;
    -h|--help) sed -n '2,54p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
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
    -not -path 'crates/freshkube-delivery/*' \
    -not -path 'crates/talos-rs/*' \
    -not -name 'tests.rs' \
    -not -path '*/tests/*' |
    LC_ALL=C sort
}

# The start of a Perl program that reads one whole file: $report prints
# "rule file:line: text" for an offence at an offset, and $buttons lists each
# Button::new and its builder chain as [offset, chain].
read_file='
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
    # A button and its builder chain run until a comma, semicolon or closing
    # bracket at its own depth. A tooltip added after a wrapper call, as in
    # ui::segment(Button::new(…), …).tooltip(…), is outside that chain.
    my $buttons = sub {
      my @found;
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
        push @found, [$start, substr($text, $start, $at - $start)];
        pos($text) = $start + 1;
      }
      @found;
    };
'

# Prints "rule file:line: text" for each offence.
offences() {
  files | xargs perl -CSD -0777 -ne "$read_file"'
    # Logs and the terminal measure rows of their own.
    my $own_rows = $file =~ m{^crates/freshkube-(logs|terminal)/};
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
    for my $button ($buttons->()) {
      my ($start, $chain) = @$button;
      next unless $chain =~ /\.icon\(/;
      $report->("tip", $start) unless $chain =~ /\.(?:label|child|children|tooltip\w*)\(/;
    }
  '
}

# Every Rust file but the motion module's, for the motion rule.
all_files() {
  find crates -path '*/src/*' -name '*.rs' \
    -not -path 'crates/freshkube-ui/src/motion/*' |
    LC_ALL=C sort
}

motion_offences() {
  all_files | xargs perl -CSD -ne '
    print "motion $ARGV:$.: ", s/^\s+//r if /\.with_(?:animations?|spring)\s*\(/;
    close ARGV if eof;
  '
}

# Every Rust file the platform rule reads.
platform_files() {
  find crates $(test -d src && echo src) \( -path 'src/*' -o -path '*/src/*' -o -name build.rs \) -name '*.rs' \
    -not -path 'crates/freshkube-core/*' \
    -not -path 'crates/freshkube-delivery/*' \
    -not -path 'crates/talos-rs/*' \
    -not -path 'crates/freshkube-ui/src/platform.rs' \
    -not -path 'crates/freshkube-ui/src/motion/system.rs' \
    -not -path 'crates/freshkube-desktop/src/stress.rs' \
    -not -path 'src/main.rs' \
    -not -name 'tests.rs' \
    -not -name '*_tests.rs' \
    -not -path '*/tests/*' |
    LC_ALL=C sort
}

# A line that names the target (target_os, target_family, target_vendor,
# std::env::consts::OS or FAMILY) anywhere, or `windows` inside a cfg, cfg! or
# cfg_attr, which rustfmt may spread over several lines. Comments don't count.
platform_offences() {
  platform_files | xargs perl -CSD -ne '
    BEGIN { $depth = 0 }
    unless (m{^\s*//}) {
      my $hit = /\btarget_(?:os|family|vendor)\b|\bconsts::(?:OS|FAMILY)\b/;
      my $cfg = "";
      if ($depth > 0) { $cfg = $_ }
      elsif (/\bcfg(?:_attr)?!?\s*\(/) { $cfg = substr($_, $-[0]) }
      if ($cfg ne "") {
        $depth += (() = $cfg =~ /\(/g) - (() = $cfg =~ /\)/g);
        $depth = 0 if $depth < 0;
        $hit ||= $cfg =~ /\bwindows\b/;
      }
      print "platform $ARGV:$.: ", s/^\s+//r if $hit;
    }
    if (eof) { close ARGV; $depth = 0 }
  '
}

# Every Rust file the label and buttons rules read: the shared components'
# too, but not the form module, which draws labels and button pairs for
# everyone. The domain crates draw nothing.
form_files() {
  find crates -path '*/src/*' -name '*.rs' \
    -not -path 'crates/freshkube-ui/src/form.rs' \
    -not -path 'crates/freshkube-ui/src/form/*' \
    -not -path 'crates/freshkube-core/*' \
    -not -path 'crates/freshkube-delivery/*' \
    -not -path 'crates/talos-rs/*' \
    -not -name 'tests.rs' \
    -not -name '*_tests.rs' \
    -not -path '*/tests/*' |
    LC_ALL=C sort
}

# A label is 12.5 semibold, in either order, around a fixed text: a string
# literal or a bare name, as a label closure's parameter. Bold text built from
# data (an event's reason) is not a label. A pair's buttons are a Revert or a
# Discard anywhere, and a Cancel whose chain closes a dialog; a Cancel that
# stops a running operation stands alone and passes.
form_offences() {
  form_files | xargs perl -CSD -0777 -ne "$read_file"'
    my $size = qr/\.text_size\(\s*(?:dp|px)\(\s*12\.5\s*\)\s*\)/;
    my $bold = qr/\.font_weight\(\s*FontWeight::SEMIBOLD\s*\)|\.font_semibold\(\)/;
    my $fixed = qr/\.child\(\s*(?:"(?:[^"\\\n]|\\.)*"|[a-z_][a-z0-9_]*)\s*\)/;
    while ($text =~ /(?:$size\s*(?:$bold)|(?:$bold)\s*$size)\s*$fixed/g) {
      $report->("label", $-[0]);
    }
    for my $button ($buttons->()) {
      my ($start, $chain) = @$button;
      my $pair = $chain =~ /\.label\(\s*"(?:Revert|Discard)"\s*\)/
        || ($chain =~ /\.label\(\s*"Cancel"\s*\)/ && $chain =~ /\bclose_dialog\b/);
      $report->("buttons", $start) if $pair;
    }
  '
}

# "rule file slice": the files that broke the label or buttons rule when it
# arrived (#697), each with the slice of #697 that rewrites it. The list only
# shrinks: a slice removes its lines when its files pass, and the check fails
# until it does. Nothing is added to it.
pending_list() {
  if [ -n "$pending_file" ]; then
    cat "$pending_file"
    return
  fi
  cat <<'EOF'
buttons crates/freshkube-desktop/src/desktop/settings/destinations/form.rs 5
buttons crates/freshkube-desktop/src/desktop/settings/destinations/mod.rs 5
label crates/freshkube-desktop/src/desktop/settings/destinations/form.rs 5
label crates/freshkube-monitoring/src/page/settings.rs 9
label crates/freshkube-monitoring/src/page/source.rs 9
EOF
}

found="$(offences; motion_offences; platform_offences; form_offences)"
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

pending="$(pending_list | sed -e 's/#.*//' -e '/^[[:space:]]*$/d')"
waiting="$(printf '%s\n' "$pending" | awk 'NF { print $1, $2 }' | LC_ALL=C sort -u)"
excused="$(printf '%s\n%s\n' "$allowed" "$waiting" | sed '/^$/d' | LC_ALL=C sort -u)"

new="$(LC_ALL=C comm -23 <(printf '%s\n' "$now" | sed '/^$/d') <(printf '%s\n' "$excused" | sed '/^$/d'))"
clean="$(LC_ALL=C comm -13 <(printf '%s\n' "$now" | sed '/^$/d') <(printf '%s\n' "$allowed" | sed '/^$/d'))"
done_slices="$(LC_ALL=C comm -13 <(printf '%s\n' "$now" | sed '/^$/d') <(printf '%s\n' "$waiting" | sed '/^$/d'))"

status=0
if [ -n "$new" ]; then
  status=1
  echo "check-style: these files break the shared-component rules (docs/DESIGN.md):"
  while read -r rule file; do
    printf '%s\n' "$found" | grep -F "$rule $file:" | sed 's/^/  /'
  done <<<"$new"
  echo "Use the shared components (PageHeader, ui::status_glyph, the table) instead,"
  echo "give an icon-only button a tooltip, restrict a vertical scroll to its axis and"
  echo "animate through ui::motion, ask freshkube_ui::platform what differs by platform,"
  echo "label fields with form::row and set out button pairs with form::actions;"
  echo "the allowlist and the pending list only shrink, so don't add to them."
fi
if [ -n "$clean" ]; then
  status=1
  echo "check-style: these allowlist entries no longer break their rule; remove them from scripts/style-allowlist.txt:"
  printf '%s\n' "$clean" | sed 's/^/  /'
fi
if [ -n "$done_slices" ]; then
  status=1
  echo "check-style: these files no longer break their rule; remove their lines from pending_list in scripts/check-style.sh:"
  while read -r rule file; do
    printf '%s\n' "$pending" | awk -v r="$rule" -v f="$file" '$1 == r && $2 == f { print "  " $1 " " $2 " (#697 slice " $3 ")" }'
  done <<<"$done_slices"
fi
if [ "$status" = 0 ]; then
  echo "check-style: ok ($(printf '%s\n' "$allowed" | sed '/^$/d' | wc -l | tr -d ' ') allowlisted, $(printf '%s\n' "$waiting" | sed '/^$/d' | wc -l | tr -d ' ') pending #697)"
fi
exit "$status"
