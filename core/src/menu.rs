use std::time::{Duration, Instant};

use crate::assets::{Assets, Load};
use crate::host;
use crate::keys::{self, Keys};
use crate::settings::{
    self, CATEGORIES, Corner, GROUPS, ICON_SCALE_RANGE, ICON_ZOOM_RANGE, LANDMARK_SCALE_RANGE, KeyBind, MEGAZOOM_RANGE, OPACITY_RANGE, PICKUPS_GROUP,
    ROUNDNESS_RANGE, SIZE_RANGE, Settings, TerrainUse, ZOOM_RANGE,
};
use crate::ue::{LinearColor, Margin, Obj, Vec2, Weak};
use crate::widgets::{self, CENTER, END, FILL, Place, START, Widgets, rgba};

const TEXT: LinearColor = rgba(0.93, 0.96, 0.98, 1.0);
const DIM: LinearColor = rgba(0.56, 0.64, 0.72, 1.0);
const ACCENT: LinearColor = rgba(0.435, 0.827, 1.0, 1.0);
const ON_ACCENT: LinearColor = rgba(0.02, 0.07, 0.11, 1.0);
const OFF: LinearColor = rgba(0.16, 0.2, 0.25, 1.0);
const OFF_TEXT: LinearColor = rgba(0.42, 0.48, 0.55, 1.0);
const BACKDROP: LinearColor = rgba(0.01, 0.02, 0.04, 0.55);
const PANEL: LinearColor = rgba(0.04, 0.07, 0.12, 0.95);
const HEADER: LinearColor = rgba(0.06, 0.11, 0.17, 1.0);
const ROW: LinearColor = rgba(1.0, 1.0, 1.0, 0.04);
const CARD_EDGE: LinearColor = rgba(0.435, 0.827, 1.0, 0.25);
const CARD_EDGE_OFF: LinearColor = rgba(1.0, 1.0, 1.0, 0.08);
const BANNER: LinearColor = rgba(0.13, 0.3, 0.4, 1.0);
const BANNER_OFF: LinearColor = rgba(0.09, 0.11, 0.14, 1.0);
const TAB_IDLE: LinearColor = rgba(0.09, 0.14, 0.2, 1.0);
const OUTLINE: LinearColor = rgba(0.435, 0.827, 1.0, 0.9);
const DRAG_AREA: LinearColor = rgba(0.435, 0.827, 1.0, 0.08);

const WINDOW_SIZE: Vec2 = Vec2 { x: 920.0, y: 660.0 };
const CONTROL_WIDTH: f32 = 380.0;
const MUTED_ROW: f32 = 0.35;
const INPUT_EVERY: Duration = Duration::from_millis(250);
const HANDLE: f64 = 18.0;
const EDGE: f64 = 2.0;
const Z_MENU: i32 = 50;
const KEY_E: u8 = b'E';
const KEY_R: u8 = b'R';
const TABS: [&str; 3] = ["Minimap", "Markers", "Keys"];

#[derive(Clone, Copy)]
enum Toggle {
    Enabled,
    ShowHint,
    MarkersInFog,
    RotateWithCamera,
    ViewCone,
    AutoZoom,
    HideInBase,
    IconZoom,
    LandmarkBacking,
    EdgePins,
    FogOfWar,
    Category(usize),
    Pickup(usize),
}

/// Where the Markers tab gets its pictures and its item pickup rows.
pub struct MenuContent<'a> {
    /// A texture for a marker category's row.
    pub category_texture: &'a dyn Fn(usize) -> Option<String>,
    /// The game's own icon widget for a category's row, set up like the minimap's, when it uses one.
    pub category_widget: &'a mut dyn FnMut(usize) -> Option<Obj>,
    pub pickups: &'a dyn Fn(&Settings) -> Vec<PickupRow>,
}

/// An item pickup type seen so far: index into `Settings::pickups`, display name, icon texture.
pub type PickupRow = (usize, String, String);

#[derive(Clone, Copy)]
enum Number {
    Size,
    Opacity,
    Zoom,
    DungeonZoom,
    IconScale,
    Roundness,
    Megazoom,
    IconZoomStrength,
    LandmarkScale,
}

/// A setting picked from a short list with back and forward buttons.
#[derive(Clone, Copy)]
enum Cycle {
    Corner,
    OverworldTerrain,
    DungeonTerrain,
}

impl Cycle {
    fn step(self, settings: &mut Settings, forward: bool) {
        fn next<T: Copy + PartialEq>(all: &[T], current: T, forward: bool) -> T {
            let at = all.iter().position(|item| *item == current).unwrap_or(0);
            all[(at + if forward { 1 } else { all.len() - 1 }) % all.len()]
        }
        match self {
            Cycle::Corner => settings.corner = next(&Corner::ALL, settings.corner, forward),
            Cycle::OverworldTerrain => {
                settings.overworld_terrain = next(&TerrainUse::ALL, settings.overworld_terrain, forward);
            }
            Cycle::DungeonTerrain => settings.dungeon_terrain = next(&TerrainUse::ALL, settings.dungeon_terrain, forward),        }
    }

    fn label(self, settings: &Settings) -> &'static str {
        match self {
            Cycle::Corner => settings.corner.label(),
            Cycle::OverworldTerrain => settings.overworld_terrain.label(),
            Cycle::DungeonTerrain => settings.dungeon_terrain.label(),        }
    }
}

#[derive(Clone, Copy)]
enum KeyAction {
    Toggle,
    ZoomIn,
    ZoomOut,
    Menu,
    Megazoom,
}

impl Toggle {
    fn get(self, settings: &Settings) -> bool {
        match self {
            Toggle::Enabled => settings.enabled,
            Toggle::ShowHint => settings.show_hint,
            Toggle::MarkersInFog => settings.markers_in_fog,
            Toggle::RotateWithCamera => settings.rotate_with_camera,
            Toggle::ViewCone => settings.view_cone,
            Toggle::AutoZoom => settings.auto_zoom,
            Toggle::HideInBase => settings.hide_in_base,
            Toggle::IconZoom => settings.icon_zoom,
            Toggle::LandmarkBacking => settings.landmark_backing,
            Toggle::EdgePins => settings.edge_pins,
            Toggle::FogOfWar => settings.fog_of_war,
            Toggle::Category(index) => settings.categories[index],
            Toggle::Pickup(index) => settings.pickups.get(index).is_some_and(|(_, shown)| *shown),
        }
    }

