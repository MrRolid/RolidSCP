#!/bin/sh
# Extract the translatable texts from the code into po/rolidscp.pot and merge
# them into every po/<language>.po. Needs only cargo (no gettext tools).
# Run from anywhere; translations of unchanged texts are kept.
set -e
cd "$(dirname "$0")/.."
CARGO="${CARGO:-cargo}"
command -v "$CARGO" >/dev/null 2>&1 || CARGO="$HOME/.cargo/bin/cargo"
"$CARGO" run --quiet -p rolid-po-tool -- update
"$CARGO" run --quiet -p rolid-po-tool -- check
