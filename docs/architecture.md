# Architecture

AssetCraft is a Cargo workspace of small crates. The engine — everything except
the two front ends — is GUI-agnostic, so the CLI and the future desktop app share
one implementation. This document describes the crate graph, the flow of a
couple of representative operations, and the decisions behind the shape.

## Crate graph

```
        assetcraft-cli        assetcraft-gui
        (binary)              (binary, M5)
             │  │                   │
             │  │                   │  (will depend on the same crates)
             │  └──────────┬────────┘
             ▼             ▼
   assetcraft-auth   assetcraft-catalog   assetcraft-download   assetcraft-library
             │             │                       │                    │
             └─────────────┴───────────┬───────────┴────────────────────┘
                                       ▼
                                assetcraft-core
                                       ▲
                                       │
                          assetcraft-dcc-blender
                          (adapter, M4)
```

Dependencies point one way, always toward `assetcraft-core`. Binaries depend on
service and core crates; service crates depend only on core. DCC adapters
implement behaviour defined against core types, so a new integration is additive
rather than a change to the engine.

| Crate | Kind | Responsibility | Milestone |
| --- | --- | --- | --- |
| `assetcraft-core` | lib | Domain model, config, error type, selection, asset structure | M0 |
| `assetcraft-catalog` | lib | Fab API client and response cache | M1 |
| `assetcraft-auth` | lib | Epic OAuth and token storage | M1 |
| `assetcraft-download` | lib | Download queue, transfer engine, manifest and chunk parsing, reassembly | M2 |
| `assetcraft-transfer` | lib | Asset id to files on disk: resolve, fetch chunks, reassemble, report contents | M2 |
| `assetcraft-library` | lib | Local library index | M3 |
| `assetcraft-dcc-blender` | adapter | Blender export | M4 |
| `assetcraft-dcc-godot` | adapter | Godot export | M6 |
| `assetcraft-update` | lib | Release feed, checksum verification, binary swap | M5 |
| `assetcraft-cli` | bin | Headless front end | M1 |
| `assetcraft-gui` | bin | egui desktop app | M5 |

## Representative flows

### Search (implemented, M1)

```
assetcraft search "mossy rock"
        │
        ▼
Config::load()                         platform dirs + endpoints + user agent
        │
        ▼
FabClient::from_config(&config)        builds a reqwest client
        │
        ▼
FabClient::search(&SearchQuery)        maps the query to Fab parameters
        │
        ▼
GET /i/listings/search?...             (no auth required)
        │
        ▼
wire::SearchResults                    raw, permissive serde types
        │
        ▼
convert::page_from_search              domain translation
        │
        ▼
Page<AssetSummary>                     printed, or emitted as JSON
```

The `wire` → `convert` split is the load-bearing idea: only `wire.rs` knows
Fab's JSON field names, and only `convert.rs` knows how they map to our model.

### Sign-in (implemented, M1)

```
assetcraft login
        │
        ▼
EpicAuth::authorization_url()          https://www.epicgames.com/id/api/redirect?clientId=…
        │
        ▼
   (user signs in in a browser and copies the code)
        │
        ▼
EpicAuth::exchange_code(code)
        │
        ▼
POST /account/api/oauth/token           Basic <launcher client id:secret>
        │
        ▼
EpicToken                               access + refresh, with an absolute expiry
        │
        ▼
DefaultTokenStore::save()               OS keyring, or an owner-only (0600)
                                        file where no secret service answers
```

### Download (implemented, M2)

```
DownloadFile { url, destination, size?, sha256? }
        │
        ▼
Downloader::download_all(files, on_progress)
        │   at most `concurrency` transfers in flight (a semaphore)
        ▼
per file: attempt loop (up to `max_attempts`)
        │
        ├─ .part exists? ──▶ GET with  Range: bytes=<len>-
        │                       ├─ 206 ──▶ append, hashing the prefix first
        │                       └─ 200 ──▶ server ignored the range; restart
        │
        ├─ 429/5xx/timeout ──▶ sleep base·2ⁿ (capped) and retry
        ├─ other non-2xx    ──▶ fail immediately
        │
        ▼
per chunk: BandwidthLimiter::acquire(len)  (one limiter shared by the batch)
        │
        ▼
verify size and SHA-256 ──▶ rename .part → destination (atomic)
```

Two invariants make this safe: work happens in a `.part` file so the
destination is never half-written, and a `.part` file is always a *prefix* of
the target, which is what makes resuming sound. A checksum mismatch deletes the
part file, because a corrupt file that survives would be resumed from and
reported as complete.

When `DownloadOptions::bandwidth` is set, the byte budget is shared by every
transfer in the batch rather than applied per file, so the cap is on the engine:
ten concurrent transfers still total the limit.

### Asset resolution and reassembly (implemented, M2)

The step that turns an owned asset into bytes: pick a format and file, ask Fab
for its signed download info, read the binary manifest that info points at,
fetch the chunks, and splice them back into files.

