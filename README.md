# AssetCraft

A cross-platform, open-source asset browser, downloader, and DCC exporter for
Megascans / Fab PBR assets — written in Rust.

AssetCraft is a ground-up reimplementation of the [Quixel Bridge](https://quixel.com/)
desktop app: browse and search a photogrammetry asset library, download the
quality tier and file formats you need, and push finished materials straight into
your content-creation tools. It is built for speed, a small footprint, and clean
cross-platform behavior on Windows, macOS, and Linux.

> **Status: pre-alpha.** The workspace builds, the CLI can search the Fab
> catalog and sign in to your account, and CI runs on Linux, macOS, and Windows.
> There is no desktop UI yet. See [ROADMAP.md](ROADMAP.md) for what lands next,
> or jump to [Building from source](#building-from-source) to try the CLI.

---

## Disclaimer

AssetCraft is an **unofficial, community project**. It is not affiliated with,
endorsed by, or supported by Epic Games, Quixel, or Fab.

- Assets are served by **Fab**, Epic's marketplace. AssetCraft talks to Fab's
  official APIs using **your own** Epic/Fab account; it does not ship, resell, or
  redistribute any asset, and it cannot grant you access to content you have not
  licensed.
- You are responsible for complying with the [Fab Terms of Service](https://www.fab.com/)
  and the license attached to each asset you download.
- The original **Quixel Bridge** application is deprecated; Fab is now the
  supported source for Megascans content. AssetCraft targets Fab first.

---

## Table of contents

- [Features](#features)
- [Architecture](#architecture)
- [Installation](#installation)
- [Quick start](#quick-start)
- [Configuration](#configuration)
- [Supported DCCs](#supported-dccs)
- [Building from source](#building-from-source)
- [Project layout](#project-layout)
- [Contributing](#contributing)
- [License](#license)
- [Acknowledgements](#acknowledgements)

---

## Features

**Planned.** Nothing here is implemented yet; this is the target surface for
`v1.0`.

### Catalog

- Search and filter the Fab/Megascans library by category, tags, type, and
  resolution.
- Rich asset detail: preview thumbnails, LODs, texture maps, license, and
  download options.
- Offline browsing of the metadata you have already cached.

### Downloads

- Concurrent, **resumable** downloads with retry and backoff.
- Selectable quality tiers and file formats per asset.
- Checksum verification and integrity checks on every completed download.
- Reassemble Fab's chunked artifacts and verify each output file against its SHA-1.
- Bandwidth limits and a persistent download queue.

### Library

- Local SQLite index of everything you own, independent of the app that fetched it.
- Deduplication by asset ID, with re-download and re-export from the library.
- Thumbnail and preview cache for fast, offline browsing.

### Integrations

- Standardized PBR folder layout (base color, normal, roughness, AO, etc.).
- One-click export to DCC tools, starting with **Blender**.
- A headless CLI for scripted and CI workflows.

### App

- A fast, native **egui** desktop UI with a small install footprint.
- Cross-platform builds for Windows, macOS, and Linux.
- Auto-update.

See [ROADMAP.md](ROADMAP.md) for the order in which these ship.

---

## Architecture

AssetCraft is a Cargo workspace of small, focused crates. The core and service
crates are GUI-agnostic, so the same engine powers both the desktop app and the
headless CLI.

| Crate | Kind | Responsibility |
| --- | --- | --- |
| `assetcraft-core` | lib | Domain model, traits, configuration, and error types. |
| `assetcraft-auth` | lib | Epic/Fab OAuth device flow and secure token storage (OS keyring). |
| `assetcraft-catalog` | lib | Fab API client: search, filters, metadata, and response cache. |
| `assetcraft-download` | lib | Concurrent, resumable download engine with integrity checks, manifest and chunk parsing, and reassembly. |
| `assetcraft-library` | lib | Local SQLite library index, thumbnails, dedup, offline browse. |
| `assetcraft-dcc-blender` | adapter | Blender exporter and Python addon bridge. |
| `assetcraft-cli` | bin | Headless binary for scripted / CI workflows. |
| `assetcraft-gui` | bin | egui desktop app: browse, search, filter, and open asset details. |

Dependency direction is one-way: binaries depend on the service and core crates,
never the reverse. DCC adapters implement a shared export trait defined in
`assetcraft-core`, so new integrations are additive.

A more detailed diagram and design notes will live in `docs/architecture.md`
(planned).

---

## Installation

> Not available yet — no release has been published. This section documents the
> intended install paths.

### Prebuilt binaries

Once `v0.1` ships, grab an installer for your platform from the
[Releases](https://github.com/cmspeedau/AssetCraft/releases) page:

| Platform | Artifact |
| --- | --- |
| Windows | `.msi` / portable `.zip` |
| macOS | `.dmg` (universal) |
| Linux | `.AppImage` / `.deb` |

### From source

Requires a recent stable Rust toolchain (see [Building from source](#building-from-source)):

```bash
git clone https://github.com/cmspeedau/AssetCraft.git assetcraft
cd assetcraft
cargo install --path crates/assetcraft-cli
```

---

## Quick start

Working today:

```bash
# Sign in to your Epic/Fab account (opens a browser for an authorization code)
assetcraft login

# Browse the catalog without signing in
assetcraft search "rock cliff" --type surface --limit 20
assetcraft taxonomy licenses
assetcraft taxonomy tags

# Inspect an asset's formats, and pick the file you want
assetcraft formats <asset-id>
assetcraft formats <asset-id> --format fbx --quality 4K --prefer largest

# Resolve an owned asset into the chunks and files to fetch (requires sign-in)
assetcraft resolve <asset-id> --format fbx --quality 4K --chunks

# Download and reassemble it, verifying every file (requires sign-in).
# The CDN base the chunk paths hang off is passed in with --chunk-base until a
# real signed-in response confirms how Fab supplies it.
assetcraft download <asset-id> --format fbx --quality 4K \
  --output ./Downloads --chunk-base https://cdn.example.com/CloudDir

# Sync the assets you own into the local library (requires sign-in)
assetcraft library

# Browse it offline, search it, or check it for damage
assetcraft library --offline
assetcraft library --search "rock"
assetcraft library --verify

# Cache thumbnails so the library browses offline
assetcraft library --thumbnails

# Lay a downloaded asset out as a Blender PBR material and write an import
# script. `download` records where the files landed, so with no --source the
# export resolves them from the library and works with the network off; pass
# --source to export a directory directly, or --run to hand the script to a
# detected Blender.
assetcraft export <asset-id> --target ./MyProject
assetcraft export <asset-id> --source ./Downloads --format fbx --run

# Fetch a single file, resuming and verifying as it goes
assetcraft fetch https://example.com/asset.zip --output asset.zip --sha256 <hash>

# Cap the download rate (bytes per second; `k`/`M` suffixes are accepted)
assetcraft fetch https://example.com/asset.zip --output asset.zip --limit 2M

# Launch the desktop app (`--demo` uses the offline catalog)
assetcraft-gui
```

---

## Configuration

AssetCraft follows platform conventions for config, cache, and data directories
(via the `directories`-style project paths):

| Path | Contents |
| --- | --- |
| Config | Settings, default quality tier and format, DCC paths. |
| Cache | API response cache, thumbnails, partial downloads. |
| Data | Local library index (SQLite) and downloaded assets. |
| Tokens | OAuth credentials, stored in the OS keyring — never on disk in plaintext. |

The exact file format is TBD (TOML is the working assumption).

---

## Supported DCCs

| Tool | Status |
| --- | --- |
| Blender | CLI export works (`assetcraft export`): standard PBR layout + generated import script. In-editor addon planned. |
| Plain folder export | Available (the layout `export` writes) |
| Godot | Planned — next DCC (roadmap M6): glTF meshes, `StandardMaterial3D`/`ORMMaterial3D` from a packed ORM texture, generated GDScript/`.tscn`. |
| Unreal Engine | Under consideration |
| Maya | Under consideration |
| Houdini | Under consideration |
| Unity | Under consideration |

Want a DCC on this list? Open an issue describing your workflow.

---

## Building from source

### Prerequisites

- **Rust**: stable toolchain via [rustup](https://rustup.rs/). The pinned
  `rust-toolchain.toml` selects the channel and the `rustfmt`/`clippy`
  components automatically.
- **Git**.

TLS uses the platform's native stack (OpenSSL on Linux, Schannel on Windows,
Secure Transport on macOS), so on Linux you also need the OpenSSL development
headers (`libssl-dev` on Debian/Ubuntu).

Platform-specific tooling for a native egui window (M5) and packaging will be
documented per-OS as it is wired up.

### Build

```bash
cargo build --workspace            # debug
cargo build --workspace --release  # optimized
cargo test --workspace             # run tests
cargo clippy --workspace -- -D warnings
```

---

## Project layout

```text
assetcraft/
├── Cargo.toml              # workspace manifest
├── crates/
│   ├── assetcraft-core/        # domain model, traits, config, errors
│   ├── assetcraft-auth/        # Fab OAuth + token storage
│   ├── assetcraft-catalog/     # Fab API client + metadata cache
│   ├── assetcraft-download/    # download engine
│   ├── assetcraft-library/     # local library index
│   ├── assetcraft-dcc-blender/ # Blender exporter
│   ├── assetcraft-cli/         # headless binary
│   └── assetcraft-gui/         # egui desktop app
├── docs/                   # architecture and design notes
├── ROADMAP.md
└── README.md
```

---

## Contributing

Contributions are welcome. See the [roadmap](ROADMAP.md) for where the project
is and what is next.

Before opening a pull request:

1. Run `cargo fmt --all`.
2. Run `cargo clippy --workspace --all-targets -- -D warnings`.
3. Run `cargo test --workspace`.
4. Keep changes focused and match the surrounding style.

For larger features — especially new DCC integrations — open an issue to discuss
the approach first. Roadmap items are tracked in [ROADMAP.md](ROADMAP.md); feel
free to propose changes to it.

---

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you, as defined in the Apache-2.0 license, shall
be dual-licensed as above, without any additional terms or conditions.

---

## Acknowledgements

- **Epic Games / Quixel / Fab** for the Megascans library and the marketplace
  APIs this project builds against.
- The **Rust** ecosystem: [egui](https://github.com/emilk/egui),
  [reqwest](https://github.com/seanmonstar/reqwest),
  [tokio](https://github.com/tokio-rs/tokio),
  [sqlx](https://github.com/launchbadge/sqlx), and the many crates this project
  depends on.
