# SimpleMiniMap

A minimap mod for Palworld (v1.0.5, UE 5.1.1). It is a native UE4SS C++ mod whose logic lives in Rust.

- `shim/` is a thin C++ layer built against UE4SS. It exports `start_mod`/`uninstall_mod`, registers hooks and hands Rust a table of host functions (`shim/include/smm_host.h`).
- `core/` is a Rust `staticlib` linked into the same `main.dll`. All minimap logic is here.
- `deps/RE-UE4SS` is a git submodule pinned to `2281fa311e417b1dfddedbcd49972d764fddb244`, the exact UE4SS build the Palworld Steam Workshop package ships (v3.0.1 Beta). C++ mods must be built against the UE4SS version the game runs, so if the Workshop UE4SS updates, move the submodule and rebuild.

## Working on this project

- Read the user's global `~/.claude/CLAUDE.md` and memory first.
- Research before writing code. Confirm every class, function, parameter and enum value in the SDK dump, and read how the engine or a reference mod handles it. Every in-game test costs the user a launch, so never ship a guess. When the cause is unknown, gather evidence first (log lines, screenshot crops) and say what is known and what is not.
- Batch several researched changes into one build, then ask for one test.
- Only read-only git commands. Give the user any other git command to run.
- Defaults must work without the user setting anything up.
- Pre-release: no migration code for old settings files.
- No em dashes, no semicolons in prose, minimal comments (one line, only for constraints the code cannot show).

## Build and deploy

From the repo root (approved, run it after changes):

```
cmake -S . -B build -G "Visual Studio 17 2022"
cmake --build build --config Game__Shipping__Win64 --target SimpleMiniMap
```

The build runs `cargo build --release` for `core/`, links it into `main.dll` and copies it to `C:\Program Files (x86)\Steam\steamapps\common\Palworld\Mods\NativeMods\UE4SS\Mods\SimpleMiniMap\dlls\main.dll`. The copy fails while the game is running ("Error copying file"). The build itself still succeeded, so tell the user to close the game and deploy again.

Changing the host interface: `SMM_HOST_API_VERSION` in `smm_host.h` and `API_VERSION` in `core/src/host.rs` must match, and the field order of `SmmHostApi` must match `HostApi` exactly. Remove host functions that end up unused.

## Testing and evidence