```
assetcraft download <id> --format fbx --quality 4K [--output dir] [--chunk-base URL]
        │
        ▼
FabClient::listing_formats(&id)         GET /i/listings/{id}/asset-formats (no auth)
        │
        ▼
core::select(&formats, &criteria)       Selection { format_code, file.id }
        │
        ▼
FabClient::download_info(id, fmt, file) GET …/files/{file}/download-info (auth)
        │                               ──▶ DownloadInfo { artifact_id, distribution_points }
        ▼
FabClient::fetch_bytes(point.manifest_url)      GET the signed manifest URL
        │
        ▼
Manifest::parse(&bytes)                 chunked manifest (magic 0x44BEC00C)
        │                               ──▶ chunks (to fetch) + files (chunk parts)
        ▼
Manifest::chunk_downloads(base_url)     each chunk's URL under the CDN base
        │
        ▼
Downloader::download_all(files)         the transport engine (M2)
        │
        ▼
Chunk::decode(&bytes)                   chunk file (magic 0xB1FE3AA2), zlib body
        │                               ──▶ decompressed chunk bytes, keyed by GUID
        ▼
Manifest::assemble(&store)              splice chunk parts, check each file's SHA-1
        │
        ▼
AssembledFile { filename, data }        written under the output directory
```

`Manifest` is the resolution result: `chunks` are the compressed blobs to
download, and `files` are the logical outputs, each cut from one or more
`chunk_parts` that point into chunks. Reassembly gathers the decompressed chunks
into a `ChunkStore` keyed by GUID, copies each part into its file's place, and
verifies the result against the SHA-1 the manifest records — the integrity check
the download engine's definition of done rests on. Chunk parts address the
*decompressed* chunk bytes, so a chunk file's header is stripped and its zlib
body inflated before splicing.

The one piece that is not confirmed against real data is the CDN base URL the
chunk paths hang off: the manifest carries each chunk's relative path but not
the host, and how Fab pairs a base with a manifest can only be confirmed against
a real signed-in response. Until then the transfer layer derives the base from
the directory of the signed manifest URL — the layout the documented examples
and the stub server use — and `download` takes `--chunk-base` as an override for
a response that pairs something else. `resolve` stops at printing the plan and
needs no base. With no `--output`, `download` writes to the location the library
recorded for the asset — or, the first time, to the standard asset directory —
which is what lets `assetcraft download <id>` alone re-download an owned asset.

The manifest and chunk parsers are faithful ports of the layout the wider Epic
tooling ecosystem documents, and their byte layouts are pinned by hand-built
fixtures in the tests rather than by writers we also wrote, so the readers are
checked against the specification. Neither has been validated against bytes from
a real Fab account, and encrypted manifests and chunks (`stored_as & 0x2`) are
rejected rather than guessed at. See the decision below.

### Format and quality selection (implemented, M2)

```
assetcraft formats <id> --format fbx --quality 4K --prefer largest
        │
        ▼
FabClient::listing_formats(&id)        GET /i/listings/{id}/asset-formats (no auth)
        │
        ▼
convert::formats_from_wire             AssetFormat { format_type, files }
        │
        ▼
core::select(&formats, &criteria)      pure: filter by code, quality text, platform, engine
        │
        ▼
Selection { format_code, format_name, file }
```

The chosen `file.id` is exactly what a download request needs; the step that
turns it into bytes is asset resolution, above.

### Asset structure: texture maps and LODs (implemented, M1 metadata)

What a listing does *not* say is how many levels of detail a model has or which
PBR maps it ships. The file list does, so that is what gets read.

```
assetcraft resolve <id>                manifest fetched, nothing downloaded yet
        │
        ▼
Resolved.structure                     AssetStructure::from_sized(manifest files)
        │
        ├─ assetcraft resolve           "Contents: 5 texture maps · 1 mesh (LODs 0-2)"
        └─ assetcraft structure <id>    the detail, per material and per level

assetcraft structure <id> --offline    the same answer from a downloaded asset
        │
        ▼
Library::download(&id) ──▶ collect files ──▶ AssetStructure::from_sized(paths + sizes)
   no sign-in, no connection
```

Two naming conventions carry the information, and `assetcraft-core::structure`
implements both: a **role token** at the end of a stem names the PBR slot a map
fills (`T_Rock_BaseColor`, `boulder_AO`), and a **level token** (`lod` plus
digits at a word boundary, anywhere in the name) says which level of detail a
file belongs to (`SM_Rock_LOD1`, `T_Rock_Roughness_LOD1`). A name can carry both,
which is why the role is read from the name with the level token removed: that
is the one place the two conventions interact, and it is why `plan` and
`AssetStructure` agree about what a file is.

