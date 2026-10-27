#!/usr/bin/env bash
# The macOS release files (plan 07 tasks 4 and 5), from the build's two archives:
#
#   packaging/macos/release.sh TAG ARM64_ARCHIVE X64_ARCHIVE OUT_DIR
#
# Writes to OUT_DIR, each with a .sha256:
#   uclog-TAG-macos-arm64.tar.gz, uclog-TAG-macos-x64.tar.gz   uclog and uclog-hub, signed
#   uclog-TAG-macos.dmg                                         ucLog.app (universal), signed and
#                                                               stapled; with signing only
#
# Signing, from the environment:
#   MACOS_SIGN_IDENTITY   the Developer ID Application identity (in the keychain). Unset: the
#                         archives are copied unsigned and there is no .dmg.
#   NOTARY_PROFILE        a notarytool keychain profile (xcrun notarytool store-credentials), or
#   NOTARY_KEY, NOTARY_KEY_ID, NOTARY_ISSUER   an App Store Connect API key (.p8 file), its id
#                         and issuer. Neither: signed but not notarised (for a local check).
#
# Notarising sends the files to Apple: once for the binaries and the app (then the app is
# stapled, so it opens offline), once for the .dmg (then stapled). A bare binary cannot be
# stapled: Gatekeeper checks it online.
set -euo pipefail

if (( $# != 4 )); then
  sed -n '2,20p' "$0" >&2
  exit 2
fi
tag=$1 arm64=$2 x64=$3 out=$4
n=${tag#v}
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$out"
out=$(cd "$out" && pwd)
work=$(mktemp -d)
export COPYFILE_DISABLE=1  # no ._ files in the tarballs

id=${MACOS_SIGN_IDENTITY:-}
notary=()
if [[ -n ${NOTARY_PROFILE:-} ]]; then
  notary=(--keychain-profile "$NOTARY_PROFILE")
elif [[ -n ${NOTARY_KEY:-} ]]; then
  notary=(--key "$NOTARY_KEY" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER")
fi

sign() {
  codesign --force --options runtime --timestamp --sign "$id" "$@"
  codesign --verify --strict --verbose=2 "$@"
}

notarise() {
  local r
  r=$(xcrun notarytool submit "$1" "${notary[@]}" --wait --output-format json)
  echo "$r"
  if [[ $(python3 -I -c 'import json, sys; print(json.loads(sys.argv[1])["status"])' "$r") != Accepted ]]; then
    xcrun notarytool log "$(python3 -I -c 'import json, sys; print(json.loads(sys.argv[1])["id"])' "$r")" "${notary[@]}" >&2 || true
    echo "error: notarisation of $1 failed" >&2
    exit 1
  fi
}

checksum() {
  (cd "$out" && shasum -a 256 "$1" > "$1.sha256")
}

for a in "$arm64" "$x64"; do
  tar xzf "$a" -C "$work"
done
dir_arm64=$work/uclog-$tag-macos-arm64
dir_x64=$work/uclog-$tag-macos-x64
for d in "$dir_arm64" "$dir_x64"; do
  [[ -x $d/uclog && -x $d/uclog-hub ]] || { echo "error: no uclog and uclog-hub in $d" >&2; exit 1; }
done

if [[ -z $id ]]; then
  echo "MACOS_SIGN_IDENTITY is not set: the archives are not signed, and there is no .dmg" >&2
  cp "$arm64" "$x64" "$out/"
  checksum "$(basename "$arm64")"
  checksum "$(basename "$x64")"
  exit 0
fi

# The binaries. Each gets its identifier from its embedded Info.plist (ca.unitcircle.uclog,
# ca.unitcircle.uclog-hub).
sign "$dir_arm64/uclog" "$dir_arm64/uclog-hub" "$dir_x64/uclog" "$dir_x64/uclog-hub"

# ucLog.app: the GUI, one universal uclog.
app=$work/app/ucLog.app
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
lipo -create "$dir_arm64/uclog" "$dir_x64/uclog" -output "$app/Contents/MacOS/uclog"
cp "$here/ucLog.icns" "$app/Contents/Resources/"
sed -e "s/@VERSION@/$n/" -e "s/@BUILD@/${n%%[-+]*}/" "$here/ucLog.app.plist" > "$app/Contents/Info.plist"
sign "$app"

if (( ${#notary[@]} )); then
  # One submission: the app and both architectures' binaries.
  mkdir "$work/notarise"
  ditto "$app" "$work/notarise/ucLog.app"
  ditto "$dir_arm64" "$work/notarise/arm64"
  ditto "$dir_x64" "$work/notarise/x64"
  ditto -c -k --keepParent "$work/notarise" "$work/notarise.zip"
  notarise "$work/notarise.zip"
  xcrun stapler staple "$app"
else
  echo "no notary credentials: signed, not notarised" >&2
fi

for d in "$dir_arm64" "$dir_x64"; do
  tar czf "$out/$(basename "$d").tar.gz" -C "$work" "$(basename "$d")"
  checksum "$(basename "$d").tar.gz"
done

# The .dmg: the app and a link to /Applications to drag it to.
dmg=uclog-$tag-macos.dmg
mkdir "$work/dmg"
ditto "$app" "$work/dmg/ucLog.app"
ln -s /Applications "$work/dmg/Applications"
hdiutil create -quiet -volname "ucLog $n" -srcfolder "$work/dmg" -fs HFS+ -format UDZO -ov "$out/$dmg"
codesign --force --timestamp --sign "$id" "$out/$dmg"
if (( ${#notary[@]} )); then
  notarise "$out/$dmg"
  xcrun stapler staple "$out/$dmg"
fi
checksum "$dmg"
echo "macOS release files in $out" >&2
