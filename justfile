set shell := ["bash", "-c"]

CORE_CRATE := "rebel-wallet_core"
LIB_NAME := "rebel_wallet_core"
XCF_NAME := "RebelWalletCore"
ICED_PACKAGE := "rebel-wallet_core_desktop_iced"
DYLIB_EXT := if os() == "macos" { "dylib" } else { "so" }

default:
  @just --list

doctor:
  rmp doctor

ios-tailnet-doctor *args:
  ./tools/ios-tailnet-doctor {{args}}

install-hooks:
  git config core.hooksPath .githooks

# Build Rust core for the host (needed for uniffi-bindgen).
rust-build-host:
  ./tools/cargo-with-xcode build -p {{CORE_CRATE}} --release

bindings:
  rmp bindings all

# ── iOS ──────────────────────────────────────────────────────────────────────

run-ios:
  rmp run ios

ios-gen-swift: rust-build-host
  cargo run -p uniffi-bindgen -- generate \
    --library target/release/lib{{LIB_NAME}}.{{DYLIB_EXT}} \
    --language swift \
    --out-dir ios/Bindings \
    --config rust/uniffi.toml

# Cross-compile Rust for iOS device and simulator (arm64).
ios-rust:
  #!/usr/bin/env bash
  set -e
  DEV_DIR="$(./tools/xcode-dev-dir)"
  TOOLCHAIN_BIN="$DEV_DIR/Toolchains/XcodeDefault.xctoolchain/usr/bin"
  IOS_SDK="$DEV_DIR/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk"
  SIM_SDK="$DEV_DIR/Platforms/iPhoneSimulator.platform/Developer/SDKs/iPhoneSimulator.sdk"
  for pair in "aarch64-apple-ios $IOS_SDK -miphoneos-version-min=17.0" \
              "aarch64-apple-ios-sim $SIM_SDK -mios-simulator-version-min=17.0"; do
    set -- $pair; TARGET=$1; SDK=$2; VFLAG=$3
    env -u SDKROOT -u MACOSX_DEPLOYMENT_TARGET -u CC -u CXX -u AR -u RANLIB \
      -u LIBRARY_PATH -u NIX_LDFLAGS -u NIX_CFLAGS_COMPILE \
      DEVELOPER_DIR="$DEV_DIR" SDKROOT="$SDK" CC="$TOOLCHAIN_BIN/clang" \
      RUSTFLAGS="-C linker=$TOOLCHAIN_BIN/clang -C link-arg=$VFLAG -C link-arg=-isysroot -C link-arg=$SDK" \
      cargo build -p {{CORE_CRATE}} --lib --target "$TARGET" --release
  done

# Package static libs into an xcframework.
ios-xcframework:
  #!/usr/bin/env bash
  set -e
  rm -rf ios/Frameworks/{{XCF_NAME}}.xcframework staging
  mkdir -p staging/headers
  cp ios/Bindings/{{LIB_NAME}}FFI.h staging/headers/
  cp ios/Bindings/{{LIB_NAME}}FFI.modulemap staging/headers/module.modulemap
  ./tools/xcode-run xcodebuild -create-xcframework \
    -library target/aarch64-apple-ios/release/lib{{LIB_NAME}}.a -headers staging/headers \
    -library target/aarch64-apple-ios-sim/release/lib{{LIB_NAME}}.a -headers staging/headers \
    -output ios/Frameworks/{{XCF_NAME}}.xcframework
  rm -rf staging

ios-xcodeproj:
  cd ios && xcodegen generate

# Build the iOS app for simulator.
ios-build:
  ./tools/xcode-run xcodebuild build \
    -project ios/App.xcodeproj -scheme App \
    -destination "generic/platform=iOS Simulator" \
    -configuration Debug CODE_SIGNING_ALLOWED=NO ARCHS=arm64 ONLY_ACTIVE_ARCH=YES

# Build the iOS app for a paired physical device that Xcode/CoreDevice can see.
ios-device-build device_id derived_data="build/ios-device":
  ./tools/xcode-run xcodebuild build \
    -project ios/App.xcodeproj -scheme App \
    -destination "id={{device_id}}" \
    -configuration Debug \
    -derivedDataPath "{{derived_data}}"

# Install the last physical-device build onto a paired device.
ios-device-install device_id derived_data="build/ios-device":
  xcrun devicectl device install app \
    --device "{{device_id}}" \
    "{{derived_data}}/Build/Products/Debug-iphoneos/App.app"

# Build and install onto a paired physical device.
ios-device-deploy device_id derived_data="build/ios-device":
  just ios-device-build "{{device_id}}" "{{derived_data}}"
  just ios-device-install "{{device_id}}" "{{derived_data}}"

# Full iOS pipeline: host build → bindings → cross-compile → xcframework → xcodegen → build.
ios-full: ios-gen-swift ios-rust ios-xcframework ios-xcodeproj ios-build

# ── Utilities ────────────────────────────────────────────────────────────────

# Rebuild Rust + regenerate bindings for all platforms (no platform build).
rebind: rust-build-host bindings
