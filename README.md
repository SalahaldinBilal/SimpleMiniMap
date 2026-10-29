# SimpleMiniMap

A minimap for Palworld, built as a native UE4SS mod so it costs next to nothing per frame.

Made for Palworld v1.0.5 with the UE4SS build from the Palworld Steam Workshop (v3.0.1 Beta).

## Features

- A corner minimap that uses the game's own map, its explored-area clouds and its icons.
- Rendered terrain: the ground around you is captured from above once per area and kept, so it is far more detailed than the map art. It works in the overworld and in dungeons, where roofs are left out so you can see the rooms.
- Markers for fast travel points, bases, dungeons, towers, bosses, quests, warps, oil rigs, supply drops, treasure maps, death drops and pins.
- Markers for wild, shiny and own pals, humans and other players, with pal portraits and an above or below arrow when they are on another level.
- Markers for chests, eggs, effigies, notes, junk, ore, skill fruit trees, lotuses, forage, fishing spots and items lying on the ground. Every type can be switched on or off.
- Landmarks (fast travel points, bases, dungeons, towers, bosses, quests and warps) stay readable when zoomed out, and the nearest ones off the minimap are pinned to its edge.
- Smooth zoom, icons that scale with zoom, a separate zoom for dungeons, and an optional auto zoom that widens the view while you move fast.
- North-up or rotating with the camera, a view cone, any corner, any size, square to round.
- Hides itself on game menus, during teleports and loading fades, and (optionally) inside your bases.
- An in-game settings window.

## Install

The easy way is to subscribe to SimpleMiniMap and UE4SS Experimental (Palworld) on the Steam Workshop.

To install by hand:

1. Install UE4SS Experimental (Palworld) from the Steam Workshop if you do not have it.
2. Download the zip from the latest GitHub release.
3. Extract its `SimpleMiniMap` folder into `Palworld\Mods\NativeMods\UE4SS\Mods\`.
4. Start the game.

## Keys

| Key | Action |
| --- | --- |
| Alt+J | Open the settings window |
| Alt+N | Show or hide the minimap |
| Alt+= or Alt+Numpad Plus | Zoom in |
| Alt+- or Alt+Numpad Minus | Zoom out |
| Alt+K | Megazoom on or off (a wide view, 1500 m by default) |

You can rebind every key in the settings window or in `settings.ini`.

## Settings

Most settings are in the window that Alt+J opens, and they save as you change them. They are also kept in `Mods\SimpleMiniMap\settings.ini`, which is written on first run with a comment above each option. Settings added in a new version are added to the file automatically.

Some defaults worth knowing:

- Rendered terrain is on in the overworld and in dungeons. Each one can be set to Off, On, or "When zoomed in close" (400 m or less across).
- The view is 220 m across, and 120 m in dungeons.
- Respawn points and your own pals are hidden. So are very common pickups (wood, stone, small pal crystals, berries, eggs, arrows, leather and wool).
- The minimap hides inside your bases.

## Notes

- Rendered terrain uses fixed lighting, so tiles match at any time of day. Lakes that get their colour from reflecting the sky can show through to the lake bed, and lamps can brighten tiles captured near them at night.
- Pal portraits are generated once and cached in `Mods\SimpleMiniMap\cache\`. Deleting that folder is safe.

## Building

Requires Visual Studio 2022 (C++ workload), CMake and Rust (stable, MSVC toolchain).

```
git clone --recursive https://github.com/SalahaldinBilal/SimpleMiniMap
cmake -S . -B build -G "Visual Studio 17 2022"
cmake --build build --config Game__Shipping__Win64 --target SimpleMiniMap
```

`deps/RE-UE4SS` is a submodule pinned to the UE4SS build the Workshop package ships. A native mod has to be built against the UE4SS version the game runs, so when the Workshop UE4SS updates, the submodule needs to move to match.

The build copies `main.dll` into the game's mod folder at the default Steam path. Close the game first, or the copy fails.

`shim/` is a small C++ layer over UE4SS, and `core/` holds the minimap itself, written in Rust.