    fn set(self, settings: &mut Settings, on: bool) {
        match self {
            Toggle::Enabled => settings.enabled = on,
            Toggle::ShowHint => settings.show_hint = on,
            Toggle::MarkersInFog => settings.markers_in_fog = on,
            Toggle::RotateWithCamera => settings.rotate_with_camera = on,
            Toggle::ViewCone => settings.view_cone = on,
            Toggle::AutoZoom => settings.auto_zoom = on,
            Toggle::HideInBase => settings.hide_in_base = on,
            Toggle::IconZoom => settings.icon_zoom = on,
            Toggle::LandmarkBacking => settings.landmark_backing = on,
            Toggle::EdgePins => settings.edge_pins = on,
            Toggle::FogOfWar => settings.fog_of_war = on,
            Toggle::Category(index) => settings.categories[index] = on,
            Toggle::Pickup(index) => {
                if let Some(entry) = settings.pickups.get_mut(index) {
                    entry.1 = on;
                }
            }
        }
    }
}

impl Number {
    /// Slider range, step, and the slider value for a setting.
    fn range(self) -> (f32, f32, f32) {
        let (min, max, step) = match self {
            Number::Size => (SIZE_RANGE.0, SIZE_RANGE.1, 10.0),
            Number::Opacity => (OPACITY_RANGE.0 * 100.0, OPACITY_RANGE.1 * 100.0, 5.0),
            Number::Zoom | Number::DungeonZoom => (ZOOM_RANGE.0, ZOOM_RANGE.1, 10.0),
            Number::IconScale => (ICON_SCALE_RANGE.0 * 100.0, ICON_SCALE_RANGE.1 * 100.0, 5.0),
            Number::Roundness => (ROUNDNESS_RANGE.0, ROUNDNESS_RANGE.1, 5.0),
            Number::Megazoom => (MEGAZOOM_RANGE.0, MEGAZOOM_RANGE.1, 50.0),
            Number::IconZoomStrength => (ICON_ZOOM_RANGE.0 * 100.0, ICON_ZOOM_RANGE.1 * 100.0, 5.0),
            Number::LandmarkScale => (LANDMARK_SCALE_RANGE.0 * 100.0, LANDMARK_SCALE_RANGE.1 * 100.0, 5.0),
        };
        (min as f32, max as f32, step as f32)
    }

    fn get(self, settings: &Settings) -> f32 {
        (match self {
            Number::Size => settings.size,
            Number::Opacity => settings.opacity * 100.0,
            Number::Zoom => settings.zoom_metres,
            Number::DungeonZoom => settings.dungeon_zoom_metres,
            Number::IconScale => settings.icon_scale * 100.0,
            Number::Roundness => settings.roundness,
            Number::Megazoom => settings.megazoom_metres,
            Number::IconZoomStrength => settings.icon_zoom_strength * 100.0,
            Number::LandmarkScale => settings.landmark_scale * 100.0,
        }) as f32
    }

    fn set(self, settings: &mut Settings, value: f32) {
        let step = self.range().2;
        let value = f64::from((value / step).round() * step);
        match self {
            Number::Size => settings.size = value,
            Number::Opacity => settings.opacity = value / 100.0,
            Number::Zoom => settings.zoom_metres = value,
            Number::DungeonZoom => settings.dungeon_zoom_metres = value,
            Number::IconScale => settings.icon_scale = value / 100.0,
            Number::Roundness => settings.roundness = value,
            Number::Megazoom => settings.megazoom_metres = value,
            Number::IconZoomStrength => settings.icon_zoom_strength = value / 100.0,
            Number::LandmarkScale => settings.landmark_scale = value / 100.0,
        }
    }

    fn format(self, value: f32) -> String {
        match self {
            Number::Size => format!("{value:.0}"),
            Number::Opacity | Number::IconScale | Number::Roundness | Number::IconZoomStrength | Number::LandmarkScale => {
                format!("{value:.0}%")
            }
            Number::Zoom | Number::DungeonZoom | Number::Megazoom => format!("{value:.0} m"),
        }
    }
}

impl KeyAction {
    fn binds(self, settings: &Settings) -> &[KeyBind] {
        match self {
            KeyAction::Toggle => &settings.toggle_keys,
            KeyAction::ZoomIn => &settings.zoom_in_keys,
            KeyAction::ZoomOut => &settings.zoom_out_keys,
            KeyAction::Menu => &settings.menu_keys,
            KeyAction::Megazoom => &settings.megazoom_keys,
        }
    }

    fn binds_mut(self, settings: &mut Settings) -> &mut Vec<KeyBind> {
        match self {
            KeyAction::Toggle => &mut settings.toggle_keys,
            KeyAction::ZoomIn => &mut settings.zoom_in_keys,
            KeyAction::ZoomOut => &mut settings.zoom_out_keys,
            KeyAction::Menu => &mut settings.menu_keys,
            KeyAction::Megazoom => &mut settings.megazoom_keys,
        }
    }
}

/// A button, with what it looked like last frame so a release over it counts as one click.
struct Clickable {
    button: Weak,
    was_pressed: bool,
}

impl Clickable {
    fn new(button: Obj) -> Self {
        Self { button: Weak::new(button), was_pressed: false }
    }

    fn held(&mut self, w: &mut Widgets) -> bool {
        let pressed = self.button.get().is_some_and(|button| w.pressed(button));
        self.was_pressed = pressed;
        pressed
    }

    fn clicked(&mut self, w: &mut Widgets) -> bool {
        let before = self.was_pressed;
        let now = self.held(w);
        before && !now
    }
}

struct Labeled {
    clickable: Clickable,
    text: Weak,
}

