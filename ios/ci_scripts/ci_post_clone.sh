#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"
cd "$REPO_ROOT"

CORE_CRATE="rebel-wallet_core"
LIB_NAME="rebel_wallet_core"
XCF_NAME="RebelWalletCore"
IOS_MIN_VERSION="17.0"

if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  # shellcheck disable=SC1090
  source "$HOME/.cargo/env"
fi

rustup target add aarch64-apple-ios aarch64-apple-ios-sim

DEV_DIR="$(xcode-select -p)"
TOOLCHAIN_BIN="$DEV_DIR/Toolchains/XcodeDefault.xctoolchain/usr/bin"
IOS_SDK="$(xcrun --sdk iphoneos --show-sdk-path)"
SIM_SDK="$(xcrun --sdk iphonesimulator --show-sdk-path)"

build_rust_lib() {
  local target="$1"
  local sdk="$2"
  local min_flag="$3"

  env -u SDKROOT -u MACOSX_DEPLOYMENT_TARGET -u CC -u CXX -u AR -u RANLIB \
    -u LIBRARY_PATH -u NIX_LDFLAGS -u NIX_CFLAGS_COMPILE \
    DEVELOPER_DIR="$DEV_DIR" \
    SDKROOT="$sdk" \
    CC="$TOOLCHAIN_BIN/clang" \
    CXX="$TOOLCHAIN_BIN/clang++" \
    AR="$TOOLCHAIN_BIN/ar" \
    RANLIB="$TOOLCHAIN_BIN/ranlib" \
    IPHONEOS_DEPLOYMENT_TARGET="$IOS_MIN_VERSION" \
    CFLAGS="$min_flag -isysroot $sdk" \
    CXXFLAGS="$min_flag -isysroot $sdk" \
    RUSTFLAGS="-C linker=$TOOLCHAIN_BIN/clang -C link-arg=$min_flag -C link-arg=-isysroot -C link-arg=$sdk" \
    cargo build -p "$CORE_CRATE" --lib --target "$target" --release
}

build_rust_lib "aarch64-apple-ios" "$IOS_SDK" "-miphoneos-version-min=$IOS_MIN_VERSION"
build_rust_lib "aarch64-apple-ios-sim" "$SIM_SDK" "-mios-simulator-version-min=$IOS_MIN_VERSION"

rm -rf "ios/Frameworks/$XCF_NAME.xcframework" staging
mkdir -p staging/headers
cp "ios/Bindings/${LIB_NAME}FFI.h" staging/headers/
cp "ios/Bindings/${LIB_NAME}FFI.modulemap" staging/headers/module.modulemap

xcodebuild -create-xcframework \
  -library "target/aarch64-apple-ios/release/lib${LIB_NAME}.a" -headers staging/headers \
  -library "target/aarch64-apple-ios-sim/release/lib${LIB_NAME}.a" -headers staging/headers \
  -output "ios/Frameworks/$XCF_NAME.xcframework"

rm -rf staging
