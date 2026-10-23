# AssetCraft — Roadmap

This document tracks where AssetCraft is going and in what order. It is a
working plan, not a contract: dates are deliberately omitted, and sequencing can
change as we learn. See [README.md](README.md) for what the project is.

## Current status

- **M0 — Foundations:** complete.
- **M1 — Catalog & Auth:** in progress. Public catalog reads, sign-in, token
  storage and refresh, the response cache, and the authenticated library and
  download-info endpoints are in. The OS keyring backend remains.
- **M2 — Download Engine:** in progress. The transport engine (bounded
  concurrency, resumption, retries, integrity verification, a global bandwidth
  limit), the persistent queue, format and quality selection, the binary
  manifest parser, and chunk reassembly (decode a chunk file, splice its parts
  into the files, verify each file's SHA-1) are all in. What remains is the CDN
  base URL Fab pairs with the manifest, which has to be confirmed against a real
  signed-in response.
- **M3 — Library:** in progress. A SQLite library backs offline browse, search,
  an integrity check, a thumbnail cache, and repair. A downloaded asset's
  location is recorded, so previously downloaded assets are browsable and
  re-exportable by id with the network off. Re-download from the library remains.
- **M4 — Blender Export:** in progress. A Blender adapter lays an asset's files
  out in a standardized PBR tree, classifies each map into a Principled BSDF
  material graph, and writes a version-tolerant Python script; `export --run`
  hands it to Blender, and Blender is detected per OS. An installable in-Blender
  addon remains.

## Vision

A fast, dependable, cross-platform tool that makes a photogrammetry asset
library feel local: search it, fetch exactly the quality and formats you need,
and get finished materials into your DCC in one click — without a heavyweight
launcher and without lock-in.

## Guiding principles

1. **Engine first, UI second.** Core and service crates stay GUI-agnostic so the
   same engine drives the desktop app and the CLI.
2. **No lock-in.** Assets land in a standardized, documented folder layout; the
   local library index is portable and readable on its own.
3. **Offline-friendly.** Anything you have cached or downloaded remains browsable
   and re-exportable without a network connection.
4. **Respect the platform.** Use Fab's official APIs with the user's own account,
   honor the ToS, and never redistribute asset content.
5. **Small and quick.** Native, low-memory, fast startup — a deliberate contrast
   to Electron-class tools.

## Milestones

Each milestone has a version target and a **definition of done (DoD)**. A
milestone is complete only when its DoD is fully checked.

### M0 — Foundations → `v0.1.0`

Stand up the workspace so contributors can build, test, and trust the toolchain.

- [x] Cargo workspace with all crates from the README architecture section
      scaffolded (compiling stubs are fine).
- [x] `rust-toolchain.toml` pinning a stable toolchain.
- [x] CI: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` on
      Windows, macOS, and Linux.
- [x] `assetcraft-core`: domain model skeleton, shared `Error` type, config
      loading, and structured logging (`tracing`).
- [x] Dual license files present (`LICENSE-APACHE`, `LICENSE-MIT`).
- [x] `docs/architecture.md` describing the crate graph and data flow.

**DoD:** `cargo build --workspace` and `cargo test --workspace` pass on all three
platforms in CI.

### M1 — Catalog & Auth → `v0.2.0`

Read the library. This is the first milestone that talks to Fab.

- [x] Fab API client with search, pagination, and filters (type, category, sort,
      free/discounted, seller).
- [x] Sign-in with Epic, storing the token behind a `TokenStore` (a private file
      today; see `docs/architecture.md` for why the OS keyring is deferred).
- [x] Transparent refresh: commands resolve a usable token before calling an
      authenticated endpoint.
- [x] On-disk response cache for public reads, with a time-to-live.
- [x] Asset metadata model covering previews, categories, formats, and download
      locations.
- [x] Authenticated library and download-info endpoints, including the Fab CSRF
      handshake they require.
- [ ] OS keyring token backend.
- [ ] Texture-map and LOD metadata (lands with the download engine, M2).

> **Note on the device flow.** The original plan called for an OAuth *device
> flow*. Epic's device-authorization endpoint answers `401` for the public
> launcher client while `client_credentials` on the same client succeeds, so the
> browser authorization-code flow is what ships.

**DoD:** `assetcraft login` succeeds on a clean machine and `assetcraft search`
lists real results with metadata against the live Fab API.

> **Verification note.** The live Fab JSON API sits behind Cloudflare and answers
> data-centre clients with a challenge, so the live end-to-end check is a manual
> step rather than a CI one. Everything up to the network boundary is covered by
> mocked tests.

### M2 — Download Engine → `v0.3.0`

Get bytes on disk, reliably.

- [x] Concurrent downloads with a bounded worker pool.
- [x] Resumable transfers: a partial file is a prefix, resumed with a range
      request, and survives a restart.
- [x] Retry with exponential backoff, capped, and clear failure reporting.
      Client errors are not retried.
- [x] Checksum and size verification on completion, in a `.part` file that is
      renamed into place only once it verifies.
- [x] A persistent, de-duplicating download queue.
- [x] Resolve an owned asset into the files to fetch: parse Fab's signed binary
      artifact manifest (Epic's chunked manifest format, magic `0x44BEC00C`).
- [x] Reassemble files from chunks: decode each chunk file (magic `0xB1FE3AA2`),
      splice its parts into place, and verify each file against its SHA-1.
- [x] Quality-tier and format selection per asset.
- [x] Global bandwidth limit.

> **Ordering note.** The transport engine and the manifest parser were split.
> The engine can be proven correct with tests against a local server; the
> manifest and chunk parsers were written from the format the wider Epic tooling
> ecosystem documents and are pinned by hand-built fixtures, but they have not
> been validated against bytes from a real account, so a first run against real
> data is a validation step. Format and quality selection sits between them: it
> works over the format metadata the catalog already returns, so it did not have
> to wait for the parser. `assetcraft download` ties resolution, chunk fetching,
> and reassembly together, but it needs the CDN base URL passed in with
> `--chunk-base` until a real signed-in response confirms how Fab derives it.
> See `docs/architecture.md`.

**DoD:** A large asset can be downloaded, interrupted mid-transfer, resumed to
completion, and verified — with no corruption.

> **Reaching the DoD.** Every piece is in place and tested in isolation: the
> engine resumes and verifies transfers, the manifest names the chunks, and
> reassembly checks each output file's SHA-1. The one gap is the CDN base URL
> the chunk paths hang off, which `assetcraft download` takes as `--chunk-base`
> until a real signed-in response confirms how Fab supplies it.

### M3 — Library → `v0.4.0`

Own what you have, even offline.

- [x] SQLite-backed local library index.
- [x] De-duplication by asset id: syncing the same asset twice is a no-op.
- [x] Offline browse of cached metadata (`assetcraft library --offline`,
      `assetcraft library --search <text>`).
- [x] Library integrity check (`assetcraft library --verify`).
- [x] Thumbnail and preview cache (`assetcraft library --thumbnails`).
- [x] Offline browse marks which assets are on disk and re-exportable
      (`assetcraft library --offline` shows each item's download location).
- [x] Re-export a previously downloaded asset straight from the library:
      `download` records where an asset's files land, and
      `assetcraft export <asset-id> --target <dir>` finds them by id and lays
      them out with the network off.
- [x] Repair damaged entries, rather than only reporting them
      (`assetcraft library --repair` realigns stale titles and drops rows whose
      stored item can no longer be read, leaving their download record in place).
- [ ] Re-download straight from the library — the exporter is ready, but a fresh
      fetch still needs the CDN base URL M2 has not confirmed (see M2).

**DoD:** With the network disabled, previously downloaded assets are fully
browsable and re-exportable.

> **Reaching the DoD.** Offline browse already covers the "browsable" half, and
> re-export now covers the rest: a download records its output directory in the
> library, `export` resolves an asset from that record, and the Blender adapter
> lays out the files and writes the import script — none of which touches the
> network. A *re-download* is the one action that cannot happen offline, so it
> stays open with the CDN base URL that gates it.

### M4 — Blender Export → `v0.5.0`

The first real integration.

- [x] Standardized PBR folder layout emitted for every asset (`meshes/`,
      `textures/`, `other/`, with name collisions disambiguated).
- [x] Blender exporter that builds a correct PBR material graph from the maps
      (roles classified by name, wired into a Principled BSDF; glossiness
      inverted, AO multiplied, normal and displacement routed through nodes).
- [x] A generated, version-tolerant Blender Python script that builds the
      materials and, unless disabled, imports the meshes into the active scene
      and assigns the material — `assetcraft export --run` hands it to Blender.
- [ ] An installable in-Blender addon for one-click import from inside the
      editor (the generated script is the bridge; the addon is a follow-on).
- [x] Per-OS Blender path detection (Linux, macOS, Windows, plus `PATH`).
- [x] Export from the CLI (`assetcraft export`) and as a programmatic API
      (`assetcraft_dcc_blender::{plan, export, material_script}`).

**DoD:** `assetcraft export <asset-id> --dcc blender` produces a scene-ready,
correctly-textured material in Blender.

> **Reaching the DoD.** The layout, role classification, and material graph are
> pure and covered by tests, and the generated script is inspected by tests for
> the sockets and imports it emits. Blender itself is not in CI, so the final
> "does it look right in the viewport" check — and the CDN base URL still needed
> to fetch the bytes in the first place — remain manual validation steps.

### M5 — GUI & Packaging → `v1.0.0`

Make it a product.

- [x] egui desktop shell: browse, search, filter, asset detail, driven by the
      live catalog client (with an offline demo catalog).
- [x] Download queue wired to the download engine: resolve, fetch, reassemble,
      verify, and record in the library, with phase progress in the app.
- [x] Local library view: assets on disk, read from the library database, with
      their paths in the detail panel.
- [x] Owned library view: the signed-in account's Fab library, paged.
- [ ] Collections view and settings.
- [x] Login and account status inside the app (Epic launcher flow, token shared
      with the CLI).
- [ ] Installers/artifacts for Windows (`.msi`/`.zip`), macOS (`.dmg`), Linux
      (`.AppImage`/`.deb`).
- [ ] Auto-update.
- [ ] Onboarding docs and screenshots in the README.

**DoD:** A non-developer can install AssetCraft, sign in, download an asset, and
export it to Blender without touching a terminal.

### M6 — Godot Export → `v1.1.0`

The second DCC, and the first that shows the shared export trait earns its keep.

Godot is a different target from Blender in three ways that shape the work: it
wants **`.glb`/`.gltf`** (glTF is Godot's first-class, no-reimport path) rather
than FBX/OBJ into a Python graph; it builds materials from a **`.gd` script or a
`.tscn`/`.tres` resource** rather than a Principled BSDF node tree; and it packs
occlusion-roughness-metallic into a **single ORM texture** rather than wiring
separate maps. M6 reuses M4's layout and role classification and re-expresses
them in Godot's terms.

- [ ] `assetcraft-dcc-godot` adapter crate implementing the shared export trait,
      reusing the M4 PBR layout (`meshes/`, `textures/`, `other/`) and the same
      name-based role classification rather than re-deriving either.
- [ ] A generated, version-tolerant Godot script (GDScript) that builds a
      `StandardMaterial3D` (or `ORMMaterial3D`) per classified material — albedo,
      metallic, roughness, emission, normal — and applies it to imported meshes.
- [ ] ORM packing: combine the AO, roughness, and metallic maps the classifier
      finds into the single occlusion/roughness/metallic texture Godot expects,
      falling back to `StandardMaterial3D` scalars when a map is absent.
- [ ] Mesh path: prefer glTF (`.glb`/`.gltf`) as the lossless import Godot reads
      without a reimport step; fall back to the FBX/OBJ a Fab format already
      provides, recorded so the caller sees which was used.
- [ ] A scene export: a `.tscn` (text scene) referencing the material and meshes,
      so `--run` yields a scene the user can open rather than only a script.
- [ ] `assetcraft export --dcc godot` wired into the CLI; `export --run` hands
      the generated script/scene to a detected Godot (headless `--headless
      --script` for the script path), mirroring `export --run` for Blender.
- [ ] Per-OS Godot path detection (Linux, macOS, Windows, plus `PATH`), reusing
      the M4 detection shape.
- [ ] Programmatic API: `assetcraft_dcc_godot::{plan, export, material_script,
      scene}` alongside the Blender equivalents, so the GUI (M5) drives both.
- [ ] An installable in-editor Godot plugin (`editor_plugin`, GDScript) for
      one-click import from inside the editor (the generated script is the
      bridge; the plugin is a follow-on, as with the Blender addon).

**DoD:** `assetcraft export <asset-id> --dcc godot` produces a scene-ready,
correctly-textured `StandardMaterial3D`/`ORMMaterial3D` in Godot.

> **Reaching the DoD.** The layout and role classification are already pure and
> tested in M4; the new surface is the GDScript/`.tscn` emission and ORM packing,
> which are likewise pure and covered by tests inspecting the script and scene
> they emit and the ORM channels they write. Godot itself is not in CI, so — as
> with Blender — the "does it look right in the viewport" check, and the CDN base
> URL still needed to fetch the bytes, remain manual validation steps.

## Cross-cutting tracks

Ongoing, not tied to a single milestone:

- **Performance** — startup time, memory ceiling, download throughput.
- **Accessibility & UX** — keyboard navigation, theming, clear error states.
- **Internationalization** — externalize strings ahead of community translations.
- **Testing** — integration tests against recorded/mocked API responses.
- **Docs** — keep README, ROADMAP, and `docs/` current with the code.

## Non-goals

- Redistributing, mirroring, or reselling asset content.
- Bypassing Fab licensing, entitlement, or paywalls.
- A general-purpose 3D editor or renderer.
- Targeting the deprecated legacy Bridge service as a primary backend.

## How to shape this roadmap

- Propose changes via an issue or pull request against this file.
- Tell us which DCC you need next — that directly orders the DCC milestones
  (Blender in M4, Godot planned in M6).
- Milestone DoD checkboxes are the source of truth for "done"; keep them honest
  when closing work.