Two limits are stated rather than hidden. A mesh that packs its levels inside one
file reports as a single level-less mesh, because file names are all that is
being read. And a name that follows neither convention is reported as it reads —
a texture with `TextureRole::Unknown`, a mesh with no level — rather than guessed
into a slot.

### Library (implemented, M3)

```
assetcraft library                    ──▶ fetch every page from /e/accounts/{id}/ue/library
                                            │
                                            ▼
                                     Library::insert(item)   INSERT OR IGNORE, so a repeat is a no-op
                                            │
                                            ▼
                                     library.sqlite          JSON item + denormalised title
                                            │
              assetcraft library --offline ──┤
             assetcraft library --search q ──┤── SELECT … ORDER BY title
              assetcraft library --verify  ──┤── read every item back and report what fails
             assetcraft library --repair   ──┴── realign stale titles, drop unreadable rows
```

Media is cached beside the index, not inside it. `assetcraft library
--thumbnails` fetches each item's thumbnail through the catalog client and
stores it in a content-addressed `MediaCache`, so offline browsing points at a
local file rather than a URL.

`Library::verify` only reports damage; `Library::repair` acts on it offline. A
row whose item reads back is kept and its denormalised `title` is realigned with
the item it was derived from, since the title is rebuilt only on insert. A row
whose item can no longer be parsed is dropped from the `assets` table — it can
never be shown, searched, or re-exported — but its `downloads` record is left in
place: the files are still on disk, and a later `assetcraft library` sync restores
the metadata row.

### Blender export (implemented, M4)

An owned asset on disk becomes a scene-ready material: lay the files out in a
standard tree, work out which map fills which slot, and emit a Blender script.

```
assetcraft export <id> --target dir [--run]
        │
        ▼
Library::download(&id)                  find the recorded download location
        │                               (or use an explicit --source)
        ▼
dcc::plan(root, id, format, files)      pure: sort into meshes/ textures/ other/,
        │                               classify each map, group into materials
        ▼
dcc::export(plan, source, options)      copy files into the standard layout,
        │                               write assetcraft_export.py
        ▼
dcc::material_script(plan, import)      render a version-tolerant Blender script
        │
        └─ --run ──▶ detect::find() ──▶ blender --background --python script
```

The planner reads only names, never bytes, so the whole layout is testable
without Blender or a network. `classify` (in `assetcraft-core`, which the next
adapter will reuse) maps a file name to a PBR role and the material it belongs
to; each role carries a semantic `Binding` that says how it wires into the
Principled BSDF — glossiness inverted into roughness, ambient occlusion
multiplied into base colour, normal and displacement through their nodes.
Keeping the binding semantic rather than a socket name is what lets the
script fall back across Blender's renamed sockets (`Specular` → `Specular IOR
Level`, `import_scene.obj` → `wm.obj_import`) without the planner knowing about
any of it.

The written export is three things: the files, the script, and — since the
addon landed — a manifest beside them.

### Importing from inside Blender (implemented, M4)

```
assetcraft export <id> --target dir
        │   export() writes: files, assetcraft_export.py, assetcraft_export.json
        ▼
blender_addon (sidebar panel, N)
        │
        ├─ exports.discover(root)       walk the preference's export library,
        │                               list every folder holding a manifest
        ▼
ASSETCRAFT_OT_import_export
        │
        └─ runpy.run_path(assetcraft_export.py)   run the export's own script
                                                in the live session — the
                                                same script --run hands to
                                                blender --background
```

`assetcraft_export.json` (written by `dcc::manifest`, camelCase, `/`-separated
export-relative paths) is the discovery and description contract: which asset
and format this is, which meshes import, which maps make which material. The
addon lists and describes exports from it and never opens the script — the
script is run, not parsed. Paths in both files are export-relative; the script
resolves them against its own directory at run time (the path baked at export
time is only the fallback for runners that omit `__file__`), so an export
copied or synced to another machine still imports from wherever it now lives.

Verification without a Blender binary: the crate's `addon_fixture` test (run
with `ASSETCRAFT_ADDON_FIXTURE` set) writes a real export; the addon's Python
tests then run its script against a stub `bpy` and assert it did exactly what
the manifest promised — same meshes, same importers, same materials, maps
loaded from the paths the manifest named. The CI `addon` job chains the two.

### Exporting from the app (implemented, M5)

```
detail panel, asset with a library download record
        │   Command::ExportBlender { id }            (background worker)
        ▼
Library::download(id)                 the recorded source dir and format —
        │                             the same record the CLI's export reads
dcc::collect_files + plan             <data>/exports/<id>/<format>, the same
        │                             sorted relative listing the CLI builds
dcc::export (run: false)              files, script, manifest — offline,
        │                             on a blocking thread, no network types
        ▼
blender_addon                         finds the export by its manifest; the
                                      default library root is this directory
