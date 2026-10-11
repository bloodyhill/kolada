# Shipping the iPad and iPhone app: TestFlight and the App Store

What is built here, and the steps that need the user's Apple Developer account (signing in and paying for it are the user's).

## Built and checked here, with no account

- `tools/ios-build.sh --run`: the app in the iPad simulator; `tools/ios-launch.sh`: its launch against an empty UIKit app.
- `tools/ios-release.sh`: the device app (arm64) with the icon, the paper or graphite launch screen, the privacy manifest (`apps/ios/Notsy/PrivacyInfo.xcprivacy`: no tracking, nothing collected; the reasons for file timestamps, the app's defaults and elapsed time), the build information the store reads (`DT*` keys), packed as `target/ios/Notsy-<version>.ipa`. Unsigned until the steps below.

## What the user does once

1. **Apple Developer Program** membership (paid, yearly), as a person or an organisation. Note the **Team ID** (developer.apple.com, Membership).
2. **App ID**: Certificates, Identifiers & Profiles, Identifiers, +: an explicit App ID `com.notsy.ios` (the bundle identifier in `apps/ios/Notsy/Info.plist`; change both together if another is wanted). Capabilities: none needed now (iCloud Drive vaults are reached through the Files picker, which needs no entitlement).
3. **Distribution certificate**: Certificates, +, Apple Distribution; make the signing request in Keychain Access (Certificate Assistant), download the certificate and open it so it is in the login keychain. `security find-identity -v -p codesigning` then lists `Apple Distribution: <Name> (<TEAMID>)`.
4. **App Store provisioning profile**: Profiles, +, App Store Connect, the App ID above, the certificate above; download `Notsy_App_Store.mobileprovision`.
5. **The app in App Store Connect**: My Apps, +, New App: iOS, name Notsy, the bundle ID above, a SKU. TestFlight's internal testers need no review.
6. **An API key for uploads**: App Store Connect, Users and Access, Integrations, App Store Connect API, +, role Developer (or App Manager); note the Key ID and Issuer ID, download `AuthKey_<KEYID>.p8` once, into `~/.appstoreconnect/private_keys/`.

## Each release

```bash
NOTSY_IOS_SIGN_ID="Apple Distribution: <Name> (<TEAMID>)" \
NOTSY_IOS_PROFILE=~/Downloads/Notsy_App_Store.mobileprovision \
NOTSY_ASC_KEY_ID=<KEYID> NOTSY_ASC_ISSUER=<ISSUER> \
tools/ios-release.sh
```

It signs with the profile's entitlements, packs the .ipa, validates and uploads it. Raise `CFBundleVersion` in `apps/ios/Notsy/Info.plist` for every upload (the store refuses a build number it has seen); the version string follows the workspace's (`Cargo.toml`).

In App Store Connect the build appears under TestFlight after processing (minutes); add internal testers, or a public group (that one has a short review). For the App Store itself: screenshots (`tools/ios-build.sh --run` and `xcrun simctl io <device> screenshot` at the sizes asked for), the description, the privacy answers (no data collected, as the manifest says), then Submit for Review.

## Not yet in the app, for a store release

- **The writing model** in process (iOS starts no helper processes); writing help stays off silently until it is there and the user has approved its download.
- **Fonts on a device**: the simulator reads the Mac's font files; a device reads `/System/Library/Fonts/Core`, checked only once a device is attached.
- **A device run**: install with `xcrun devicectl device install app --device <id> target/ios/Notsy-device.app` after signing with a development profile (the same steps with an Apple Development certificate and a Development profile listing the device).