enum Kind {
    Switch { toggle: Toggle, off: Labeled, on: Labeled },
    Cycle { cycle: Cycle, left: Clickable, right: Clickable, text: Weak },
    Slider { number: Number, slider: Weak, text: Weak, last: f32 },
    Group { group: usize, pill: Labeled, banner: Weak, edge: Weak, count: Weak, rows: Vec<Weak>, members: Vec<Toggle> },
    Key { action: KeyAction, button: Labeled },
}

struct Control {
    tab: usize,
    kind: Kind,
}

struct Window {
    widget: Weak,
    tree: Weak,
    root: Weak,
    backdrop: Weak,
    panel: Weak,
    tabs: Vec<Labeled>,
    pages: Vec<Weak>,
    current: usize,
    footer: Vec<(Clickable, Action)>,
    controls: Vec<Control>,
    listening: Option<usize>,
    icons: Vec<(Weak, String)>,
    edit: Option<Edit>,
    next_input: Instant,
}

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Edit,
    Reset,
    Close,
    Save,
    Cancel,
}

struct Edit {
    layer: Weak,
    area: Clickable,
    area_slot: Weak,
    handle: Clickable,
    handle_slot: Weak,
    edges: Vec<Weak>,
    buttons: Vec<(Clickable, Action)>,
    before: (f64, f64, f64),
    drag: Option<Drag>,
}

#[derive(Clone, Copy)]
struct Drag {
    resize: bool,
    mouse: Vec2,
    left: f64,
    top: f64,
    size: f64,
}

/// Whether a frame of menu work changed the settings.
#[derive(Default)]
struct Outcome {
    changed: bool,
}

/// The in-game settings window and the minimap move and resize mode.
pub struct Menu {
    w: Widgets,
    window: Option<Window>,
    toggle_requested: bool,
}

impl Menu {
    pub fn new() -> Result<Self, String> {
        Ok(Self { w: Widgets::resolve()?, window: None, toggle_requested: false })
    }

    pub fn widgets(&mut self) -> &mut Widgets {
        &mut self.w
    }

    /// Whether every key should be watched, to capture a new binding.
    pub fn capturing_key(&self) -> bool {
        self.window.as_ref().is_some_and(|window| window.listening.is_some())
    }

    /// Opens or closes on the next update, where the player controller is at hand.
    pub fn toggle(&mut self) {
        self.toggle_requested = true;
    }

    /// Gives input back to the game, then takes the window down. In this order: Slate keeps a focus path
    /// into the window until input is handed back, and removing the window first crashes the game.
    fn close(&mut self, controller: Obj) {
        let Some(window) = self.window.take() else { return };
        self.w.release_input(controller);
        if let Some(widget) = window.widget.get() {
            crate::ue::set_bool_property(widget, "bIsFocusable", false);
            self.w.visibility(widget, widgets::COLLAPSED);
            self.w.remove(widget);
        }
        host::info("settings menu closed");
    }

    /// The world is going away: hand input back while the controller still exists.
    pub fn world_unload(&mut self, controller: Option<Obj>) {
        self.toggle_requested = false;
        match controller {
            Some(controller) => self.close(controller),
            None => self.window = None,
        }
    }

    /// Runs the menu for a frame. Returns true when it changed the settings.
    pub fn update(
        &mut self,
        world: Obj,
        controller: Obj,
        settings: &mut Settings,
        keys: &Keys,
        assets: &mut Assets,
        content: &mut MenuContent<'_>,
    ) -> bool {
        let mut outcome = Outcome::default();
        if std::mem::take(&mut self.toggle_requested) {
            if self.window.is_some() {
                self.close(controller);
                return false;
            }
            match self.build(world, controller, settings, content) {
                Ok(window) => {
                    self.window = Some(window);
                    host::info("settings menu opened");
                }
                Err(reason) => {
                    self.w.release_input(controller);
                    host::warning(&format!("settings menu could not open: {reason}"));
                }
            }
        }
        let Some(mut window) = self.window.take() else { return false };
        let Some(widget) = window.widget.get() else {
            self.w.release_input(controller);
            return false;
        };
        // Reapplying input takes focus back to the window, which drops a slider or drag the mouse is holding.
        if Instant::now() >= window.next_input && !keys.mouse_held() {
            window.next_input = Instant::now() + INPUT_EVERY;
            self.w.take_input(controller, widget);
        }
        self.load_icons(&mut window, world, assets);

        let mut close = false;
        if window.edit.is_some() {
            close = self.update_edit(&mut window, world, settings, keys, &mut outcome);
        } else if let Some(index) = window.listening {
            if keys.plain_pressed(keys::ESCAPE) {
                window.listening = None;
                self.refresh(&mut window, settings);
            } else if let Some(bind) = keys.captured(&settings::bindable_keys()) {
                if let Kind::Key { action, .. } = window.controls[index].kind {
                    *action.binds_mut(settings) = vec![bind];
                    outcome.changed = true;
                }
                window.listening = None;
                self.refresh(&mut window, settings);
            }
        } else {
            close = self.update_window(&mut window, world, settings, keys, &mut outcome);
        }

        self.window = Some(window);
        if close {
            self.close(controller);
        }
        outcome.changed
    }