```

One flow, two front doors: the CLI's `export` and the app's **Export to Blender**
button differ only in who clicks. The file listing (`collect_files`) moved into
the adapter crate so both build plans from the identical sorted input, and the
worker runs the whole thing with the network off — the M3 promise, now
reachable from the GUI as well as the terminal. The export root is
`Paths::exports_dir()`, and the addon defaults its browsed root to the same
directory (see the decision below), so nothing on either side needs configuring.

The app deliberately does not launch Blender. `--background` (the CLI's
`--run`) starts a throwaway instance that cannot touch the scene the user is
working in, and an interactive launch would block the worker for the lifetime
of the editor. Writing the export and letting the addon bring it into the open
scene is the hand-off that actually needs no terminal.

### Library download tracking (implemented, M3)

```
assetcraft download <id> … --output dir
        │   after the files verify and land in dir
        ▼
Library::record_download(id, dir, fmt)  INSERT … ON CONFLICT DO UPDATE
        │
        ▼
downloads(asset_id, path, format)       a second table, kept apart from the
                                        Fab-sourced item JSON
        │
        ├─ assetcraft library --offline  marks each item on disk [exportable]
        │                                (or [files missing] if the dir is gone)
        ▼
assetcraft export <id> --target dir     resolves the source and format here
```

`download` stores the canonicalised output directory, so the record survives a
different working directory. The path lives in its own table rather than inside
the `LibraryItem` JSON: local state stays out of the Fab-sourced record, so a
later `assetcraft library` sync (which is `INSERT OR IGNORE`) never overwrites
where a download landed, and a download can be tracked even before the item's
metadata has been synced.

The GUI's Local view reads membership from this table, not the index: a sync of
a large account lists hundreds of owned assets whose files were never fetched,
and "Local" that promises those is a view full of dead cards. What counts as
local is a `downloads` row whose path still exists — the same test the CLI
labels `[exportable]`. Collections draw on the index instead: they organise
what the account owns, downloaded or not.

### The app downloads from Purchased, not from the catalog (implemented, M5)

```
Home / Free card, signed in          Purchased card, signed in
        │  Download?                          │  Download
        ▼                                     ▼
"Downloads start from your          owned_for(client, account_id, card id)
 Purchased library", or the card           │  joins the card to the entry,
 it already is: Open in Purchased          │  takes its artifactId
                                             ▼
                                   POST /e/artifacts/{id}/manifest → chunks
```

Fab answers the artifact manifest for an asset the account *owns* — the library
endpoint is keyed by account id, and it is the only source of an `artifactId` —
so a catalog card has nothing to download even when it is signed in and the
button is green. Rather than offer a button that can only fail, the detail panel
puts the action where the route exists: the Purchased view downloads, a catalog
card the account owns says so and offers the jump (`Open in Purchased`, which
selects the same asset once its page has arrived), and one the account does not
own states where downloads start.

The resolution picker sits with the settings that still mean something rather
than next to a download: an artifact is the one build Fab chose to deliver, with
no files left to choose between.

### Favourites and collections (implemented, M5)

```
assetcraft-gui  heart              favourite / unfavourite
assetcraft-gui  Collections view   create / rename / delete collection
assetcraft-gui  detail panel       add to / remove from collection
        │
        ▼
favorites(asset_id)                                   the set of hearts
collections(id, name UNIQUE COLLATE NOCASE)           the names the user chose
collection_items(collection_id → collections ON DELETE CASCADE,
                 asset_id)                            the membership relation
```

These are the user's own organisation, kept beside the index they organise —
local state like download records: never Fab-sourced, never rebuilt by a sync,
and cleared by `Library::remove` along with the asset they describe. Names are
unique case-insensitively because two panels differing only in capitalisation
would defeat the point of organising. Membership is editable from the detail
panel rather than by drag-and-drop: a checkbox row against every collection
scales to a touch of keyboardless mouse work and needs no selection model, and
the list of collections is read once per detail frame.

### Settings (implemented, M5)

```
assetcraft-gui  Settings dialog  ──▶  settings.toml  ◀──  hand edits
        │                             (config_dir, TOML)          │
        └───── worker re-reads per download ◀─────────────────────┘
assetcraft download  reads it too: download_dir slots under
                     --output and the library record,
                     default_quality under --quality
