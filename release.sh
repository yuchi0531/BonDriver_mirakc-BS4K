#!/usr/bin/env bash
# ====================================================================
#  BonDriver_Mirakurun release packaging script
#
#  3 ターゲットをビルドして、同梱 zip を release/ に出力する。
#    - win-x86_64 : x86_64-pc-windows-msvc  -> BonDriver_Mirakurun.dll
#    - win-x86    : i686-pc-windows-msvc    -> BonDriver_Mirakurun.dll
#    - linux-x86_64: x86_64-unknown-linux-gnu -> BonDriver_Mirakurun.so
#
#  Usage:
#    ./release.sh [version]
#      version 省略時は Cargo.toml の version を使う。
#
#  Windows ターゲットのクロスコンパイルには cargo-xwin と、
#  新しい lld-link(rust-lld 同梱)が必要。環境変数で上書き可能:
#    LLD_LINK  使用する lld-link のパス(既定: rustup の gcc-ld/lld-link)
#
#  出力:
#    release/BonDriver_Mirakurun-<ver>-win-x86_64.zip
#    release/BonDriver_Mirakurun-<ver>-win-x86.zip
#    release/BonDriver_Mirakurun-<ver>-linux-x86_64.zip
# ====================================================================
set -euo pipefail
cd "$(dirname "$0")"

export PATH="$HOME/.cargo/bin:$PATH"

VERSION="${1:-$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')}"
echo "[INFO] version = $VERSION"

OUT="release"
STAGE="$OUT/_stage"
rm -rf "$OUT"
mkdir -p "$STAGE"

# ---- Windows ビルド用の lld-link ラッパ ------------------------------------
# cargo-xwin 同梱の lld-link は /export:...,EXPORTAS,... を解釈できないため、
# rust-lld(新しい LLD)を使う。cargo-xwin は "-flavor link" を前置するので
# ラッパで取り除く。
setup_win_linker() {
    if [ -n "${LLD_LINK:-}" ]; then
        RUST_LLD="$LLD_LINK"
    else
        RUST_LLD="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | awk '/host:/{print $2}')/bin/gcc-ld/lld-link"
    fi
    if [ ! -x "$RUST_LLD" ]; then
        echo "[ERROR] lld-link not found: $RUST_LLD" >&2
        exit 1
    fi
    WRAP="$OUT/_lldwrap-link"
    cat > "$WRAP" <<EOF
#!/bin/sh
if [ "\$1" = "-flavor" ] && [ "\$2" = "link" ]; then shift 2; fi
exec "$RUST_LLD" "\$@"
EOF
    chmod +x "$WRAP"
    echo "$WRAP"
}

build_win() {
    local triple="$1"
    local wrap
    wrap="$(setup_win_linker)"
    echo "[INFO] building $triple ..."
    XWIN_ARCH="x86_64,x86" RUSTFLAGS="-Clinker=$wrap" \
        cargo xwin build --release --target "$triple"
}

# ---- Windows x86_64 -------------------------------------------------------
build_win x86_64-pc-windows-msvc
cp -f target/x86_64-pc-windows-msvc/release/BonDriver_Mirakurun.dll \
      "$STAGE/BonDriver_Mirakurun.dll"
cp -f BonDriver_Mirakurun.ini "$STAGE/BonDriver_Mirakurun.ini"
( cd "$STAGE" && zip -q -r "../BonDriver_Mirakurun-$VERSION-win-x86_64.zip" . )
rm -f "$STAGE"/*

# ---- Windows x86 (32bit) --------------------------------------------------
build_win i686-pc-windows-msvc
cp -f target/i686-pc-windows-msvc/release/BonDriver_Mirakurun.dll \
      "$STAGE/BonDriver_Mirakurun.dll"
cp -f BonDriver_Mirakurun.ini "$STAGE/BonDriver_Mirakurun.ini"
( cd "$STAGE" && zip -q -r "../BonDriver_Mirakurun-$VERSION-win-x86.zip" . )
rm -f "$STAGE"/*

# ---- Linux x86_64 ---------------------------------------------------------
echo "[INFO] building x86_64-unknown-linux-gnu ..."
cargo build --release --target x86_64-unknown-linux-gnu
cp -f target/x86_64-unknown-linux-gnu/release/libBonDriver_Mirakurun.so \
      "$STAGE/BonDriver_Mirakurun.so"
cp -f BonDriver_Mirakurun.ini "$STAGE/BonDriver_Mirakurun.so.ini"
( cd "$STAGE" && zip -q -r "../BonDriver_Mirakurun-$VERSION-linux-x86_64.zip" . )
rm -f "$STAGE"/*

rmdir "$STAGE"
echo
echo "[OK] release artifacts:"
ls -la "$OUT"/*.zip