    fn update_window(&mut self, window: &mut Window, world: Obj, settings: &mut Settings, keys: &Keys, outcome: &mut Outcome) -> bool {
        let w = &mut self.w;
        let mut action = None;
        if keys.plain_pressed(keys::ESCAPE) {
            action = Some(Action::Close);
        } else if keys.plain_pressed(KEY_E) {
            action = Some(Action::Edit);
        } else if keys.plain_pressed(KEY_R) {
            action = Some(Action::Reset);
        }
        for (clickable, footer_action) in &mut window.footer {
            if clickable.clicked(w) {
                action = Some(*footer_action);
            }
        }
        let mut tab = None;
        for (index, button) in window.tabs.iter_mut().enumerate() {
            if button.clickable.clicked(w) {
                tab = Some(index);
            }
        }

        let mut changed = false;
        let current = window.current;
        let mut listen = None;
        for (index, control) in window.controls.iter_mut().enumerate() {
            if control.tab != current {
                continue;
            }
            match &mut control.kind {
                Kind::Switch { toggle, off, on } => {
                    let (turn_off, turn_on) = (off.clickable.clicked(w), on.clickable.clicked(w));
                    let wanted = if turn_off { Some(false) } else if turn_on { Some(true) } else { None };
                    if let Some(wanted) = wanted.filter(|&wanted| wanted != toggle.get(settings)) {
                        toggle.set(settings, wanted);
                        changed = true;
                    }
                }
                Kind::Cycle { cycle, left, right, .. } => {
                    let (back, forward) = (left.clicked(w), right.clicked(w));
                    if back || forward {
                        cycle.step(settings, forward);
                        changed = true;
                    }
                }
                Kind::Slider { number, slider, text, last } => {
                    let Some(slider) = slider.get() else { continue };
                    let value = w.slider_value(slider);
                    if (value - *last).abs() > 0.01 {
                        *last = value;
                        number.set(settings, value);
                        if let Some(text) = text.get() {
                            w.set_text(text, &number.format(value));
                        }
                        outcome.changed = true;
                    }
                }
                Kind::Group { group, pill, .. } => {
                    if pill.clickable.clicked(w) {
                        settings.groups[*group] = !settings.groups[*group];
                        changed = true;
                    }
                }
                Kind::Key { button, .. } => {
                    if button.clickable.clicked(w) {
                        listen = Some(index);
                    }
                }
            }
        }
        if let Some(index) = listen {
            window.listening = Some(index);
            changed = true;
        }

        if let Some(tab) = tab.filter(|&tab| tab != window.current) {
            window.current = tab;
            for (index, page) in window.pages.iter().enumerate() {
                if let Some(page) = page.get() {
                    w.visibility(page, if index == tab { widgets::SELF_HIT_TEST_INVISIBLE } else { widgets::COLLAPSED });
                }
            }
            changed = true;
        }

        match action {
            Some(Action::Close) => return true,
            Some(Action::Reset) => {
                let defaults = Settings::default();
                *settings = Settings {
                    toggle_keys: settings.toggle_keys.clone(),
                    zoom_in_keys: settings.zoom_in_keys.clone(),
                    zoom_out_keys: settings.zoom_out_keys.clone(),
                    menu_keys: settings.menu_keys.clone(),
                    ..defaults
                };
                self.sync_sliders(window, settings);
                changed = true;
            }
            Some(Action::Edit) => {
                match self.build_edit(window, world, settings) {
                    Ok(edit) => window.edit = Some(edit),
                    Err(reason) => host::warning(&format!("move and resize mode could not start: {reason}")),
                }
                return false;
            }
            _ => {}
        }
        if changed {
            outcome.changed = true;
            self.refresh(window, settings);
        }
        false
    }

    fn sync_sliders(&mut self, window: &mut Window, settings: &Settings) {
        for control in &mut window.controls {
            if let Kind::Slider { number, slider, text, last } = &mut control.kind {
                let value = number.get(settings);
                *last = value;
                if let Some(slider) = slider.get() {
                    self.w.set_slider(slider, value);
                }
                if let Some(text) = text.get() {
                    self.w.set_text(text, &number.format(value));
                }
            }
        }
    }

    fn load_icons(&mut self, window: &mut Window, world: Obj, assets: &mut Assets) {
        let w = &mut self.w;
        window.icons.retain(|(image, path)| {
            let Some(image) = image.get() else { return false };
            match assets.get(world, path) {
                Load::Ready(texture) => {
                    w.image_texture(image, texture);
                    w.visibility(image, widgets::HIT_TEST_INVISIBLE);
                    false
                }
                Load::Waiting => true,
                Load::Failed => false,
            }
        });
    }

    /// Brings every control's look in line with the settings.
    fn refresh(&mut self, window: &mut Window, settings: &Settings) {
        let w = &mut self.w;
        for (index, tab) in window.tabs.iter().enumerate() {
            let active = index == window.current;
            if let Some(button) = tab.clickable.button.get() {
                w.button_color(button, if active { ACCENT } else { TAB_IDLE });
            }
            if let Some(text) = tab.text.get() {
                w.text_color(text, if active { ON_ACCENT } else { DIM });
            }
        }
        for (index, control) in window.controls.iter().enumerate() {
            match &control.kind {
                Kind::Switch { toggle, off, on } => {
                    let state = toggle.get(settings);
                    paint_choice(w, off, !state);
                    paint_choice(w, on, state);
                }
                Kind::Cycle { cycle, text, .. } => {
                    if let Some(text) = text.get() {
                        w.set_text(text, cycle.label(settings));
                    }
                }
                Kind::Slider { .. } => {}
                Kind::Group { group, pill, banner, edge, count, rows, members } => {
                    let on = settings.groups[*group];
                    paint_choice(w, pill, on);
                    if let Some(text) = pill.text.get() {
                        w.set_text(text, if on { "ON" } else { "OFF" });
                    }
                    if let Some(banner) = banner.get() {
                        w.border_color(banner, if on { BANNER } else { BANNER_OFF });
                    }
                    if let Some(edge) = edge.get() {
                        w.border_color(edge, if on { CARD_EDGE } else { CARD_EDGE_OFF });
                    }
                    let shown = members.iter().filter(|member| member.get(settings)).count();
                    if let Some(count) = count.get() {
                        let text = match (on, members.len()) {
                            (false, _) => "Whole category hidden".to_string(),
                            (true, 0) => "None found yet. Item types are added as you come across them.".to_string(),
                            (true, total) => format!("{shown} of {total} types shown"),
                        };
                        w.set_text(count, &text);
                    }
                    for row in rows.iter().filter_map(|row| row.get()) {
                        w.opacity(row, if on { 1.0 } else { MUTED_ROW });
                    }
                }
                Kind::Key { action, button } => {
                    let listening = window.listening == Some(index);
                    let label = if listening {
                        "Press a key (Esc cancels)".to_string()
                    } else {
                        settings::format_keys(action.binds(settings))
                    };
                    if let Some(text) = button.text.get() {
                        w.set_text(text, &label);
                        w.text_color(text, if listening { ON_ACCENT } else { TEXT });
                    }
                    if let Some(button) = button.clickable.button.get() {
                        w.button_color(button, if listening { ACCENT } else { OFF });
                    }
                }
            }
        }
    }