```

Two fields — quality and download root — each with a live consumer on both
binaries; no more keys than those until something reads them. A missing file
is the defaults (a user who chose
nothing chose them); a file that cannot be parsed is an error naming itself and
saying to fix or delete it, because defaulting around a typo would quietly undo
the choices the user did make. The app reads the quality at startup to seed its
resolution picker and the download worker re-reads the file per download, so
hand edits land without either program being restarted or notified.

A third field, `auto_update`, has one consumer: the app, at startup. `None`
is the feature-list promise (it checks); `Some(false)` is the opt-out, and the
Settings dialog's checkbox writes it.

### Auto-update (implemented, M5)

```
assetcraft-gui at start (settings)          assetcraft update [--apply]
        │ Command::CheckUpdate                            │
        ▼                                                 ▼
        └──────────────► UpdateClient::check(Product) ◄──┘
                         GET Config::update_feed_url —
                         GitHub's Releases API, or a mirror
                         named by ASSETCRAFT_UPDATE_FEED
                                 │ plan_update(): a pure rule
                                 ▼
        GUI: banner "vX.Y.Z is available"  [Update] ──► Command::DownloadUpdate
        CLI: says so; --apply goes on
                                 │
                                 ▼
        download: recorded size must match, sha256 must equal the release's
        own `digest`; staged under cache/updates, discarded on any mismatch
                                 │
                                 ▼
        apply: copy beside the exe → rename exe→.old → rename aside→exe
               (the first rename is undone if the second fails)
                                 │
                                 ▼
        GUI offers Restart (spawn the path that is now the new binary,
        close this window); CLI says the next start is the new one