- Log: `...\Palworld\Mods\NativeMods\UE4SS\UE4SS.log`, lines tagged `[SimpleMiniMap]`. Every 10 s: `frame budget: N fps, our cost X us average, Y us worst` and a timing tree (`profile.rs`). Time new work with `profile::time("name", || ...)` or `profile::scope("name")`.
- SDK dump: `...\Palworld\Mods\NativeMods\UE4SS\CXXHeaderDump\` (`Pal.hpp`, `Engine.hpp`, `Engine_enums.hpp`, `UMG.hpp`, `PPSkyCreatorPlugin.hpp`, `WBP_*.hpp`). A function is declared on one class: resolve it on that class or a subclass, never on a base class.
- Screenshots: ffmpeg is installed. Crop and enlarge regions (`crop=...,scale=...:flags=neighbor`) into the scratchpad to inspect edges. No Python imaging library.
- User settings: `...\Mods\SimpleMiniMap\settings.ini`.

## Architecture of `core/`

- `lib.rs`: entry points, frame budget, key handling, actor event queue, panic logging.
- `host.rs`, `ue.rs`: the host table, `Obj`, `Weak` (engine weak pointer), `Function` (resolves a UFunction and its parameter offsets by name, calls ProcessEvent, reads out-parameters and struct fields with `param`/`param_field`), map and data-table helpers.
- `game.rs`: per-frame player state: world, controller, pawn, location, yaws, game screen open (`PalHUDService.IsAnyOverlayUIActive`), screen fading (`IsAnyFadeWidgetActive`, teleports, sleep, cutscenes), in base, inside a stage (`PalUtility.IsInsideStage`). The minimap hides for screens, fades and (optionally) the base.
- `minimap.rs`: markers, culling, badges, chevrons, edge pins, zoom animation, icon scaling, the update loop. The biggest file.
- `layers.rs`: everything inside the minimap box: shape strips, map and fog images, marker canvases, terrain tile canvases, rim, view cone, generated textures.
- `terrain.rs`: rendered terrain tiles (below).
- `markers.rs`, `actors.rs`, `objects.rs`, `icons.rs`: location markers, world actors, world-wide map object models, icon tables.
- `badges.rs`: pal badges and chevrons, cached as PNG in `Mods\SimpleMiniMap\cache\`.
- `assets.rs`: background asset loads (`KismetSystemLibrary.LoadAsset`, polled, direct load after 5 s).
- `menu.rs`, `widgets.rs`, `keys.rs`: the Alt+J settings window, its UMG toolkit, and key polling with `GetAsyncKeyState`.
- `settings.rs`: `settings.ini`, written with defaults on first run. Missing keys are added automatically. Pickup item ids keep their spelling and match case-insensitively.
- `worldmap.rs`: map regions (main island, Sky Island "Tree") and world-to-UV.

## Engine rules learned the hard way

- Everything touching the engine runs on the game thread in the `EngineTick` post hook. That hook runs after `UGameEngine::Tick` has drawn the frame, so a change made around a scene capture and undone right after is never seen by the game's own view.
- Never hold raw UObject pointers across frames. Use `Weak`. Fast travel destroys actors without a world change.
- Render targets from `CreateRenderTarget2D` are only weakly held. Keep them alive by putting them on an image widget (the collapsed store canvas in `layers.rs`) or as a component's `TextureTarget`.
- Bitfield bools need `ue::set_bool_property` (objects) or `ue::set_struct_bool` (struct values like `FPostProcessSettings`).
- One missing UFunction makes the whole `new()` that resolves it fail, which disables that module silently apart from a warning. Optional extras go in their own resolve step.
- GPU readbacks (`ReadRenderTarget`) stall the game thread, 5 to 50 ms. Use them once, never per frame or per tile.
- The map textures (`T_WorldMap`, `T_TreeMap`) stream, and a UMG image never requests their sharp mips. `keep_full_resolution` forces them resident.
- Markers and tiles are anchored to fractions of the map layer, so a zoom resize moves them with no per-marker calls. `move_map` runs before the cull, which checks the map's shape at its current place.
- Rotation: the minimap's layer turns, icons counter-turn. Offsets drawn next to an icon (chevrons) must be turned by the icons' counter-turn too.
- Dungeons can sit inside the main map region, under the overworld (one was 390 m below sea level). Detect them with `IsInsideStage`, and inside a stage only show markers within 50 m of the player's height.
- The game's icon widgets need their state set after `Setup`: `SetEnable(IsEnableFastTravel)` on teleporters and bases, `UpdateIcon` on towers, `UpdateBossIconState` on bosses. Refreshed once a second while shown.
- A player character within 4 m of the player is the local player (riding and gliding can make your character a different actor from the pawn).

## Rendered terrain (`terrain.rs`)

- 256 m tiles at 512 px, doubling per level up to 4 km so 3 or 4 tiles cover the view. Each tile is captured once by an orthographic `SceneCaptureComponent2D` on the pawn and kept for the session (64 at most). A capture happens only for a missing tile, at most every 0.75 s, never while the minimap is hidden, and not in the 2 s after a teleport. Not cached on disk (that needs a readback).
- Settings: `OverworldTerrain` and `DungeonTerrain`, each `Off`, `On` or `Close` (400 m or less across). Dungeon tiles use 10 m height bands with the camera just above the band, so roofs drop out, and are dropped on each new entry.
- Survey before each capture: 5 by 5 rays down across the tile. All must hit (the ground is streamed in), and the surface camera goes 30 m plus 4% of the tile width over the highest hit, so nothing is cut off.
- Fixed lighting, so tiles match at any time of day. During each capture only: the sky actor (`APPSkyCreator`) sun, moon and sky light are set to intensity 0 (fast update, no shadow rebuild), its volumetric clouds and height fog are hidden, its weather particles are excluded, and two map lights on the pawn are shown (key from the north-west at 50 degrees, fill from above at 60%, no shadows). Exposure is manual, Lumen GI and reflections are off. Once per world the map lights are calibrated from one readback so the ground lands at a normal brightness.
- Capture source `SCS_SceneColorSceneDepth`, the only lit source whose alpha comes out opaque.
- Known limits, not bugs to chase:
  - UE 5.1 orthographic views have no real camera position (view origin W = 0), so anything measured from the camera (foliage cull distances, distance fades) is measured from the world origin. Switching foliage culling off per capture was tried and removed: it cost 3 to 6 ms per capture, and grass blades are vertical cards that are edge-on from straight above anyway, so it only added dark bush blobs.
  - Cliff and hill meshes (`M_PalLit_Top`) look flat from above: their top grass texture is a small tiling pattern that blurs at 0.5 m per pixel, while the landscape's texture stays busy. Higher tile resolution is the only lever.
  - Lakes that take their colour from reflecting the sky are see-through, because the sky light is off for captures. The sea colours itself and shows. A possible route is setting the game's time to noon with `APPSkyCreator::SetTime` around each capture, untested.
  - The game's lamps still light tiles captured near them at night.
- Removed approaches, do not retry: re-rendering every frame or on a timer (PalMiniMap's cost), a "something overhead" cave mode (an upward trace, set off by trees and overhangs, cut hills flat), a flat `SCS_BaseColor` look (no water, no shading), camera height relative to the player (cut hilltops), frozen auto exposure with the game's own lighting (tiles differ with time of day and shadow cascades).

## Open work

- Performance pass from the timing tree. Known spikes: badge bakes (readbacks), the map icon class poll (about 30 ms once a second).
- Releases: `.github/workflows/release.yml` runs on a `v*.*.*` tag that must match the version in `core/Cargo.toml`, `ModVersion` in `shim/src/dllmain.cpp` and `workshop/Info.json`. It builds with Ninja and `SMM_DEPLOY=OFF`, publishes `SimpleMiniMap-<version>.zip` (a `SimpleMiniMap` folder with `dlls\main.dll` and `enabled.txt`) as a GitHub release, and uploads to the Steam Workshop (Palworld app 1623730) with steamcmd. Release notes come from the version's section in `CHANGELOG.md`. Changes go under `## [Unreleased]`, and `./scripts/bump-version.ps1 <version>` sets the version in all of them and turns that section into the release's. The Workshop item is built from `workshop/` (`Info.json`, `thumbnail.png`, and `description.txt` in Steam BBCode, with no double quotes or backslashes). Keep `README.md` and `workshop/description.txt` in step with features, keys and defaults. The user runs git.

## Reference material

- PalMiniMap (Lua): `C:\Users\salah\Downloads\PalMiniMap 3915 2.3.8 2026-08-03T13-45Z CPBp3l6nl\Pal\Binaries\Win64\ue4ss\Mods\PalMiniMap\Scripts`. Useful for engine facts and pitfalls, not for approach: its performance is poor. Read it, do not copy it.
- Versys Minimap (Lua, all rights reserved, research only): `C:\Users\salah\Downloads\Versys Minimap V1.1.337 5430 1.1.337 2026-09-15T14-13Z rDhLqo6nh\VersysMinimap`.
