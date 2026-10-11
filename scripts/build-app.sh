#!/usr/bin/env bash
#
# midistage.app を組む（docs/design/05「署名・公証・配布」。Ladyland の scripts/build-app.sh を写した）。
#
# メニューバーの SwiftUI app に Rust の midistaged を同梱し、LaunchAgent plist を
# Contents/Library/LaunchAgents/ に置く。SMAppService がそれを登録する。
#
#   scripts/build-app.sh              # ./dist/midistage.app
#   scripts/build-app.sh --install    # /Applications へ配置（動いていれば終了してから）
#   scripts/build-app.sh --run        # 作って起動
#   scripts/build-app.sh --dist       # 公証 + staple + DMG（Developer ID が要る）
#   scripts/build-app.sh --publish    # --dist + その tag の GitHub Release へ DMG を添付（tag の上でだけ）
#
# 署名: Developer ID があれば Hardened Runtime で署名（公証に出せる）、無ければ ad-hoc。
# 同梱 midistaged（ネストした実行ファイル）は先に個別署名する — 公証はこれが無いと弾く。
# entitlements は付けない（AU ホストではないので library validation を外す理由が無い）。
# 公証の資格情報は Team 単位。"midistage" か "ladyland" の keychain profile があればそれを使う。無ければ一度だけ（対話）:
#   xcrun notarytool store-credentials "midistage" --apple-id <Apple ID> --team-id 3EQKG4B352 --password <アプリ用パスワード>
# 対象は arm64 / macOS 14 以降。
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_PKG="$REPO_ROOT/apps/menubar"
DIST="$REPO_ROOT/dist"
APP="$DIST/midistage.app"
CONTENTS="$APP/Contents"
# 公証の資格情報は Apple ID / Team 単位でアプリごとではない。Ladyland で登録した "ladyland" が
# 同じ Team（3EQKG4B352）なので、"midistage" が無ければそれを使う（2026-10-12 実測）
notary_profile() {
    if [ -n "${MIDISTAGE_NOTARY_PROFILE:-}" ]; then echo "$MIDISTAGE_NOTARY_PROFILE"; return; fi
    for p in midistage ladyland; do
        if xcrun notarytool history --keychain-profile "$p" >/dev/null 2>&1; then echo "$p"; return; fi
    done
    echo "midistage"
}
NOTARY_PROFILE=""
BUNDLE_ID="club.chronista.midistage"
AGENT_PLIST="club.chronista.midistaged.plist"
INSTALL=0; RUN=0; MAKE_DIST=0; PUBLISH=0
for arg in "$@"; do
    case "$arg" in
        --install) INSTALL=1 ;;
        --run) RUN=1 ;;
        --dist) MAKE_DIST=1 ;;
        --publish) PUBLISH=1; MAKE_DIST=1 ;;
        *) echo "不明な引数: $arg" >&2; exit 2 ;;
    esac
done
if [ "$PUBLISH" = "1" ]; then
    RELEASE_TAG="$(git -C "$REPO_ROOT" describe --tags --exact-match 2>/dev/null || true)"
    [ -n "$RELEASE_TAG" ] || { echo "--publish は tag の上でだけ使えます" >&2; exit 1; }
    [ -z "$(git -C "$REPO_ROOT" status --porcelain)" ] || { echo "--publish: 未コミットの変更があります" >&2; exit 1; }
    gh auth status >/dev/null 2>&1 || { echo "--publish: gh が未認証です" >&2; exit 1; }
fi
cd "$REPO_ROOT"
SHORT_VERSION="$(git describe --tags --abbrev=0 2>/dev/null | sed 's/^v//' || echo "0.0.1")"
BUILD_VERSION="$(git rev-list --count HEAD).$(git rev-parse --short HEAD)"

echo "==> midistaged を release ビルド"
cargo build --release -p midistaged
MIDISTAGED="$REPO_ROOT/target/release/midistaged"
test -x "$MIDISTAGED"

echo "==> MidistageApp を release ビルド"
( cd "$APP_PKG" && swift build -c release --product MidistageApp )
BINARY="$(cd "$APP_PKG" && swift build -c release --product MidistageApp --show-bin-path)/MidistageApp"
test -x "$BINARY"

echo "==> バンドルを組む"
rm -rf "$APP"
mkdir -p "$CONTENTS/MacOS" "$CONTENTS/Resources" "$CONTENTS/Library/LaunchAgents"
cp "$BINARY" "$CONTENTS/MacOS/MidistageApp"
cp "$MIDISTAGED" "$CONTENTS/MacOS/midistaged"
cp "$APP_PKG/Resources/$AGENT_PLIST" "$CONTENTS/Library/LaunchAgents/$AGENT_PLIST"
plutil -lint "$CONTENTS/Library/LaunchAgents/$AGENT_PLIST" >/dev/null
cat > "$CONTENTS/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>midistage</string>
    <key>CFBundleDisplayName</key><string>midistage</string>
    <key>CFBundleIdentifier</key><string>${BUNDLE_ID}</string>
    <key>CFBundleExecutable</key><string>MidistageApp</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>${SHORT_VERSION}</string>
    <key>CFBundleVersion</key><string>${BUILD_VERSION}</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.music</string>
    <!-- メニューバー常駐。Dock / Cmd+Tab に出ない -->
    <key>LSUIElement</key><true/>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