```

The release feed is GitHub's Releases API because that is the one endpoint
that carries a digest GitHub itself computed over the stored bytes — an
update verified against a digest the publisher did not choose is theatre, so
an asset without a parseable `sha256:` digest is never even offered. The
naming rule (`assetcraft-<cli|gui>-<target triple>[.exe]`, raw binaries
uploaded by `.github/workflows/release.yml`) means a running copy finds its
own replacement by exact name and can never install the other product's
binary or another platform's. Drafts are never offered; prereleases are only
ever offered to prereleases. `plan_update` is pure — release JSON in, chosen
asset out — so the whole rule is unit-tested, and the wire halves run against
httpmock fixtures; there are no live releases to test against yet, which is
the point of building the rule first and the workflow in the same breath.
Windows is why `apply` is three renames rather than one write: a running
image cannot be overwritten, but it can be renamed out of the way.

## Decisions

### We own the catalog client rather than wrapping `egs-api`

`egs-api` (MIT) already implements Epic auth and much of Fab, and it was used as
a reference for the endpoints. We still wrote our own client so that our domain
types are not dictated by another crate's public API, and so the dependency
surface stays small. That decision can be revisited if the maintenance cost of
tracking Fab outweighs the control.

### TLS comes from `native-tls`, not rustls

`reqwest` 0.13's default TLS is rustls with the `aws-lc-rs` provider, which
requires `cmake` at build time. That is not guaranteed to be present on
developer machines or minimal CI images. `native-tls` uses the platform's TLS
(OpenSSL, Schannel, Secure Transport) and needs no extra build tooling. If we
later want a pure-Rust build everywhere, the alternative is
`rustls-no-provider` plus the `ring` provider installed at startup.

### Tokens sit behind a `TokenStore` trait

The default backend (`DefaultTokenStore`) prefers the OS credential store —
the macOS Keychain, the Windows Credential Manager, the Linux D-Bus secret
service (over `zbus`, so there is no system build dependency) — via the
`keyring` crate. A keyring needs a running secret service, and CI, containers,
and headless Linux have none, so every keyring failure logs and falls back to
the private JSON file; sign-in therefore works wherever AssetCraft runs. A
file token read while the keyring answers cleanly is promoted into the
keyring and deleted from disk, so the file is a fallback and never a habit.
The trait keeps the whole question out of callers, and the store is tested
against a keyring double — the real keychain is checked by a manually run
`--ignored` smoke test, as it must not be written from CI.

### Offline and terminal-free is the plugin contract

Two milestone definitions of done are product promises, and DCC integrations
are where they are kept or broken:

- *With the network disabled, previously downloaded assets are fully browsable
  and re-exportable* (M3).
- *A non-developer can install AssetCraft, sign in, download an asset, and
  export it without touching a terminal* (M5).

For an adapter the first halves into a hard rule: **an adapter reads local
files and touches nothing else.** Its inputs are a source directory — by
default the location the library recorded when the asset was downloaded — and
its outputs are the laid-out files, the generated script, and optionally a
hand-off to a detected editor binary. No adapter fetches bytes, resolves
download info, or talks to a marketplace; that is `assetcraft-transfer` and
the catalog client's job, reached through sign-in, the CLI, or the GUI's
download queue. The pure-function shape of
`assetcraft_dcc_blender::{plan, export, material_script}` — no client, no
async, no network types in any signature — is what makes the rule checkable
by reading the signature, and the shared export trait
(`assetcraft-core::dcc::DccExporter`, landed with the second adapter in M6)
carries the same constraint through associated plan, options, and report types.

The second halves into the GUI rule: every adapter exposes exactly that
programmatic API so the desktop app can drive plan → layout → script →
editor, and `export --dcc <tool> --run` gives the terminal user the same
path. A plugin that only works from a console, or only with a live
connection to fetch its inputs, does not ship.

### Texture-role and LOD reading lives in core

Which PBR slot `T_Rock_BaseColor.png` fills, and which level of detail
`SM_Rock_LOD1.fbx` belongs to, is a fact about the asset rather than about
Blender — so `classify`, `lod_tag`, and the layout's `FileKind` live in
`assetcraft-core::structure`, and `assetcraft-dcc-blender` re-exports them. The
direction of the dependency decides this for us: an adapter depends on core and
never the reverse, so a reading that the browser, the Blender exporter, and the
next two adapters all need cannot live in the first of them. The Blender half
stays in the Blender crate: `Binding::for_role` maps a shared role onto a
Principled BSDF socket, and only Blender has one.

One reading serves both promises differently: at resolve time it describes an
asset from its manifest, before a chunk has been fetched; offline it describes
the files a previous download left behind. Adding an adapter should mean adding a
*rendering* of this structure, never a second parser for it.

### Login uses the authorization-code flow, not the device flow

Epic's OAuth device-authorization grant is not available to the public launcher
client: `POST /account/api/oauth/deviceAuthorization` returns `401
authentication_failed` for those credentials, while `client_credentials` on the
same client succeeds (verified 2026-10-16). The roadmap originally called for a
device flow; the browser authorization-code flow is the one that actually works
with a client we can legitimately use.

### The launcher credentials are not secrets

`LAUNCHER_CLIENT_ID` / `LAUNCHER_CLIENT_SECRET` are Epic's public *client*
credentials, embedded in every Epic Games Launcher installation. They identify
the application, not a user, and grant no access on their own — a user still has
to sign in. They are recorded here the same way they appear in other community
tools.

### A response cache keyed by URL, with the URL stored in the entry

`ResponseCache` keys entries by a dependency-free FNV-1a hash of the request URL
and stores the URL inside the entry. On read it verifies the stored URL matches,
so a hash collision degrades to a cache miss rather than serving the wrong body.

### The download engine was built before asset resolution

An asset's files arrive as a binary manifest, and parsing it correctly can only
be confirmed against a manifest produced by a real account. The transport engine
— concurrency, resumption, retries, integrity — has no such dependency: it can
be proven correct here, with tests, against a local server. So the engine was
built and tested first.

The parser then followed from the format the wider Epic tooling ecosystem
documents, rather than being guessed at: its byte layout is pinned by a
hand-built fixture written from that specification, not by a writer we also
wrote, so the reader is checked against the spec instead of against itself. The
chunk-file reader and the reassembler that follows it were built the same way,
against the documented chunk layout. Both fixtures were additionally parsed with
an independent reader from that ecosystem during development, which agreed with
ours. That is still a weaker guarantee than real bytes give, and it is stated
plainly rather than hidden: the readers are marked unvalidated against real Fab
data, and a first run against a real manifest and chunks is a validation step.
Encrypted manifests and chunks are rejected outright. This is the one place the
project ships readers it could not exercise end to end, and it says so in the
module docs, the ROADMAP, and here.

### Quality is matched by substring, not by a fixed tier list

A Megascans listing carries its quality tier in a file's name or asset type
(`…_4K.zip`, `High`), and the wording differs between sellers, so hard-coding a
tier vocabulary would encode guesses about data we have not seen. Selection
instead matches the requested quality as a case-insensitive substring of those
two fields, and platform and engine version as case-insensitive equality against
the file's lists. `--prefer largest|smallest` breaks ties by reported size, with
an unknown size ranked last in either direction. The result is the `file.id` a
download request needs; it is deliberately not a claim about what the tiers are
called.

### The bandwidth limit is a token bucket that may go into debt

A rate limiter has to hold the *aggregate* rate when several transfers run at
once, and the obvious design — sleep until the bucket has room, then take the
bytes — lets every waiter wake at the same moment and burst past the limit. So a
reservation is taken *before* the wait: the balance is allowed to go negative,
and the caller sleeps for however long that debt takes to clear at the rate. The
next caller sees the debt and waits longer, which serialises the throughput
without a lock held across the sleep. Capping refills at the burst size only
limits the positive side, so an idle period cannot bank throughput it never had.

### The Blender adapter writes a script rather than driving Blender

There are several ways to get a material into Blender: link against its C API,
talk to a running instance over a socket, or write a script and hand it to
`blender --background --python`. The last wins on every axis that matters here:
it needs no Blender at build time, it keeps the integration to one file the user
can read, diff, and re-run by hand, and it works whether Blender is run for them
(`export --run`) or by them. Version drift — renamed sockets, moved operators —
is absorbed by the script looking candidates up rather than by pinning a Blender
release in Rust.

### The addon runs the generated script instead of reimplementing it

The in-editor addon could build materials itself — it sits in Blender and has
`bpy` right there. It does not. The exporter already generates the complete
recipe per export, and a second implementation of the material graph in Python
would be a second place for the layout to be wrong and a second thing to keep
version-tolerant. So the addon's only Blender act is `runpy.run_path` on the
export's own script: identical results through the terminal (`export --run`)
and through the sidebar, and the addon keeps no material logic to drift.

That leaves the question of what the addon *does* need from the export: to
list exports and describe them without running anything. For that the exporter
writes a manifest (`assetcraft_export.json`) beside the script, and the
reading lives in `blender_addon/assetcraft_blender/exports.py` — deliberately
bpy-free so the manifest contract is testable (and CI-tested) without a Blender
to test against. The manifest is written last, after files and script, so a
half-written export never announces itself to the panel; a manifest whose
version the addon does not recognise is skipped rather than guessed at.

### The export root is a cross-language contract, tested on both sides

An export the app writes has to be findable by the addon without either
program talking to the other or the user copying a path — so they share one
location, computed twice: by Rust's `directories` (`Paths::exports_dir()`,
under the project's platform data directory) and by a Python function in the
bpy-free `exports.py` that mirrors `directories`' rules for the project
identity `dev.AssetCraft.AssetCraft`. Those rules are not as obvious as they
sound — Linux lower-cases the application name, macOS uses the dotted bundle
identifier, Windows nests organisation and application under a `data`
segment — so the agreement is pinned by a matching pair of tests: the Rust one
asserts the trailing segments `directories` resolves to, the Python one the
segments the mirror produces per platform. Change either side and the two
start disagreeing loudly.

The addon's preference still overrides the default: pointing it at some other
export root (a CLI `--target`, an external drive, a synced folder) is the
supported way to browse exports the app did not write.

The mirror has to find the *host's* data directory even when Blender itself is
sandboxed — on Linux that usually means Flatpak's `org.blender.Blender`. The
sandbox redefines `XDG_DATA_HOME` to `~/.var/app/org.blender.Blender/data`,
which is precisely the variable the mirror would otherwise trust, and the app
never writes there. So `exports.py` asks `/.flatpak-info` — the file every
sandbox mounts at its root — whether it is inside one, and inside a sandbox it
resolves the XDG *default* from the real home instead, which the sandbox mounts
as-is (Flathub's Blender grants `filesystem=host`, the same grant that makes
the exports readable at all). A host `XDG_DATA_HOME` relocated outside the
home directory is invisible from any sandbox under any rule; the preference is
the answer there, and both READMEs say so. `detect::find()` needed no flatpak
special case: a Flatpak install exports a `org.blender.Blender` wrapper into
the system and per-user `flatpak/exports/bin` directories, and running that
path is the supported way to launch it, headless arguments included.

### The Godot adapter re-renders the Blender layout, and shares its trait

The second adapter (M6) is the test of whether the first one was a layout
implementation or a Blender implementation. It answers: `assetcraft-dcc-godot`
calls the same `plan`, `classify`, and channel dedupe the Blender adapter
exported — the standardized `meshes/`/`textures/`/`other/` tree, material
grouping by name, and the roughness/glossiness channel rule are read once and
rendered twice. Only the *rendering* is Godot's: an `ORMMaterial3D` built on
the occlusion/roughness/metallic texture packed from the AO (R), roughness (G),
and metallic (B) maps, with the conventional 1.0/0.5/0.0 defaults when a source
map is absent; a `StandardMaterial3D` with separate maps and scalars when
nothing can be packed; a `.tscn` that instances the meshes and attaches the
generated GDScript; and glTF preferred over FBX/OBJ when the asset carries it,
recorded in the report so the caller sees which path was taken.

What both adapters share is the trait, not the code: `DccExporter` in
`assetcraft-core::dcc` names `name`, `plan`, `export`, `material_script`,
`scene`, and `detect`, with associated `Plan`, `Options`, and `Report` types
because a Blender run produces a Principled BSDF graph and a Godot run produces
a scene and a packed texture. The signature still has no client, no async, and
no network type in it — which is the whole point of putting it in core.

The Godot plugin mirrors the Blender addon's division of labour rather than its
language: `godot_plugin/assetcraft_godot/exports.gd` is the editor-free reader
(manifest validation, the same `dev.AssetCraft.AssetCraft` export-root mirror,
Flatpak's `/.flatpak-info` check, and a bounded walk that never descends into
an export), and `plugin.gd` is the dock that copies a chosen export into
`res://assetcraft_imports/` and opens its scene. The export directory ships its
own `project.godot`, so it can also be opened directly to inspect the result.
Godot 4.4.1 runs in CI for this much: the Rust exporter writes a fixture, the
plugin scripts are parse-checked, and the reader is exercised headlessly
against that fixture. The import dock waits for Godot's resource scan before
opening the scene, and asks before replacing an existing import. Whether the
viewport looks right is still a manual check, as it is for Blender.

