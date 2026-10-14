# Attribution — Android port

- **GridCraft**: <https://github.com/storytold/gridcraft> (MIT OR Apache-2.0), unmodified except for the Android patch on the `android` branch.
- **Android shell** (`apps/gridcraft-android/`, the vendored-crate notes and the save-dialog/clipboard patches) was started
  from [@kanaaka/gridcraft-android](https://github.com/kanaaka/gridcraft-android) (Apache-2.0 OR MIT, October 2026) and
  forward-ported to upstream v0.7.0 by [@nideta231](https://github.com/nideta231), who added the Android system bridge
  (`src/android_bridge.rs`), package identity, size-optimised build profile and release CI.
- Unofficial: not produced or endorsed by storytold / the ArtCraft team.
