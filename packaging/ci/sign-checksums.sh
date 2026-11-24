#!/bin/bash
# Sign the checksums of a release: SHA256SUMS -> SHA256SUMS.asc (an armored,
# detached OpenPGP signature).
#
#   sign-checksums.sh SHA256SUMS
#
# The private key comes from the environment (GitHub secrets):
#   RELEASE_GPG_KEY         the armored private key (gpg --armor --export-secret-keys)
#   RELEASE_GPG_PASSPHRASE  its passphrase (empty if it has none)
# The signature is then checked against the public key in the repository
# (RELEASE_KEY_FILE, default docs/release-key.asc), so a secret that does not
# belong to the published key stops the release.
#
# Without the public key in the repository nothing is signed (a warning):
# signing starts with the release after the key was committed. With the key
# there, a missing secret is an error.
set -euo pipefail
SUMS=$1
PUB=${RELEASE_KEY_FILE:-docs/release-key.asc}

if [ ! -f "$PUB" ]; then
  echo "::warning title=Not signed::$PUB is not in the repository: the checksums are not signed"
  exit 0
fi
if [ -z "${RELEASE_GPG_KEY:-}" ]; then
  echo "::error title=Signing key missing::$PUB is in the repository, but the secret RELEASE_GPG_KEY is not set"
  exit 1
fi

work=$(mktemp -d)
cleanup() {
  GNUPGHOME="$work/sign" gpgconf --kill all 2>/dev/null || true
  GNUPGHOME="$work/check" gpgconf --kill all 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT
mkdir -m 700 "$work/sign" "$work/check"

# Sign in a keyring of its own (never the runner's or a user's).
printf '%s\n' "$RELEASE_GPG_KEY" | GNUPGHOME="$work/sign" gpg --batch --quiet --import
fpr=$(GNUPGHOME="$work/sign" gpg --batch --with-colons --list-secret-keys | awk -F: '$1 == "fpr" { print $10; exit }')
[ -n "$fpr" ] || { echo "::error title=Signing key::RELEASE_GPG_KEY holds no private key"; exit 1; }
printf '%s' "${RELEASE_GPG_PASSPHRASE:-}" >"$work/pass"
GNUPGHOME="$work/sign" gpg --batch --yes --quiet --pinentry-mode loopback --passphrase-file "$work/pass" \
  --local-user "$fpr" --armor --detach-sign --output "$SUMS.asc" "$SUMS"
rm -f "$work/pass"

# Check against the published key only.
GNUPGHOME="$work/check" gpg --batch --quiet --import "$PUB"
if ! GNUPGHOME="$work/check" gpg --batch --verify "$SUMS.asc" "$SUMS"; then
  echo "::error title=Signature::the signature does not check against $PUB (the secret holds another key?)"
  rm -f "$SUMS.asc"
  exit 1
fi
echo "signed with $fpr: $SUMS.asc"