### The artifact Fab delivers is the newest engine build, not the first listed

`projectVersions[]` is one entry per engine target, and each carries its own
`artifactId` — the token the artifact manifest takes. Fab's order is not
chronological: "European Hornbeam" arrives as UE 5.0, then UE 5.1 v2, then UE
4.27, and "Quixel Bridge" arrives with nine builds in scattered order from 5.1
to 5.8. Taking the first entry, which is what a `find_map` does, silently
delivers a five-year-old build of an asset the user owns a current one of; on
this account 62 of 434 owned assets are ordered oldest-first.

So the catalog takes the newest build *that has an artifact*, ranked by the
highest `engineVersions[]` entry it covers, parsed as a number pair because
`UE_5.10` outranks `UE_5.9` and a string compare says the opposite. A build with
no artifact cannot win, because Fab will not serve it; a build that names no
engine release ranks below one that does, and among equals Fab's own order
decides. The chosen id is stored on the item, and `assetcraft library` prints it
— with 434 assets, "what will this actually deliver" has to be answerable
without opening the database by hand.

A stored item carries the choice its sync made, so `assetcraft library` refreshes
existing rows as well as adding new ones (`Library::refresh`): the artifact of
the build Fab delivers today beats the one it named at the first sync, and the
download's path is a different table for exactly that reason. Most of that
refresh is Fab's churn rather than real change — the library rotates
`image_previews` URLs on a large share of assets per request — which is why the
sync says "refreshed from Fab" rather than "changed".

