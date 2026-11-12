# Horizon on iOS (arm64, experimental)

The same game as the Android port, built for Apple's phones and tablets. Rendering is **wgpu on
Metal** — the one backend an iOS build compiles in; Vulkan and GLES are not linked there at all.
The game runs as an ordinary iOS app bundle: its executable is `horizon-client`'s `main`, which
iOS starts on the main thread (where winit's event loop must be made and run), and
`crates/horizon-client/src/entry.rs` sets the game's folders from the app's own container.

Status: builds, links and packages on Apple silicon; **not yet run on a device**. The same is
true of the Android port (see [ANDROID.md](ANDROID.md)).

## Build the IPA (macOS host)

```sh
rustup target add aarch64-apple-ios
python3 packaging/ios/fetch_assets.py .minecraft          # Mojang's 26.3 files (~525 MB)
cargo run --release -p horizon-prepare -- --prepare \
  --minecraft-dir .minecraft --server-jar server.jar      # -> .local/horizon-client/26.3
./packaging/ios/build_ipa.sh                              # -> target/ios/Horizon.ipa
```

`build_ipa.sh` builds `horizon-client` for `aarch64-apple-ios` with `--no-default-features
--features audio` (the `controllers` feature is gilrs, a desktop library), copies the prepared
resources beside the executable (where the game looks for them), fills in
`packaging/ios/Info.plist` and zips `Payload/Horizon.app` into an IPA. The signature is ad hoc
(`codesign -s -`): enough for a jailbroken device, while a real device wants your own signing
identity — sign the assembled `Payload/Horizon.app` with it, or let Xcode do it.

CI builds the same thing on `macos-15` ([build-ios.yml](.github/workflows/build-ios.yml)) and
uploads the IPA as an artifact. Nothing is patched at build time: the platform is in the source.

## Install

```sh
# From a Mac with the device attached and your own signing identity:
codesign --force --sign "Apple Development: you@example.com" target/ios/Payload/Horizon.app
ios-deploy --bundle target/ios/Payload/Horizon.app
```

Or copy `Horizon.ipa` to the device (AltStore, Sideloadly) or open it in the simulator's
`simctl`. Needs iOS 13 or newer, arm64 and Metal.

## Game files

Horizon never ships Minecraft's files. `fetch_assets.py` downloads the manifest, the client and
server jars, the libraries and every asset object into `.minecraft`; `horizon-prepare` turns them
into `.local/horizon-client/26.3`, which the IPA carries in `Horizon.app/resources`.

The game folder is the app's `Documents`, shown in the **Files** app under *On My iPhone >
Horizon* (`UIFileSharingEnabled` and `LSSupportsOpeningDocumentsInPlace` in the Info.plist):
`saves`, `resourcepacks`, `mods` and `shaderpacks` all live there and can be dropped in over
iTunes, the Files app or iCloud Drive.

## What is iOS-specific in the code

| Where | What |
| --- | --- |
| `horizon-client-render/src/gfx.rs` | `backend_order` picks Metal on `macos` and `ios`; `horizon_hdr::open_vulkan_device` is not called; the driver-failure message names Metal |
| `horizon-client-render/Cargo.toml` | the `metal` wgpu feature is the only one for Apple targets; `vulkan` and `gles` are compiled for Windows/Linux/Android only |
| `horizon-hdr` | Vulkan's `VK_EXT_hdr_metadata` and its `ash` dependency exist only off Apple's platforms; Metal's HDR is the swapchain's colour space, which wgpu sets |
| `horizon-client/src/entry.rs` | `prepare_platform` sets `HORIZON_HOME` to `Documents`, `HORIZON_CACHE` to `Library/Caches` and makes the game folder the working directory |
| `horizon-client/src/game.rs` | the mobile lifecycle (`resumed`/`suspended`, the background flag). `Game::sync_ime` shows the keyboard through winit's `set_ime_allowed`, which is `becomeFirstResponder` on iOS |
| `horizon-client/src/game/touch.rs` | touch controls, on-screen buttons and the close button are the phone platforms' (Android and iOS) |
| `horizon-client-input` | no `arboard` clipboard (the game keeps its own), no "key without modifiers" (the soft keyboard has none) |
| `horizon-threads` | process memory through `task_info` (libproc's `proc_pidinfo` is macOS only), RAM through `hw:memsize` |

## Limitations

- **No controllers.** `controllers` (gilrs) is a desktop feature and the IPA is built without
  it. Horizon's touch controls and, over Bluetooth, a hardware keyboard work.
- **No native mods.** `.hnmod` libraries must be signed with the app, so only WebAssembly mods
  are offered (`game/prebuilt_mods.rs`).
- **The soft keyboard covers the window.** iOS does not shrink the view (and winit reports no
  keyboard height), so the chat is lifted by a fixed share of the screen while a field has focus
  (`game/ime.rs`, `SOFT_KEYBOARD`), rather than by the keyboard's real height.
- **No Java servers' profile page.** The name-and-skin screen is the Android activity's; iOS has
  no such page and shows no profile button.
