#!/usr/bin/env bash
# Builds the Android app: libmain.so (the game) and libSDL3.so for the ABIs given, SDL's Java glue, then the APK.
#   android/build-apk.sh [--release | --fast] [abi ...]   default ABI: x86_64 (the emulator); use arm64-v8a for a phone
# --fast: a signed release APK built with the `phone` profile (no LTO, incremental) and a warm Gradle daemon, for test builds.
# Needs: Rust with the Android targets, cargo-ndk, the Android SDK and NDK (ANDROID_HOME, ANDROID_NDK_HOME),
# a JDK 17+, cmake, and SSD_APK_ASSETS_DIR: the folder of the art overrides and voice to bundle (see below).
# The APK lands in android/app/build/outputs/apk/.
set -euo pipefail
cd "$(dirname "$0")/.."
# .cargo/config.toml points CMAKE at a Windows path; an environment value wins over it.
export CMAKE="${CMAKE_ANDROID:-$(command -v cmake)}"
# The APK's version and code from android/app/build.gradle (`v1.0.1 (30)`), named in the report a failed start shows
# (crates/ssd-app/src/android_report.rs).
export SSD_APK_VERSION="$(sed -n "s/^ *versionName '\(.*\)'.*/\1/p" android/app/build.gradle) ($(sed -n 's/^ *versionCode \([0-9]*\).*/\1/p' android/app/build.gradle))"
profile=debug; cargo_profile=(); gradle_task=assembleDebug
if [ "${1:-}" = "--release" ]; then profile=release; cargo_profile=(--release); gradle_task=assembleRelease; shift; fi
daemon=--no-daemon
if [ "${1:-}" = "--fast" ]; then profile=phone; cargo_profile=(--profile phone); gradle_task=assembleRelease; daemon=--daemon; shift; fi
abis=("$@"); [ ${#abis[@]} -eq 0 ] && abis=(x86_64)
# A debug build still runs the game's own crates fully optimised: at the dev profile's level 1 the HD renderer is too slow
# on a phone. Without debug info the library is ten times smaller.
export CARGO_PROFILE_DEV_OPT_LEVEL="${CARGO_PROFILE_DEV_OPT_LEVEL:-3}" CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-0}"

jni=android/app/src/main/jniLibs
rm -rf "$jni"
for abi in "${abis[@]}"; do
    case $abi in
        arm64-v8a) triple=aarch64-linux-android ;;
        x86_64) triple=x86_64-linux-android ;;
        *) echo "unsupported ABI $abi" >&2; exit 2 ;;
    esac
    export CMAKE_TOOLCHAIN_FILE="${ANDROID_NDK_HOME:?}/build/cmake/android.toolchain.cmake" ANDROID_ABI="$abi" ANDROID_PLATFORM=android-26
    # (an empty array expanded so, as macOS's bash 3.2 under `set -u` takes a bare "${a[@]}" for an unbound variable)
    cargo ndk -t "$abi" -o "$jni" build -p ssd-android ${cargo_profile[@]+"${cargo_profile[@]}"}
    # cargo-ndk copies libmain.so only; SDL's shared library is built by its build script.
    sdl=$(ls -t target/"$triple"/"$profile"/build/sdl3-sys-*/out/lib/libSDL3.so 2>/dev/null | head -1 || true)
    [ -n "$sdl" ] || sdl=$(find target/"$triple"/"$profile"/build -name libSDL3.so | head -1)
    cp "$sdl" "$jni/$abi/libSDL3.so"
done

# SDL's Java glue (zlib licence) comes from the SDL sources cargo already fetched; it is not tracked.
sdl_java=$(ls -d "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/sdl3-src-*/SDL/android-project/app/src/main/java/org/libsdl/app | head -1)
rm -rf android/app/src/main/java/org
mkdir -p android/app/src/main/java/org/libsdl
cp -r "$sdl_java" android/app/src/main/java/org/libsdl/app

# The files the game reads from its data folder besides the translation: the desktop launcher's art overrides
# (art-override/) and the transformation's voice (voice/ssj-transform.wav). They are not in the repository; SSD_APK_ASSETS_DIR
# names the folder holding them (the launcher's assets, launcher/fetch_assets.sh). They go to build/bundled-assets, an assets
# folder of android/app/build.gradle, with their list bundled-files.txt, which the game reads at start
# (crates/ssd-app/src/android.rs, install_bundled). Rebuilt each time, so that no file of an older build stays.
src_assets="${SSD_APK_ASSETS_DIR:?set SSD_APK_ASSETS_DIR to the folder holding art-override/ and voice/ (the assets of the launcher)}"
[ -d "$src_assets/art-override" ] && [ -f "$src_assets/voice/ssj-transform.wav" ] ||
    { echo "$src_assets has no art-override/ or no voice/ssj-transform.wav" >&2; exit 2; }
bundled=android/app/build/bundled-assets
rm -rf "$bundled"
mkdir -p "$bundled/voice"
cp -R "$src_assets/art-override" "$bundled/art-override"
cp "$src_assets/voice/ssj-transform.wav" "$bundled/voice/ssj-transform.wav"
# The note the desktop launcher writes beside the pictures (art-override\LICENSE-ART.txt: where they come from, their
# CC BY-NC-SA licence, how to go back), the same bytes: the string ART_LICENSE of launcher/src/main.rs, read here.
python3 - launcher/src/main.rs "$bundled/art-override/LICENSE-ART.txt" <<'PY'
import re, sys
source = open(sys.argv[1], encoding="utf-8").read()
found = re.search(r'^const ART_LICENSE: &str = "((?:[^"\\]|\\.)*)";', source, re.M | re.S)
if not found:
    sys.exit(f"{sys.argv[1]}: no const ART_LICENSE")
literal, text, at = found.group(1), [], 0
escapes = {"r": "\r", "n": "\n", "t": "\t", "\\": "\\", '"': '"', "'": "'", "0": "\0"}
while at < len(literal):
    c = literal[at]
    if c != "\\":
        text.append(c); at += 1; continue
    e = literal[at + 1]
    if e == "\n":  # a line continuation: the newline and the next line's leading whitespace are dropped
        at += 2
        while at < len(literal) and literal[at] in " \t\n\r":
            at += 1
    elif e == "u":
        end = literal.index("}", at)
        text.append(chr(int(literal[at + 3:end], 16))); at = end + 1
    elif e in escapes:
        text.append(escapes[e]); at += 2
    else:
        sys.exit(f"unknown escape \\{e} in ART_LICENSE")
open(sys.argv[2], "wb").write("".join(text).encode("utf-8"))
PY
(cd "$bundled" && find art-override voice -type f | LC_ALL=C sort > bundled-files.txt)
echo "bundled $(wc -l < "$bundled/bundled-files.txt" | tr -d ' ') files from $src_assets"

# Gradle packages an APK incrementally over the last one and never gives back the room of a bigger old library.
rm -rf android/app/build/outputs/apk
(cd android && ./gradlew "$daemon" "$gradle_task")