### The library stores an item as JSON, with one denormalised column

The `assets` table holds the whole `LibraryItem` as JSON, plus a copy of the
title. The JSON is the source of truth, so adding a field to the domain model
cannot silently drop data that is already stored — either the item reads back
exactly, or `verify` reports the row. The title is duplicated only because it is
what we sort and search on, and SQLite can index and order a column but not a
JSON path as cheaply.

Search uses `instr(lower(title), …)` rather than `LIKE`, so a query containing
`%` or `_` matches those characters literally instead of acting as a wildcard.

### Cached media is content-addressed by URL, with the URL kept alongside

The thumbnail cache names each image after a hash of its URL, so re-caching an
item is a no-op and two items that share a preview share one file. A hash alone
could in principle collide, so each image has a sibling `<key>.url` file holding
the source URL; a read whose stored URL does not match the requested one is a
miss rather than the wrong picture. The sidecar is written after the image, so a
half-finished entry is simply refetched. The image itself stays a valid file on
disk, which keeps the cache browsable without the tool.

## Endpoint reference

All Fab paths are relative to `https://www.fab.com`.

| Purpose | Method | Path | Auth |
| --- | --- | --- | --- |
| Search listings | GET | `/i/listings/search` | no |
| Listing detail | GET | `/i/listings/{uid}` | no |
| Asset formats | GET | `/i/listings/{uid}/asset-formats` | no |
| License types | GET | `/i/taxonomy/licenses` | no |
| Tag groups | GET | `/i/tags/groups` | no |
| My library | GET | `/e/accounts/{account_id}/ue/library?count=&cursor=` | yes |
| Artifact manifest | POST | `/e/artifacts/{artifact_id}/manifest` | yes |

Epic account service: `https://account-public-service-prod03.ol.epicgames.com`,
token endpoint `/account/api/oauth/token`.

Update feed: `https://api.github.com/repos/cmspeedau/AssetCraft/releases?per_page=100`
(Releases API, no auth, IP-rate-limited; GitHub demands a `User-Agent`, which
every AssetCraft client already sends). Assets are fetched from the
`browser_download_url` each release entry carries.

### The two vocabularies for one listing type

Search filters by slug (`?listing_types=3d-model,material`), and any other value
is a `400`: the accepted set is `3d-model`, `material`, `decal`,
`tool-and-plugin`, `audio`, `vfx`, `animation`, `ui`, `game-system`, and
`game-template` — `3d-scene` and `3d-plant` are Fab's *website* categories, not
filter values. The library endpoint names the same things for people instead
(`"3D"`, `"Tools & Plugins"`, `"Material & Textures"`) and adds labels with no
search slug at all (`"Tutorials & Examples"`, `"Legacy"`), so an owned entry
carries the label it was given and `assetcraft-gui::data::listing_slug_for_label`
maps it onto the filter's slug. Labels with no slug keep the card untyped, which
costs it the type filter and nothing else.

## Testing

- **Wire decoding** is tested against responses captured from the live API, so a
  field-name regression fails a test rather than reaching a user.
- **HTTP behaviour** is tested with `httpmock`, exercising real request building
  and response handling without touching the network.
- **Local state** (token permissions, cache round-trips, library de-duplication)
  is tested against temporary directories.
- **The binary manifest parser** is tested against a hand-built fixture whose
  bytes are written from the documented layout and whose section sizes are
  asserted against hand-computed values, so the reader is pinned to the
  specification rather than to a writer we also wrote.

Tests must never reach the network. Fab's JSON API sits behind Cloudflare and
answers automated clients from data-centre addresses with a challenge page, so a
test that depended on it would be flaky by construction — and would misreport a
bot filter as a code failure.