    fn build(
        &mut self,
        world: Obj,
        controller: Obj,
        settings: &Settings,
        content: &mut MenuContent,
    ) -> Result<Window, String> {
        let w = &mut self.w;
        let (widget, tree, root) = w.window(world, controller)?;

        let backdrop = w.border(tree, BACKDROP, Margin::default())?;
        w.canvas_fill(root, backdrop, 0)?;

        let panel = w.border(tree, PANEL, Margin::default())?;
        let window_box = w.sized(tree, panel, Some(WINDOW_SIZE.x as f32), Some(WINDOW_SIZE.y as f32))?;
        let slot = w.canvas_add(root, window_box, Vec2 { x: 0.5, y: 0.5 }, 1)?;
        w.place(slot, Vec2::default(), None);

        let column = w.make(w.vbox_class, tree)?;
        w.put(panel, column)?;

        let mut window = Window {
            widget: Weak::new(widget),
            tree: Weak::new(tree),
            root: Weak::new(root),
            backdrop: Weak::new(backdrop),
            panel: Weak::new(window_box),
            tabs: Vec::new(),
            pages: Vec::new(),
            current: 0,
            footer: Vec::new(),
            controls: Vec::new(),
            listening: None,
            icons: Vec::new(),
            edit: None,
            next_input: Instant::now(),
        };

        // Header: title and tabs.
        let header = w.border(tree, HEADER, Margin::xy(20.0, 12.0))?;
        w.vbox_add(column, header, Place::auto())?;
        let header_row = w.make(w.hbox_class, tree)?;
        w.put(header, header_row)?;
        let title = w.text(tree, "SimpleMiniMap", 20, TEXT, true)?;
        w.hbox_add(header_row, title, Place::auto().pad(Margin { right: 28.0, ..Margin::default() }))?;
        for name in TABS {
            let tab = labeled_button(w, tree, name, 15, TAB_IDLE, Some(130.0), Some(36.0))?;
            w.hbox_add(header_row, tab.outer, Place::auto().pad(Margin { right: 8.0, ..Margin::default() }))?;
            window.tabs.push(tab.labeled());
        }

        // Pages, one per tab, in a scroll box.
        let scroll = w.make(w.scroll_class, tree)?;
        w.vbox_add(column, scroll, Place::fill().pad(Margin::xy(20.0, 10.0)))?;
        let mut pages = Vec::new();
        for index in 0..TABS.len() {
            let page = w.make(w.vbox_class, tree)?;
            w.put(scroll, page)?;
            w.visibility(page, if index == 0 { widgets::SELF_HIT_TEST_INVISIBLE } else { widgets::COLLAPSED });
            pages.push(page);
            window.pages.push(Weak::new(page));
        }

        // Minimap tab.
        let page = pages[0];
        add_switch(w, &mut window, tree, page, 0, "Show minimap", Toggle::Enabled, None)?;
        add_switch(w, &mut window, tree, page, 0, "Show settings hint under the minimap", Toggle::ShowHint, None)?;
        add_cycle(w, &mut window, tree, page, 0, "Corner", Cycle::Corner)?;
        add_slider(w, &mut window, tree, page, 0, "Size", Number::Size, settings)?;
        add_slider(w, &mut window, tree, page, 0, "Opacity", Number::Opacity, settings)?;
        add_slider(w, &mut window, tree, page, 0, "Zoom", Number::Zoom, settings)?;
        add_slider(w, &mut window, tree, page, 0, "Zoom in dungeons", Number::DungeonZoom, settings)?;
        add_slider(w, &mut window, tree, page, 0, "Icon size", Number::IconScale, settings)?;
        add_switch(w, &mut window, tree, page, 0, "Scale icons with zoom", Toggle::IconZoom, None)?;
        add_slider(w, &mut window, tree, page, 0, "How much icons follow the zoom", Number::IconZoomStrength, settings)?;
        add_slider(w, &mut window, tree, page, 0, "Landmark icon size", Number::LandmarkScale, settings)?;
        add_switch(w, &mut window, tree, page, 0, "Dark backing behind landmarks", Toggle::LandmarkBacking, None)?;
        add_switch(w, &mut window, tree, page, 0, "Show nearby teleporters at the edge", Toggle::EdgePins, None)?;
        add_slider(w, &mut window, tree, page, 0, "Roundness", Number::Roundness, settings)?;
        add_switch(w, &mut window, tree, page, 0, "Rotate with the camera", Toggle::RotateWithCamera, None)?;
        add_switch(w, &mut window, tree, page, 0, "Show where the camera looks", Toggle::ViewCone, None)?;
        add_switch(w, &mut window, tree, page, 0, "Zoom out while moving fast", Toggle::AutoZoom, None)?;
        add_slider(w, &mut window, tree, page, 0, "Megazoom range", Number::Megazoom, settings)?;
        add_switch(w, &mut window, tree, page, 0, "Hide while in your base", Toggle::HideInBase, None)?;
        add_cycle(w, &mut window, tree, page, 0, "Rendered terrain in the overworld", Cycle::OverworldTerrain)?;
        add_cycle(w, &mut window, tree, page, 0, "Rendered terrain in dungeons", Cycle::DungeonTerrain)?;        add_switch(w, &mut window, tree, page, 0, "Clouds over unexplored areas", Toggle::FogOfWar, None)?;
        add_switch(w, &mut window, tree, page, 0, "Show markers in unexplored areas", Toggle::MarkersInFog, None)?;

        // Markers tab: one card per group.
        let page = pages[1];
        for (group, (_, label)) in GROUPS.iter().enumerate() {
            add_group(w, &mut window, tree, page, group, label, settings, content)?;
        }

        // Keys tab.
        let page = pages[2];
        add_key(w, &mut window, tree, page, "Show or hide the minimap", KeyAction::Toggle)?;
        add_key(w, &mut window, tree, page, "Zoom in", KeyAction::ZoomIn)?;
        add_key(w, &mut window, tree, page, "Zoom out", KeyAction::ZoomOut)?;
        add_key(w, &mut window, tree, page, "Megazoom on or off", KeyAction::Megazoom)?;
        add_key(w, &mut window, tree, page, "Open this menu", KeyAction::Menu)?;

        // Footer.
        let footer = w.border(tree, HEADER, Margin::xy(20.0, 10.0))?;
        w.vbox_add(column, footer, Place::auto())?;
        let footer_row = w.make(w.hbox_class, tree)?;
        w.put(footer, footer_row)?;
        let spacer = w.make(w.hbox_class, tree)?;
        w.hbox_add(footer_row, spacer, Place::fill())?;
        for (label, action) in [("E   Move and resize", Action::Edit), ("R   Reset to defaults", Action::Reset), ("Esc   Close", Action::Close)] {
            let button = labeled_button(w, tree, label, 14, OFF, None, Some(34.0))?;
            w.hbox_add(footer_row, button.outer, Place::auto().pad(Margin { left: 10.0, ..Margin::default() }))?;
            window.footer.push((Clickable::new(button.button), action));
        }

        w.show_on_screen(widget, Z_MENU);
        crate::ue::set_bool_property(widget, "bIsFocusable", true);
        w.visibility(widget, widgets::VISIBLE);
        w.take_input(controller, widget);
        self.refresh(&mut window, settings);
        Ok(window)
    }

