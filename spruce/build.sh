#!/bin/bash
set -e

OUTPUT_DIR="${OUTPUT_DIR:-/output}"
CROSS=aarch64-linux-gnu
TARGET=aarch64-unknown-linux-gnu
GLIBC_MAX=2.31

export CARGO_HOME="${CARGO_HOME:-/cache/cargo}"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=${CROSS}-gcc
export CC_aarch64_unknown_linux_gnu=${CROSS}-gcc
export AR_aarch64_unknown_linux_gnu=${CROSS}-ar
export RUSTFLAGS="-L /opt/sdl2/lib"

echo "=== Building zitch ${ZITCH_VERSION} for aarch64 ==="
git config --global --add safe.directory /src
make handheld-stage CARGO_CROSS=cargo

# Everything built here has to load on the oldest glibc among the devices.
echo "=== glibc check ==="
STAGE=target/handheld-stage
for f in "$STAGE/zitch" "$STAGE/libzitch-sdl.so" "$STAGE/gl4es/libGL.so.1"; do
    need=$(${CROSS}-objdump -T "$f" | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1)
    echo "$(basename "$f"): $need"
    test "$(printf '%s\nGLIBC_%s\n' "$need" "$GLIBC_MAX" | sort -V | tail -1)" = "GLIBC_$GLIBC_MAX"
done
# Prebuilt by others; reported, not enforced.
for f in "$STAGE/butler" "$STAGE/love" "$STAGE"/libs/*; do
    need=$(${CROSS}-objdump -T "$f" 2>/dev/null | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1)
    echo "$(basename "$f"): ${need:-static}"
done

# The App folder as it goes on the card.
APP="$OUTPUT_DIR/App/zitch"
rm -rf "$OUTPUT_DIR/App"
mkdir -p "$APP"
cp spruce/app/config.json spruce/app/launch.sh spruce/app/love.gptk spruce/app/zitch.png "$APP/"
cp -a "$STAGE" "$APP/zitch"
echo "=== Done ==="
