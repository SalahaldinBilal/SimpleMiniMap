# SimpleMiniMap

A minimap for Palworld, built as a native UE4SS mod so it costs next to nothing per frame.

Made for Palworld v1.0.5 with the UE4SS build from the Palworld Steam Workshop (v3.0.1 Beta).

## Features

- A corner minimap that uses the game's own map, its explored-area clouds and its icons.
- Rendered terrain: the ground around you is captured from above once per area and kept, so it is far more detailed than the map art. It works in the overworld and in dungeons, where roofs are left out so you can see the rooms. Overworld terrain is saved to disk, so areas you have been to show right away next time.
- Markers for fast travel points, bases, dungeons, towers, bosses, quests, warps, oil rigs, supply drops, treasure maps, death drops and pins.
- Markers for wild, shiny and own pals, humans and other players, with pal portraits and an above or below arrow when they are on another level.
- Markers for chests, eggs, effigies, notes, junk, ore, skill fruit trees, lotuses, forage, fishing spots and items lying on the ground. Every type can be switched on or off.
- Landmarks (fast travel points, bases, dungeons, towers, bosses, quests and warps) stay readable when zoomed out, and the nearest ones off the minimap are pinned to its edge. Death drops get a red ring and the same treatment.
- Effigies, notes, skill fruit trees, supply drops and treasure maps sit on colored diamonds (gold, white, purple and teal), stay full size when zoomed out and are drawn above pals and other markers.
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
- Rendered terrain is colored to look like the game's own map. Turn off "Color terrain like the game's map" for its natural colors.
- Rendered overworld terrain is saved to disk and loaded back next time. Turn off "Save rendered terrain between sessions" to stop that. The Clear button on the "Saved terrain" row deletes what was saved (click it twice to confirm).
- The view is 220 m across, and 120 m in dungeons.
- Respawn points and your own pals are hidden. So are very common pickups (wood, stone, small pal crystals, berries, eggs, arrows, leather and wool).
- The minimap hides inside your bases.

## Notes

- Rendered terrain uses fixed lighting, so tiles match at any time of day. With map colors off, lakes that get their colour from reflecting the sky can show through to the lake bed. Lamps, torches and glowing pals are switched off for the moment of each capture, though their glowing surfaces can still show as small bright spots.
- Pals, people and other players are left out of rendered terrain.
- Only overworld terrain is saved. Dungeon terrain is captured fresh on each visit, since another dungeon can be placed in the same spot.
- Saved terrain is captured again when the game updates or when player structures in that area change. Each saved tile takes up to about half a megabyte.
- Pal portraits and saved terrain are kept in `Mods\SimpleMiniMap\cache\`. Deleting that folder is safe.
## Building

Requires Visual Studio 2026 (C++ workload), CMake 4.2 or newer (the one that ships with Visual Studio 2026 works, from a Developer PowerShell) and Rust (stable, MSVC toolchain).

```
git clone --recursive https://github.com/SalahaldinBilal/SimpleMiniMap
cmake -S . -B build -G "Visual Studio 18 2026"
cmake --build build --config Game__Shipping__Win64 --target SimpleMiniMap
```

`deps/RE-UE4SS` is a submodule pinned to the UE4SS build the Workshop package ships. A native mod has to be built against the UE4SS version the game runs, so when the Workshop UE4SS updates, the submodule needs to move to match.

The build copies `main.dll` into `Mods\SimpleMiniMap-dev` at the default Steam path, so it never overwrites an installed release. Set `SMM_DEPLOY_NAME` to change the folder name, and only keep one copy enabled at a time. Close the game first, or the copy fails.

`shim/` is a small C++ layer over UE4SS, and `core/` holds the minimap itself, written in Rust.