    fn update_edit(&mut self, window: &mut Window, world: Obj, settings: &mut Settings, keys: &Keys, outcome: &mut Outcome) -> bool {
        let w = &mut self.w;
        let Some(edit) = window.edit.as_mut() else { return false };
        let screen = w.screen(world);
        let mouse = w.mouse(world);

        let mut action = None;
        if keys.plain_pressed(keys::ESCAPE) {
            action = Some(Action::Cancel);
        } else if keys.plain_pressed(keys::ENTER) {
            action = Some(Action::Save);
        }
        for (clickable, button_action) in &mut edit.buttons {
            if clickable.clicked(w) {
                action = Some(*button_action);
            }
        }

        let area_held = edit.area.held(w);
        let handle_held = edit.handle.held(w);
        let (left, top) = frame_origin(settings, screen);
        match edit.drag {
            None if handle_held || area_held => {
                edit.drag = Some(Drag { resize: handle_held, mouse, left, top, size: settings.size });
            }
            Some(drag) if (drag.resize && handle_held) || (!drag.resize && area_held) => {
                let (dx, dy) = (mouse.x - drag.mouse.x, mouse.y - drag.mouse.y);
                if drag.resize {
                    let room = (screen.x - drag.left).min(screen.y - drag.top);
                    let size = (drag.size + dx.max(dy)).clamp(SIZE_RANGE.0, SIZE_RANGE.1.min(room.max(SIZE_RANGE.0)));
                    settings.size = size.round();
                    set_origin(settings, screen, drag.left, drag.top);
                } else {
                    let left = (drag.left + dx).clamp(0.0, (screen.x - settings.size).max(0.0));
                    let top = (drag.top + dy).clamp(0.0, (screen.y - settings.size).max(0.0));
                    set_origin(settings, screen, left, top);
                }
                outcome.changed = true;
            }
            Some(_) => edit.drag = None,
            None => {}
        }
        place_edit_layer(w, edit, settings, screen);

        match action {
            Some(Action::Cancel) => {
                (settings.offset_x, settings.offset_y, settings.size) = edit.before;
                outcome.changed = true;
            }
            Some(Action::Save) => {}
            _ => return false,
        }
        if let Some(layer) = edit.layer.get() {
            w.remove(layer);
        }
        window.edit = None;
        // The backdrop catches clicks so they do not reach the game; the panel only passes them to its controls.
        if let Some(backdrop) = window.backdrop.get() {
            w.visibility(backdrop, widgets::VISIBLE);
        }
        if let Some(panel) = window.panel.get() {
            w.visibility(panel, widgets::SELF_HIT_TEST_INVISIBLE);
        }
        self.sync_sliders(window, settings);
        false
    }

    fn build_edit(&mut self, window: &mut Window, world: Obj, settings: &Settings) -> Result<Edit, String> {
        let w = &mut self.w;
        let (Some(tree), Some(root)) = (window.tree.get(), window.root.get()) else { return Err("menu is gone".into()) };
        for widget in [window.backdrop.get(), window.panel.get()].into_iter().flatten() {
            w.visibility(widget, widgets::COLLAPSED);
        }

        let layer = w.make(w.canvas_class, tree)?;
        w.canvas_fill(root, layer, 5)?;

        let area = w.button(tree, DRAG_AREA)?;
        let area_slot = w.canvas_add(layer, area, Vec2::default(), 1)?;
        let mut edges = Vec::new();
        for _ in 0..4 {
            let edge = w.border(tree, OUTLINE, Margin::default())?;
            let slot = w.canvas_add(layer, edge, Vec2::default(), 2)?;
            w.visibility(edge, widgets::HIT_TEST_INVISIBLE);
            edges.push(Weak::new(slot));
        }
        let handle = w.button(tree, ACCENT)?;
        let handle_slot = w.canvas_add(layer, handle, Vec2::default(), 3)?;

        let bar = w.border(tree, PANEL, Margin::xy(20.0, 10.0))?;
        let bar_slot = w.canvas_add(layer, bar, Vec2 { x: 0.5, y: 1.0 }, 4)?;
        w.place(bar_slot, Vec2 { x: 0.0, y: -40.0 }, None);
        let row = w.make(w.hbox_class, tree)?;
        w.put(bar, row)?;
        for hint in ["Drag the minimap to move it", "Drag the corner square to resize it"] {
            let text = w.text(tree, hint, 14, DIM, false)?;
            w.hbox_add(row, text, Place::auto().pad(Margin { right: 24.0, ..Margin::default() }))?;
        }
        let mut buttons = Vec::new();
        for (label, action) in [("Enter   Save", Action::Save), ("Esc   Cancel", Action::Cancel)] {
            let button = labeled_button(w, tree, label, 14, OFF, None, Some(34.0))?;
            w.hbox_add(row, button.outer, Place::auto().pad(Margin { left: 10.0, ..Margin::default() }))?;
            buttons.push((Clickable::new(button.button), action));
        }

        let mut edit = Edit {
            layer: Weak::new(layer),
            area: Clickable::new(area),
            area_slot: Weak::new(area_slot),
            handle: Clickable::new(handle),
            handle_slot: Weak::new(handle_slot),
            edges,
            buttons,
            before: (settings.offset_x, settings.offset_y, settings.size),
            drag: None,
        };
        let screen = w.screen(world);
        place_edit_layer(w, &mut edit, settings, screen);
        Ok(edit)
    }
}

