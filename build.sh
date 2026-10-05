#!/usr/bin/env bash
# ====================================================================
#  BonDriver_Mirakurun build script (Linux)
#
#  Usage:
#    ./build.sh            build for the host (x86_64-unknown-linux-gnu)
#
#  Output: dist/BonDriver_Mirakurun.so and dist/BonDriver_Mirakurun.so.ini
#
#  生成物は EDCB の BonDriver ディレクトリに配置して使う。
#  INI は `<so名>.ini`(例: BonDriver_Mirakurun.so.ini)として .so と同じ
#  ディレクトリに置くこと。
# ====================================================================
set -euo pipefail
cd "$(dirname "$0")"

# rustup/cargo を PATH に通す(未ログインのシェル対策)。
export PATH="$HOME/.cargo/bin:$PATH"

TARGET="${1:-x86_64-unknown-linux-gnu}"

echo "[INFO] target = $TARGET"

if command -v rustup >/dev/null 2>&1; then
    rustup target add "$TARGET" >/dev/null 2>&1 || true
fi

echo "[INFO] cargo build --release --target $TARGET"
cargo build --release --target "$TARGET"

# 生成物を dist へコピー。Rust の出力は libBonDriver_Mirakurun.so なので
# ホストが期待する `BonDriver_Mirakurun.so` にリネームする。
if [ "$TARGET" = "$(rustc -vV | awk '/host:/{print $2}')" ]; then
    SRC="target/release/libBonDriver_Mirakurun.so"
else
    SRC="target/$TARGET/release/libBonDriver_Mirakurun.so"
fi

mkdir -p dist
cp -f "$SRC" dist/BonDriver_Mirakurun.so
cp -f BonDriver_Mirakurun.ini dist/BonDriver_Mirakurun.so.ini

echo
echo "[OK] Build complete: dist/BonDriver_Mirakurun.so"
