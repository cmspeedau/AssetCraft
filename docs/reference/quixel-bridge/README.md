# Quixel Bridge — UI Reference

Screenshots of the original **Quixel Bridge** desktop app (2021–2022 builds) and
the **Bridge for Unreal Engine 5** panel, kept as the visual target for
AssetCraft's desktop UI ([`crates/assetcraft-gui`](../../crates/assetcraft-gui)).

> **License / attribution.** All screenshots are © Epic Games / Quixel AB,
> extracted from Epic's official public documentation
> (*"Quixel Bridge 2022 — Official documentation"* PDF, which also contains the
> *Bridge for Unreal Engine 5* guide). They are included here for
> interoperability reference only. AssetCraft is an unofficial project and is
> not affiliated with Epic Games or Quixel. These images must never ship inside
> an AssetCraft build.

## Sources

| Source | What we took |
|---|---|
| [Quixel Bridge 2022 official documentation (PDF)](https://assets-unreal2-epic-prod-us2.s3.dualstack.us-east-1.amazonaws.com/original/4X/b/6/1/b61c946c01dc89154b79a9790cdd7314cbe978ea.pdf) | Standalone app screenshots (setup, library, import, asset panel, download/export settings, about) |
| Same PDF, ch. *Bridge for Unreal Engine 5* | UE5 dock-panel variant, folder layout |
| [Quixel Bridge plugin for UE — Epic docs](https://dev.epicgames.com/documentation/unreal-engine/quixel-bridge-plugin-for-unreal-engine) | Tab/navigation behavior notes (Home, Collections, MetaHumans, Favorites, Local) |

## Application anatomy

Bridge is a three-pane window (≈1920×1080 default, resizable):

```
┌────┬──────────────────────────────────────────────┬───────────────┐
│    │  [ search ............................ ]  🔔  👤│               │
│ L  ├──────────────────────────────────────────────┤  asset detail │
│ e  │  All Types ▾  All Sizes ▾  All States ▾  🎨 ▾ │  right panel  │
│ f  ├──────────────┬──────────────┬───────────────  │  (opens on    │
│ t  │  ┌────────┐  │  ┌────────┐  │  ┌────────┐    │   asset click,│
│    │  │ thumb  │  │  │ thumb  │  │  │ thumb  │    │   ~420 px,    │
│ n  │  │  badge │  │  │  badge │  │  │  hover │    │   scrollable: │
│ a  │  ├────────┤  │  ├────────┤  │  ├────────┤    │   preview     │
│ v  │  │ name   │  │  │ name   │  │  │ ♥  ⤓   │    │   + info      │
│    │  └────────┘  │  └────────┘  │  └────────┘    │   + download/ │
│ t  │              │              │                │   export bar  │
│ r  │  (virtualized grid, infinite scroll)          │               │
│ e  ├──────────────────────────────────────────────┴───────────────┤
│ e  │  downloads drawer / status                                   │
└────┴──────────────────────────────────────────────────────────────┴────┘
```

- **Left nav (≈200 px):** app logo; primary tabs with item counts (Home,
  Collections, MetaHumans\*, Favorites, Free, Purchased, Acquired, Local); the
  `Local`/purchased views expose a secondary category tree (Megascans →
  3D Assets / 3D Plants / Surfaces / Mixes / Imported; Home exposes asset-type
  categories: 3D Assets, 3D Plants, Surfaces, Decals, Atlases, Imperfections,
  Brushes). Footer: website link + settings.
- **Search bar (top, full width):** rounded, dark `#2E2E2E`; supports typed
  filters as removable chips (`free ×`, `In Local | Megascans ×`); type-ahead
  suggestions dropdown; Enter → full results.
- **Filter bar:** dropdown chips `All Types / All Sizes / All States /
  All Environments / All Colors` + toggleable filter bar icon; color filter
  renders as swatch chips.
- **Grid:** square thumbnails in responsive columns, `FREE`/`NEW` pill badges
  (top-left), favorite heart + download-on-hover icons, marquee/ctrl multi-select
  with checkboxes and a shared Export/Download bar, card label strip shows name.
- **Right panel (≈420 px):** preview area with 3 modes (Renders / Maps /
  3D preview) + zoom viewer; asset name, info-icon row (type, size, tileable,
  assembly…); tags; variations grid; related assets & related collections;
  bottom action bar: **resolution dropdown + green Download** and Export
  settings gear.
- **Downloads:** bottom drawer listing transfers with green progress bars;
  progress also renders as overlay on the grid card.

## Screen index

| Image | Screen | Notes for AssetCraft |
|---|---|---|
| `01-setup-library-path.jpg` | First-run wizard: library path | step 1 of setup; folder picker + `NEXT` |
| `02-setup-export-target.jpg` | First-run wizard: export target | DCC dropdown + `DOWNLOAD PLUGIN` |
| `03-settings-preferences.jpg` | Preferences (General/Privacy/Legal) | Library Path, API port, usage data |
| `10-home-browse.jpg` | **Main Home screen** (primary mock) | full 3-pane: nav, filter bar, categories row, asset grid |
| `11-browse-color-filter.jpg` | Grid + active color filter | swatch chips + filtered results |
| `12-browse-color-filter-strip.jpg` | Color filter strip close-up | chip sizing |
| `13-browse-local-tabs.jpg` | Local tab: External/Mixes/Downloaded/Purchased | local library views |
| `14-toolbar-search.jpg` | Toolbar + search + signed-in state | top bar anatomy |
| `20-import-side-panel.jpg` | Import custom assets (side panel) | name/type/category/maps + preview pickers |
| `21-asset-details-annotated.jpg` | Asset right panel (annotated in source) | official field-callout image |
| `22-maps-fullscreen.jpg` | Maps fullscreen viewer | `Surface 178 · Render 1/3`, zoom, map cycling |
| `23-asset-right-panel.jpg` | Asset right panel: variations/maps/related | scrollable right pane layout |
| `24-related-assets.jpg` | Related-assets row | `NEW` badge, ♥, hover ⤓, name strip |
| `25-asset-context-menu.jpg` | Card context menu | Download Again / Report Issue / Go to Files / Copy Asset ID |
| `30-download-settings.jpg` | Global download settings (long form) | per asset-type defaults |
| `31-download-settings-2.jpg` | Download settings: textures/models tabs | map set + LOD checkboxes |
| `32-export-bar.jpg` | Multi-select export view | checkboxes + `Export` button on hover |
| `33-bottom-download-bar.jpg` | Hover download bar | `Download Settings \| Export Settings` + resolution + green `DOWNLOAD (n points)` |
| `40-export-target-vertical.jpg` | Export target panel (vertical) | target + plugin list |
| `41-multi-select.jpg` | Multi-asset grid selection | shared cost/points panel |
| `42-usd-export-steps.jpg` | USD/MDL export flow | assembly selected, right panel |
| `43-export-target-tab.jpg` | Export Settings ▸ TARGET tab | Custom Disk Export + location |
| `44-export-textures-tab.jpg` | Export Settings ▸ TEXTURES tab | format, per-map channels, naming rule |
| `45-export-models-lods.jpg` | Export Settings ▸ MODELS tab | FBX/USD, LOD checkboxes, normal maps, sources |
| `46-channel-packed-maps.jpg` | Channel packing | Normal X/Y/Z → packed map, RGB 8-bit |
| `50-ue5-editor-overview.jpg` | Bridge docked in UE5 | panel-in-host layout |
| `51-ue5-login.jpg` | Sign-in in host panel | Epic login popup |
| `52-ue5-panel-home.jpg` | Panel Home: icon rail + curated banner | narrow-rail variant of left nav |
| `53-ue5-nav-tree.jpg` | Panel left nav tree | tab tree over grid |
| `54-ue5-asset-grid.jpg` | Panel grid + selected asset | green check = acquired/downloaded |
| `55-preferences-privacy.jpg` | Preferences: library path + privacy | usage-data toggle |
| `56-ue5-export-settings.jpg` | Host-panel export settings | tabbed form, master material |
| `57-ue5-download-in-editor.jpg` | Download → editor | placeholder → final asset |
| `58-downloaded-asset-previews.jpg` | Downloaded preview cards | check overlay + name strip |
| `59-folder-structure.jpg` | Library folder layout on disk | see below |
| `60-about-dialog.jpg` | About dialog | version + launcher update |

## Design tokens (sampled from screenshots)

| Token | Value | Used for |
|---|---|---|
| `bg-app` | `#252525` | window/grid background |
| `bg-panel` | `#171717`–`#1B1918` | right panel, insets, hover states darker |
| `bg-elevated` | `#2E2E2E` | search field, bars, dialogs |
| `bg-header` | `#3A3A3A` | title strip, table headers |
| `fg-primary` | `#E6E6E6` | titles (`SMALL ROCK` ≈ 24 px) |
| `fg-secondary` | `#B6B4B0` | body text, nav labels |
| `fg-muted` | `#6E6E6E` | counts, secondary metadata |
| `accent-green` | `#91B759` / hover `#9DC75F` | **Download**, progress, FREE badge |
| `accent-blue` | `#4FA4E3` | primary buttons (`NEXT`, `DOWNLOAD PLUGIN`), links, logo |
| `accent-blue-soft` | `#74A4C0` | `NEW` pill, wizard highlight text |
| `border` | `#3A3A3A` @ ~50% | card separators, input outlines |

Rounded corners are small (4–8 px). Typography is a neutral grotesque
(Roboto-like); card names ≈13 px, section headers ≈15 px uppercase tracking.
Grid columns are fluid: `floor(grid_width / card_min)` with a card minimum of
~240 px and 8–12 px gaps. The right panel overlays the grid (grid reflows only
in the host-panel variant).

## Library folder layout (on-disk contract, `59-folder-structure.jpg`)

```
<LibraryPath>/
  Assets/<Category>/<Subcategory>/<AssetName>/   # 3DAssets, Surfaces, Imperfections, Decals,
    Geometry/  Material/  Maps/  Metadata/  Pictures/   # lowpoly, textures, JSON, preview
  Mixes/  External/  support/                     # Mixer mixes, imports, plugin downloads
```

## Interaction patterns worth copying

1. **Instant preview before download** — streaming/low-res 3D + maps view from
   the CDN; download only on explicit intent.
2. **Hover-reveal actions** (download ⤓ / ♥) keep the grid clean; badge + name
   strip are always visible.
3. **Inline download progress** on the card itself + a global downloads drawer.
4. **Typed search filters as chips** (`In Local | Megascans ×`) compose with
   the dropdown filter bar.
5. **Marquee + checkbox multi-select** with one shared export/download action.
6. **One settings dialog, tabbed per concern** (General / Privacy / Legal;
   per-export: Target / Textures / Models / Filenames).
7. **Context menu ≈ "local file operations"** (Go to Files, Copy Asset ID) —
   maps directly onto our `assetcraft-library` crate.

## AssetCraft mapping (assetcraft-gui, egui)

| Bridge concept | AssetCraft target |
|---|---|
| Left nav tabs + counts | `SidePanel` with view enum: Home, Collections\*, Favorites, Purchased, Local (M6 host-panel variant = icon rail) |
| Category tree | recursive `CollapsingHeader` from catalog taxonomy |
| Search + chips | top `TextEdit` + chip strip bound to catalog query facets |
| Filter bar | horizontal dropdown row (types/sizes/states/biomes/colors) |
| Grid | custom `load`-based virtualized grid, thumbnail cache, hover overlays |
| Right panel | `Window`/split-pane: 3 preview tabs, info icons, related lists, action bar |
| Download drawer | bottom panel fed by `assetcraft-download` events |
| Settings/Export dialogs | modal with tabs mirroring Target/Textures/Models/Filenames |
| Local view | `assetcraft-library` repair/verify features already implement this data model |