fn group_members(group: usize) -> impl Iterator<Item = usize> {
    CATEGORIES
        .iter()
        .enumerate()
        .filter(move |(_, category)| category.group == Some(group))
        .map(|(index, _)| index)
}

fn paint_choice(w: &mut Widgets, choice: &Labeled, active: bool) {
    if let Some(button) = choice.clickable.button.get() {
        w.button_color(button, if active { ACCENT } else { OFF });
    }
    if let Some(text) = choice.text.get() {
        w.text_color(text, if active { ON_ACCENT } else { OFF_TEXT });
    }
}

/// A button holding a centred label, wrapped in a size box.
struct LabeledButton {
    outer: Obj,
    button: Obj,
    text: Obj,
}

impl LabeledButton {
    fn labeled(&self) -> Labeled {
        Labeled { clickable: Clickable::new(self.button), text: Weak::new(self.text) }
    }
}

fn labeled_button(
    w: &mut Widgets,
    tree: Obj,
    label: &str,
    size: i32,
    color: LinearColor,
    width: Option<f32>,
    height: Option<f32>,
) -> Result<LabeledButton, String> {
    let button = w.button(tree, color)?;
    let text = w.text(tree, label, size, TEXT, true)?;
    w.center_text(text);
    w.put(button, text)?;
    let outer = w.sized(tree, button, width, height)?;
    Ok(LabeledButton { outer, button, text })
}

/// A settings row: label on the left, the control on the right. Returns the row and the control's box.
fn row(w: &mut Widgets, tree: Obj, page: Obj, label: &str, icon: Option<Obj>, indent: f32) -> Result<(Obj, Obj), String> {
    let border = w.border(tree, ROW, Margin::xy(14.0, 6.0))?;
    w.vbox_add(page, border, Place::auto().pad(Margin { left: indent, top: 3.0, ..Margin::default() }))?;
    let line = w.make(w.hbox_class, tree)?;
    w.put(border, line)?;
    if let Some(icon) = icon {
        let sized = w.sized(tree, icon, Some(26.0), Some(26.0))?;
        w.hbox_add(line, sized, Place::auto().pad(Margin { right: 10.0, ..Margin::default() }).align(START, CENTER))?;
    }
    let text = w.text(tree, label, 15, TEXT, false)?;
    w.hbox_add(line, text, Place::fill().align(FILL, CENTER))?;
    let control = w.make(w.hbox_class, tree)?;
    let sized = w.sized(tree, control, Some(CONTROL_WIDTH), Some(34.0))?;
    w.hbox_add(line, sized, Place::auto().align(END, CENTER))?;
    Ok((border, control))
}

#[allow(clippy::too_many_arguments)]
fn add_switch(
    w: &mut Widgets,
    window: &mut Window,
    tree: Obj,
    page: Obj,
    tab: usize,
    label: &str,
    toggle: Toggle,
    icon: Option<(Obj, f32)>,
) -> Result<Obj, String> {
    let (icon, indent) = icon.map_or((None, 0.0), |(icon, indent)| (Some(icon), indent));
    let (row_border, control) = row(w, tree, page, label, icon, indent)?;
    let off = labeled_button(w, tree, "OFF", 14, OFF, None, Some(30.0))?;
    w.hbox_add(control, off.outer, Place::fill().pad(Margin { right: 6.0, ..Margin::default() }).align(FILL, CENTER))?;
    let on = labeled_button(w, tree, "ON", 14, OFF, None, Some(30.0))?;
    w.hbox_add(control, on.outer, Place::fill().align(FILL, CENTER))?;
    window.controls.push(Control { tab, kind: Kind::Switch { toggle, off: off.labeled(), on: on.labeled() } });
    Ok(row_border)
}

