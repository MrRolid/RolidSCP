# Sourced by the packaging scripts: the same commit gives the same packages,
# byte for byte, wherever and whenever it is built (with the same toolchain
# and the same system packages; see "Reproducible packages" in README.md).
#
# - Every time stamp is the commit's time (SOURCE_DATE_EPOCH, honoured by
#   dpkg-deb, rpmbuild with the macros in build-rpm.sh, and mksquashfs).
# - The paths of the source folder and of the cargo folder are not built into
#   the binary (panic messages carry source paths): they become /rolidscp and
#   /cargo.
# - Fixed file modes, sort order and time zone.
if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
    SOURCE_DATE_EPOCH=$(git log -1 --format=%ct 2>/dev/null || echo 0)
fi
export SOURCE_DATE_EPOCH
export TZ=UTC LC_ALL=C
umask 022
# Separated by 0x1f, so folders with spaces stay whole.
CARGO_ENCODED_RUSTFLAGS="--remap-path-prefix=$(pwd)=/rolidscp"$'\x1f'"--remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo"
export CARGO_ENCODED_RUSTFLAGS

# Set the time of everything under a folder (links themselves too) to the commit's.
clamp_times() {
    find "$1" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +
}
