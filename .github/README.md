<h1 align="center">GridCraft for Android <sup>(Unofficial)</sup></h1>

<p align="center">
  <b>The real <a href="https://github.com/storytold/gridcraft">storytold/gridcraft</a> spreadsheet (Excel-class) — Rust engine and egui UI — running natively on Android tablets.</b><br>
  Not a web wrapper, not a rewrite: the upstream engine, built for <code>arm64-v8a</code>, with an Android shell around it.
</p>

<p align="center">
  <a href="https://github.com/nideta231/gridcraft/releases/latest"><img alt="Download APK" src="https://img.shields.io/github/v/release/nideta231/gridcraft?label=APK&style=for-the-badge&color=2e7d32"></a>
  <img alt="Android 8+" src="https://img.shields.io/badge/Android-8.0%2B-3ddc84?style=for-the-badge&logo=android&logoColor=white">
  <img alt="Tablet first" src="https://img.shields.io/badge/focus-tablet-1f6feb?style=for-the-badge">
</p>

<p align="center"><img alt="GridCraft on an Android tablet" src="https://github.com/nideta231/gridcraft/raw/android/docs/android/demo.png" width="860"></p>

> **Unofficial.** This is a community port by [@nideta231](https://github.com/nideta231). It is not made, reviewed or
> endorsed by storytold / the ArtCraft team. Report Android problems **here**, not upstream.
> Only download GridCraft from [storytold/gridcraft](https://github.com/storytold/gridcraft/releases), getartcraft.com, or this repo's
> Releases — other "ArtCraft" repos have shipped malware.

## Install

1. Download `gridcraft-<version>-arm64-v8a.apk` from [Releases](https://github.com/nideta231/gridcraft/releases/latest) (≈7.9 MB).
2. Open it on the tablet and allow *Install unknown apps* for your browser / file manager when asked.
3. Launch **GridCraft (Unofficial)**.

Requirements: Android 8.0+, 64-bit ARM (virtually every tablet/phone since 2017), Vulkan or GLES 3.
Package id `com.nideta231.gridcraft` — it installs next to any other build, never over it.

## What's different from the desktop build

Android-specific work, on top of the unmodified upstream engine:

- **"Open with" / "Share" from other apps** — tap a file in Files, WhatsApp, Telegram or Gmail and pick GridCraft; it opens straight in, no permission needed.
- **System clipboard** — copy cells in GridCraft, paste into WhatsApp/Docs/Sheets, and back (plain text + HTML so numbers stay numbers).
- **Links open in the browser** (About, Backstage, Help).
- **Update banner**: on start (and on return, at most every 30 min) the app asks GitHub for this fork's latest release; if newer, a banner offers **Download** / **What's new** / **Later**. Needs internet only for this; nothing is installed automatically.
- **Touch**: one finger taps and drag-selects; **two-finger drag scrolls in any direction** (vertical, horizontal, diagonal, with a coasting flick); pinch zooms. The first finger's press waits ~0.14 s for a second finger, so a scroll never starts a selection; Shift/Ctrl from a hardware keyboard work (Android sends no modifier events — the shell tracks them).
- **Built-in file picker** with Downloads/Documents/app-folder roots; Save remembers the file so Ctrl+S saves silently afterwards.
- **Preferences and recent files survive** a swipe-away from Recents (written while the app runs — Android never says goodbye).
- **Edge-to-edge fullscreen**, survives rotation, split-screen and keyboard attach without restarting the Activity.
- **Size-optimised release build** (fat LTO, `opt-level=s`, stripped).

Status: **tablet-first.** A compact phone layout is planned; on a phone it runs, but the ribbon is desktop-sized.

### Known limits
- Printing goes through *Export to PDF* — no Android print service hookup yet.
- "All files access" (Settings ▸ Apps ▸ GridCraft ▸ Special access) is only needed to browse **outside** Downloads/Documents/app folder in the built-in picker.
- No stylus pressure/tilt.

## Branches and syncing

| Branch | What it is |
|---|---|
| `main` | exact mirror of upstream `storytold/gridcraft` (auto-synced daily by `sync-upstream.yml`) |
| `android` | `main` + `apps/gridcraft-android/` + a small Android patch. Releases are tagged here as `v0.7.0-android.N` |

Moving to a new upstream release: `git rebase <new-upstream-tag>` on `android`, build, tag `vX.Y.Z-android.1`, push — CI (`android-release.yml`) builds, signs and publishes the APK.

## Build it yourself

```sh
rustup target add aarch64-linux-android
cargo install cargo-apk --version 0.10.0
export ANDROID_HOME=~/Android/Sdk ANDROID_NDK_ROOT=~/Android/Sdk/ndk/<version>
cd apps/gridcraft-android
cargo apk build --profile android          # needs a keystore: CARGO_APK_ANDROID_KEYSTORE(+_PASSWORD)
adb install -r ../../target/android/apk/gridcraft.apk
```
Testing on an x86_64 emulator: add `--target x86_64-linux-android`.

## Credits and licence

- **GridCraft** is by the ArtCraft team / [storytold](https://github.com/storytold/gridcraft) — the engine, UI and everything that makes it a spreadsheet app.
- The NativeActivity shell started from [@kanaaka](https://github.com/kanaaka)'s gridcraft-android port (Apache-2.0 OR MIT); see [`ATTRIBUTION-android.md`](https://github.com/nideta231/gridcraft/blob/android/ATTRIBUTION-android.md).
- Same licence as upstream: **MIT OR Apache-2.0** ([`LICENSE-MIT`](https://github.com/nideta231/gridcraft/blob/android/LICENSE-MIT), [`LICENSE-APACHE`](https://github.com/nideta231/gridcraft/blob/android/LICENSE-APACHE)).
- The ArtCraft name and logo are ArtCraft trademarks (`docs/brand/LICENSE-brand.txt`); this port does not use the logo as its own.

The upstream README is unchanged in [`README.md`](https://github.com/nideta231/gridcraft/blob/android/README.md).