PLIST

IDENTITY="${MIDISTAGE_SIGN_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null \
    | grep "Developer ID Application" | head -1 | sed -E 's/.*"(.*)"/\1/')}"
if [ -n "$IDENTITY" ]; then
    echo "==> Developer ID で署名 + Hardened Runtime: $IDENTITY"
    codesign --force --options runtime --timestamp --sign "$IDENTITY" "$CONTENTS/MacOS/midistaged"
    codesign --force --options runtime --timestamp --sign "$IDENTITY" "$CONTENTS/MacOS/MidistageApp"
    codesign --force --options runtime --timestamp --sign "$IDENTITY" "$APP"
    codesign --verify --strict --verbose=1 "$APP" 2>&1 | sed 's/^/    /'
else
    echo "==> ad-hoc 署名（Developer ID が見つからないため。配布はできない）"
    codesign --force --sign - "$CONTENTS/MacOS/midistaged"
    codesign --force --sign - "$CONTENTS/MacOS/MidistageApp"
    codesign --force --sign - "$APP"
fi
touch "$APP"
echo "==> できました: $APP (${SHORT_VERSION} / ${BUILD_VERSION})"

if [ "$MAKE_DIST" = "1" ]; then
    [ -n "$IDENTITY" ] || { echo "配布には Developer ID 署名が必要です" >&2; exit 1; }
    NOTARY_PROFILE="$(notary_profile)"
    echo "==> 公証プロファイル: $NOTARY_PROFILE"
    if ! xcrun notarytool history --keychain-profile "$NOTARY_PROFILE" >/dev/null 2>&1; then
        echo "公証の資格情報 \"$NOTARY_PROFILE\" が未登録です。一度だけ:" >&2
        echo "  xcrun notarytool store-credentials \"$NOTARY_PROFILE\" --apple-id <Apple ID> --team-id 3EQKG4B352 --password <アプリ用パスワード>" >&2
        exit 1
    fi
    ZIP="$DIST/midistage-${SHORT_VERSION}.zip"
    echo "==> 公証へ提出（数分）"
    ditto -c -k --keepParent "$APP" "$ZIP"
    xcrun notarytool submit "$ZIP" --keychain-profile "$NOTARY_PROFILE" --wait 2>&1 | sed 's/^/    /'
    xcrun stapler staple "$APP" 2>&1 | sed 's/^/    /'
    rm -f "$ZIP"
    echo "==> DMG"
    DMG="$DIST/midistage-${SHORT_VERSION}.dmg"
    STAGE="$(mktemp -d)/midistage"; mkdir -p "$STAGE"; cp -R "$APP" "$STAGE/"; ln -s /Applications "$STAGE/Applications"
    rm -f "$DMG"
    hdiutil create -volname "midistage" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
    rm -rf "$(dirname "$STAGE")"
    codesign --sign "$IDENTITY" --timestamp "$DMG" 2>&1 | sed 's/^/    /'
    xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait 2>&1 | sed 's/^/    /'
    xcrun stapler staple "$DMG" 2>&1 | sed 's/^/    /'
    spctl -a -t install "$DMG" 2>&1 | sed 's/^/    /' || { echo "DMG の検証に失敗" >&2; exit 1; }
    if [ "$PUBLISH" = "1" ]; then
        gh release view "$RELEASE_TAG" >/dev/null 2>&1 || gh release create "$RELEASE_TAG" --title "$RELEASE_TAG" --generate-notes >/dev/null
        gh release upload "$RELEASE_TAG" "$DMG" --clobber >/dev/null
        echo "==> $(gh release view "$RELEASE_TAG" --json url -q .url)"
    fi
fi

if [ "$INSTALL" = "1" ]; then
    TARGET="/Applications/midistage.app"
    if pgrep -x MidistageApp >/dev/null; then
        echo "==> 動いている midistage を終了"
        osascript -e "tell application \"$TARGET\" to quit" 2>/dev/null || true
        for _ in $(seq 1 20); do pgrep -x MidistageApp >/dev/null || break; sleep 0.5; done
    fi
    rm -rf "$TARGET"; cp -R "$APP" "$TARGET"
    echo "==> 配置: $TARGET（常駐の登録はメニューから「常駐を有効にする」）"
    [ "$RUN" = "1" ] && open "$TARGET"
elif [ "$RUN" = "1" ]; then
    open "$APP"
fi