fn add_cycle(w: &mut Widgets, window: &mut Window, tree: Obj, page: Obj, tab: usize, label: &str, cycle: Cycle) -> Result<(), String> {
    let (_, control) = row(w, tree, page, label, None, 0.0)?;
    let left = labeled_button(w, tree, "<", 16, OFF, Some(40.0), Some(30.0))?;
    w.hbox_add(control, left.outer, Place::auto().align(START, CENTER))?;
    let text = w.text(tree, "", 15, TEXT, true)?;
    w.center_text(text);
    w.hbox_add(control, text, Place::fill().align(CENTER, CENTER))?;
    let right = labeled_button(w, tree, ">", 16, OFF, Some(40.0), Some(30.0))?;
    w.hbox_add(control, right.outer, Place::auto().align(END, CENTER))?;
    window.controls.push(Control {
        tab,
        kind: Kind::Cycle { cycle, left: Clickable::new(left.button), right: Clickable::new(right.button), text: Weak::new(text) },
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn add_slider(
    w: &mut Widgets,
    window: &mut Window,
    tree: Obj,
    page: Obj,
    tab: usize,
    label: &str,
    number: Number,
    settings: &Settings,
) -> Result<(), String> {
    let (_, control) = row(w, tree, page, label, None, 0.0)?;
    let (min, max, step) = number.range();
    let value = number.get(settings);
    let slider = w.slider(tree, min, max, step, value, OFF, ACCENT)?;
    w.hbox_add(control, slider, Place::fill().align(FILL, CENTER))?;
    let text = w.text(tree, &number.format(value), 15, TEXT, true)?;
    let sized = w.sized(tree, text, Some(70.0), None)?;
    w.hbox_add(control, sized, Place::auto().pad(Margin { left: 12.0, ..Margin::default() }).align(END, CENTER))?;
    window.controls.push(Control {
        tab,
        kind: Kind::Slider { number, slider: Weak::new(slider), text: Weak::new(text), last: value },
    });
    Ok(())
}

/// The picture slot of a marker row: an image that shows once its texture has loaded, else the game's own
/// icon widget fitted to the slot. Hidden rather than collapsed while loading, so every label lines up.
fn row_icon(w: &mut Widgets, window: &mut Window, tree: Obj, widget: Option<Obj>, texture: Option<String>) -> Result<Obj, String> {
    if let (None, Some(widget)) = (&texture, widget) {
        let fit = w.make(w.scale_box_class, tree)?;
        w.put(fit, widget)?;
        w.visibility(widget, widgets::HIT_TEST_INVISIBLE);
        return Ok(fit);
    }
    let image = w.image(tree, rgba(1.0, 1.0, 1.0, 1.0))?;
    w.visibility(image, widgets::HIDDEN);
    if let Some(path) = texture {
        window.icons.push((Weak::new(image), path));
    }
    Ok(image)
}

#[allow(clippy::too_many_arguments)]
fn add_group(
    w: &mut Widgets,
    window: &mut Window,
    tree: Obj,
    page: Obj,
    group: usize,
    label: &str,
    settings: &Settings,
    content: &mut MenuContent,
) -> Result<(), String> {
    // The card's edge is a border one unit wider than its content.
    let edge = w.border(tree, CARD_EDGE, Margin::all(1.0))?;
    w.vbox_add(page, edge, Place::auto().pad(Margin { top: 14.0, ..Margin::default() }))?;
    let card = w.make(w.vbox_class, tree)?;
    w.put(edge, card)?;

    let banner = w.border(tree, BANNER, Margin::xy(16.0, 10.0))?;
    w.vbox_add(card, banner, Place::auto())?;
    let line = w.make(w.hbox_class, tree)?;
    w.put(banner, line)?;
    let titles = w.make(w.vbox_class, tree)?;
    w.hbox_add(line, titles, Place::fill().align(FILL, CENTER))?;
    let title = w.text(tree, &label.to_uppercase(), 17, TEXT, true)?;
    w.vbox_add(titles, title, Place::auto())?;
    let count = w.text(tree, "", 12, DIM, false)?;
    w.vbox_add(titles, count, Place::auto())?;
    let pill = labeled_button(w, tree, "ON", 14, ACCENT, Some(96.0), Some(34.0))?;
    w.hbox_add(line, pill.outer, Place::auto().align(END, CENTER))?;

    let items = w.make(w.vbox_class, tree)?;
    w.vbox_add(card, items, Place::auto().pad(Margin { left: 20.0, top: 4.0, right: 8.0, bottom: 8.0 }))?;
    let mut rows = Vec::new();
    let mut members = Vec::new();
    if group == PICKUPS_GROUP {
        for (index, name, texture) in (content.pickups)(settings) {
            let icon = row_icon(w, window, tree, None, Some(texture))?;
            let toggle = Toggle::Pickup(index);
            rows.push(Weak::new(add_switch(w, window, tree, items, 1, &name, toggle, Some((icon, 0.0)))?));
            members.push(toggle);
        }
    } else {
        for category in group_members(group) {
            let texture = (content.category_texture)(category);
            let widget = if texture.is_none() { (content.category_widget)(category) } else { None };
            let icon = row_icon(w, window, tree, widget, texture)?;
            let toggle = Toggle::Category(category);
            rows.push(Weak::new(add_switch(w, window, tree, items, 1, CATEGORIES[category].label, toggle, Some((icon, 0.0)))?));
            members.push(toggle);
        }
    }

    window.controls.push(Control {
        tab: 1,
        kind: Kind::Group {
            group,
            pill: pill.labeled(),
            banner: Weak::new(banner),
            edge: Weak::new(edge),
            count: Weak::new(count),
            rows,
            members,
        },
    });
    Ok(())
}

fn add_key(w: &mut Widgets, window: &mut Window, tree: Obj, page: Obj, label: &str, action: KeyAction) -> Result<(), String> {
    let (_, control) = row(w, tree, page, label, None, 0.0)?;
    let button = labeled_button(w, tree, "", 14, OFF, None, Some(30.0))?;
    w.hbox_add(control, button.outer, Place::fill().align(FILL, CENTER))?;
    window.controls.push(Control { tab: 2, kind: Kind::Key { action, button: button.labeled() } });
    Ok(())
}

/// Top left corner of the minimap on screen, in UI units.
pub fn frame_origin(settings: &Settings, screen: Vec2) -> (f64, f64) {
    let (ax, ay) = settings.corner.anchor();
    let left = if ax == 0.0 { settings.offset_x } else { screen.x - settings.offset_x - settings.size };
    let top = if ay == 0.0 { settings.offset_y } else { screen.y - settings.offset_y - settings.size };
    (left, top)
}

fn set_origin(settings: &mut Settings, screen: Vec2, left: f64, top: f64) {
    let (ax, ay) = settings.corner.anchor();
    settings.offset_x = (if ax == 0.0 { left } else { screen.x - left - settings.size }).max(0.0).round();
    settings.offset_y = (if ay == 0.0 { top } else { screen.y - top - settings.size }).max(0.0).round();
}

fn place_edit_layer(w: &mut Widgets, edit: &mut Edit, settings: &Settings, screen: Vec2) {
    let (left, top) = frame_origin(settings, screen);
    let size = settings.size;
    if let Some(slot) = edit.area_slot.get() {
        w.place(slot, Vec2 { x: left, y: top }, Some(Vec2 { x: size, y: size }));
    }
    if let Some(slot) = edit.handle_slot.get() {
        w.place(
            slot,
            Vec2 { x: left + size - HANDLE / 2.0, y: top + size - HANDLE / 2.0 },
            Some(Vec2 { x: HANDLE, y: HANDLE }),
        );
    }
    let outline = [
        (left - EDGE, top - EDGE, size + 2.0 * EDGE, EDGE),
        (left - EDGE, top + size, size + 2.0 * EDGE, EDGE),
        (left - EDGE, top, EDGE, size),
        (left + size, top, EDGE, size),
    ];
    for (slot, (x, y, width, height)) in edit.edges.iter().zip(outline) {
        if let Some(slot) = slot.get() {
            w.place(slot, Vec2 { x, y }, Some(Vec2 { x: width, y: height }));
        }
    }
}
