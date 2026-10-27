<img src="assets/icon.svg" width="64" height="64" alt="Tap icon">

# Tap

A small desktop autoclicker written in Rust with a native Slint interface. Warm neutral colors, a compact fixed-size window, and just the controls you need.

## Download

Get the Windows executable or Linux AppImage from [Releases](https://github.com/PawWin/autoclicker/releases/latest).

- **Windows x64:** double-click the downloaded `.exe`. No installation required.
- **Linux x64:** make the `.AppImage` executable (`chmod +x Tap-*-linux-x86_64.AppImage`), then run it. If FUSE is unavailable, add `--appimage-extract-and-run`.

## Run from source

Install [Rust](https://rustup.rs/) 1.92 or newer, then:

```sh
cargo run --release
```

The executable is `target/release/tap-autoclicker` (`.exe` on Windows). The first build downloads and compiles Slint; later builds are incremental. On Linux you need a working graphical session, fontconfig, and XKB common libraries. No browser or web server is involved.

## Use

1. Set an interval from 10 to 60,000 milliseconds, or choose a preset.
2. Choose the left, right, or middle mouse button.
3. Repeat until stopped, or choose **Count** and enter the number of clicks.
4. Press **Start clicking**, then move your cursor to the target during the three-second countdown.
5. Press **F6** to start or stop from another app, where supported. **Stop clicking** always works in the app; **Escape** stops while the app has keyboard focus.

Clicks follow the current cursor position. The counter resets for each session. Settings are locked while running. Closing the window stops the worker. Intervals are best-effort and include input backend overhead; this is not a precision timing tool.

## Platform support

- **Windows:** native input injection and global F6 shortcut.
- **macOS:** input injection requires granting Accessibility permission to the app or launching terminal in System Settings. Global shortcuts may also require permission.
- **Linux / X11:** input through the Rust X11 backend; global F6 shortcut.
- **Linux / Wayland:** experimental input through Enigo's Wayland backend, requiring compositor support for virtual pointer protocols. Global F6 is disabled because X11 hotkeys cannot control native Wayland applications. Use the in-app Stop button. On unsupported compositors input can fail or be limited to XWayland applications; use an X11 session for full support.

The app reports shortcut availability, invalid settings, and input errors in the window. Release 0.0.1 was built locally: Linux on an isolated Ubuntu 22.04 userspace, and Windows through cargo-xwin targeting MSVC. Both packages were checked for startup on Linux (the Windows executable under Wine). Native Windows interaction has not been verified. macOS builds are not included.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

`ui/app.slint` defines the interface. `src/main.rs` connects callbacks, global shortcuts, and UI updates. `src/engine.rs` owns the click worker and interruptible timing. Engine tests use an injected click function and never inject real mouse input.

Built with [Slint](https://slint.dev/) and [Enigo](https://github.com/enigo-rs/enigo). This project declares GPL-3.0-only; review Slint's licensing terms before distributing under other terms.

## Release builds

An optional GitHub Actions workflow is included at `.github/workflows/release.yml`; pushing a `v*` tag triggers it when Actions is available. It builds a Windows x64 portable executable with an embedded icon and no console window, and a Linux x64 AppImage on Ubuntu 22.04. Both jobs run formatting, Clippy, and engine tests. Once both succeed, the workflow publishes the binaries and SHA-256 checksums to the tagged GitHub release. The version in `Cargo.toml` must match the tag, and release notes must exist in `releases/<version>.md`.

The workflow can also be started manually to build artifacts without publishing a release.

### Build releases locally

On Ubuntu 22.04, install the development packages listed in the workflow, plus `file`, then run:

```sh
cargo build --release --locked
bash packaging/appimage.sh
```

For a Windows executable from Linux, install Clang, LLD, LLVM tools, and [cargo-xwin](https://github.com/rust-cross/cargo-xwin), then run:

```sh
rustup target add x86_64-pc-windows-msvc
cargo xwin build --release --locked --target x86_64-pc-windows-msvc
mkdir -p dist
cp target/x86_64-pc-windows-msvc/release/tap-autoclicker.exe dist/Tap-0.0.1-windows-x86_64.exe
```

The Windows build embeds the icon and statically links the C runtime. Published executables are unsigned. GitHub Actions is not required to build or upload a release.
