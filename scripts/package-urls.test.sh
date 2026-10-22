#!/usr/bin/env bash
# Checks that the packages register the freshkube:// scheme (#873) without
# building anything: the macOS Info.plist, the Linux desktop entry and the
# Windows registry template.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
failures=0

expect() {
  local what="$1" got="$2" pattern="$3"
  if ! grep -qF -- "$pattern" <<<"$got"; then
    echo "FAIL: $what (wanted \"$pattern\")"
    failures=$((failures + 1))
  fi
}

macos="$(cat "$here/package-macos.py")"
expect "macOS registers the scheme" "$macos" '"CFBundleURLTypes"'
expect "macOS names the scheme once" "$macos" 'URL_SCHEME = "freshkube"'
expect "macOS verifies the scheme" "$macos" 'CFBundleURLTypes must register'

portable="$(python3 -I - "$here/package-portable.py" <<'PY'
import importlib.util, sys
spec = importlib.util.spec_from_file_location("portable", sys.argv[1])
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
print(module.DESKTOP_ENTRY)
print(module.URL_HANDLER_REG.replace("\r\n", "\n"))
PY
)"
expect "Linux passes the URL to the app" "$portable" 'Exec=freshkube %u'
expect "Linux claims the scheme" "$portable" 'MimeType=x-scheme-handler/freshkube;'
expect "Windows registers the scheme for the user" "$portable" '[HKEY_CURRENT_USER\Software\Classes\freshkube]'
expect "Windows is a protocol" "$portable" '"URL Protocol"=""'
expect "Windows passes the URL to the app" "$portable" 'freshkube.exe\" \"%1\""'

if [ "$failures" -ne 0 ]; then
  echo "package-urls: $failures failed" >&2
  exit 1
fi
echo "package-urls: ok"
