#!/bin/bash
set -e

OUTPUT_DIR="${OUTPUT_DIR:-/output}"
CROSS=arm-a30-linux-gnueabihf
TARGET=armv7-unknown-linux-gnueabihf
GLIBC_MAX=2.23
: "${BUTLER_REV:?the butler commit to build}"
LIBC7ZIP_REV=4d582fd820a490efaa3958f3cd5ff3c07398a813
SEVENZIP=7z2603
SEVENZIP_SHA256=9cbde5099c6deb73691b0579063da5827522ccbbcba3f0020fd04e8c8c16c0d4
ARCH_FLAGS="-mcpu=cortex-a7 -mfpu=neon-vfpv4 -mfloat-abi=hard"

export CARGO_HOME="${CARGO_HOME:-/cache/cargo}"
export GOPATH=/cache/go GOCACHE=/cache/go-build
export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=${CROSS}-gcc
export CC_armv7_unknown_linux_gnueabihf=${CROSS}-gcc
export AR_armv7_unknown_linux_gnueabihf=${CROSS}-ar
export RUSTFLAGS="-L /opt/sdl2/lib"

STAGE=/tmp/stage
rm -rf "$STAGE" /tmp/butler /tmp/libc7zip /tmp/7z
mkdir -p "$STAGE"

echo "=== Building zitch ${ZITCH_VERSION} for the A30 ==="
git config --global --add safe.directory /src
cargo build --release --target "$TARGET" --no-default-features --features sdl-host
cp "target/$TARGET/release/zitch" "$STAGE/"

echo "=== Building butler $BUTLER_REV ==="
git clone -q https://github.com/itchio/butler.git /tmp/butler
git -C /tmp/butler checkout -q "$BUTLER_REV"
BI=github.com/itchio/butler/buildinfo
(cd /tmp/butler && CGO_ENABLED=1 GOOS=linux GOARCH=arm GOARM=7 CC=${CROSS}-gcc \
    CGO_CFLAGS="$ARCH_FLAGS -O2" go build -trimpath \
    -ldflags "-X $BI.Version=head -X $BI.Commit=$BUTLER_REV -X $BI.BuiltAt=$(date +%s) -w -s" \
    -o "$STAGE/butler" .)

# The C++ runtime is the toolchain's, newer than the device's, so it goes in
# statically.
echo "=== Building libc7zip ==="
git clone -q https://github.com/itchio/libc7zip.git /tmp/libc7zip
git -C /tmp/libc7zip checkout -q "$LIBC7ZIP_REV"
cmake -S /tmp/libc7zip -B /tmp/libc7zip/build -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_TOOLCHAIN_FILE=/opt/toolchain-a30.cmake \
    -DCMAKE_SHARED_LINKER_FLAGS="-static-libstdc++ -static-libgcc"
cmake --build /tmp/libc7zip/build -j"$(nproc)" --target c7zip
cp /tmp/libc7zip/build/libc7zip.so "$STAGE/"

echo "=== Building 7z.so from $SEVENZIP ==="
mkdir -p /tmp/7z
curl -sSfL -o /tmp/7z/src.tar.xz "https://7-zip.org/a/${SEVENZIP}-src.tar.xz"
echo "$SEVENZIP_SHA256  /tmp/7z/src.tar.xz" | sha256sum -c -
tar -xJf /tmp/7z/src.tar.xz -C /tmp/7z
# The sysroot's kernel headers predate the HWCAP2 bits 7-Zip probes on 32-bit ARM.
HWCAP2="-DHWCAP2_AES=1 -DHWCAP2_PMULL=2 -DHWCAP2_SHA1=4 -DHWCAP2_SHA2=8 -DHWCAP2_CRC32=16"
make -C /tmp/7z/CPP/7zip/Bundles/Format7zF -j"$(nproc)" -f ../../cmpl_gcc.mak \
    CROSS_COMPILE=${CROSS}- MY_ARCH="$ARCH_FLAGS $HWCAP2" \
    LDFLAGS_STATIC_3="-static-libstdc++ -static-libgcc"
cp /tmp/7z/CPP/7zip/Bundles/Format7zF/b/g/7z.so "$STAGE/"

# Everything here has to load on the A30's glibc.
echo "=== glibc check ==="
for f in "$STAGE"/*; do
    need=$(${CROSS}-objdump -T "$f" | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1)
    echo "$(basename "$f"): ${need:-none} $(${CROSS}-objdump -p "$f" | awk '/NEEDED/ {printf "%s ", $2}')"
    test -z "$need" || test "$(printf '%s\nGLIBC_%s\n' "$need" "$GLIBC_MAX" | sort -V | tail -1)" = "GLIBC_$GLIBC_MAX"
done

mkdir -p "$OUTPUT_DIR/App/zitch"
rm -rf "$OUTPUT_DIR/App/zitch/zitch-a30"
cp -a "$STAGE" "$OUTPUT_DIR/App/zitch/zitch-a30"
# zitch's own and butler's sit in zitch/licenses beside the 64-bit build.
mkdir -p "$OUTPUT_DIR/App/zitch/zitch-a30/licenses"
cp /tmp/7z/DOC/License.txt "$OUTPUT_DIR/App/zitch/zitch-a30/licenses/7zip.LICENSE"
echo "=== Done ==="
