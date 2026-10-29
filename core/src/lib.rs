mod actors;
mod assets;
mod badges;
mod game;
mod host;
mod icons;
mod keys;
mod layers;
mod markers;
mod menu;
mod minimap;
mod objects;
mod profile;
mod settings;
mod terrain;
mod ue;
mod widgets;
mod worldmap;

use std::ffi::c_void;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use game::Game;
use minimap::Minimap;
use settings::Settings;
use ue::Obj;

const REPORT_EVERY: Duration = Duration::from_secs(10);

// Actor spawn and despawn events. Kept outside the core lock, because a game call made while the
// core is locked could itself spawn an actor.
static ACTOR_FILTER: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static ACTOR_EVENTS: Mutex<Vec<(bool, ue::Weak)>> = Mutex::new(Vec::new());

struct FrameBudget {
    window_start: Instant,
    frames: u32,
    spent: Duration,
    worst: Duration,
}

impl FrameBudget {
    fn new() -> Self {
        Self {
            window_start: Instant::now(),
            frames: 0,
            spent: Duration::ZERO,
            worst: Duration::ZERO,
        }
    }

    fn record(&mut self, cost: Duration) {
        self.frames += 1;
        self.spent += cost;
        self.worst = self.worst.max(cost);

        let elapsed = self.window_start.elapsed();
        if elapsed < REPORT_EVERY {
            return;
        }
        let fps = self.frames as f64 / elapsed.as_secs_f64();
        let average_us = self.spent.as_secs_f64() * 1e6 / self.frames.max(1) as f64;
        host::info(&format!(
            "frame budget: {fps:.0} fps, our cost {average_us:.1} us average, {} us worst",
            self.worst.as_micros()
        ));
        profile::report(elapsed);
        *self = Self::new();
    }
}

enum Systems {
    Pending,
    Ready { game: Game, minimap: Minimap },
    Failed,
}

struct Core {
    budget: FrameBudget,
    systems: Systems,
    settings: Option<Settings>,
    settings_path: Option<PathBuf>,
    keys: keys::Keys,
}

impl Core {
    fn new(settings: Settings, settings_path: Option<PathBuf>) -> Self {
        Self {
            budget: FrameBudget::new(),
            systems: Systems::Pending,
            settings: Some(settings),
            settings_path,
            keys: keys::Keys::default(),
        }
    }

    /// Reads the keyboard and runs the bound actions.
    fn handle_keys(keys: &mut keys::Keys, minimap: &mut Minimap) {
        let settings = minimap.settings();
        let mut watched: Vec<u8> = settings.bound_keys().collect();
        watched.extend([keys::ESCAPE, keys::ENTER, b'E', b'R']);
        if minimap.capturing_key() {
            watched.extend(settings::bindable_keys());
        }
        keys.poll(watched);
        if minimap.capturing_key() {
            return;
        }
        let settings = minimap.settings();
        let (toggle, zoom_in, zoom_out, menu, megazoom) = (
            keys.any_pressed(&settings.toggle_keys),
            keys.any_pressed(&settings.zoom_in_keys),
            keys.any_pressed(&settings.zoom_out_keys),
            keys.any_pressed(&settings.menu_keys),
            keys.any_pressed(&settings.megazoom_keys),
        );
        if megazoom {
            minimap.toggle_megazoom();
        }
        if toggle {
            minimap.toggle();
        }
        if zoom_in {
            minimap.zoom(1);
        }
        if zoom_out {
            minimap.zoom(-1);
        }
        if menu {
            minimap.toggle_menu();
        }
    }

    fn tick(&mut self, engine: Obj) {
        if matches!(self.systems, Systems::Pending) {
            let settings = self.settings.take().unwrap_or_default();
            self.systems = match Game::new(engine).and_then(|game| Ok((game, Minimap::new(settings, engine)?))) {
                Ok((game, minimap)) => {
                    host::info("game functions resolved");
                    *lock(&ACTOR_FILTER) = minimap.actor_filter();
                    Systems::Ready { game, minimap }
                }
                Err(reason) => {
                    host::error(&format!("could not resolve game functions, minimap disabled: {reason}"));
                    Systems::Failed
                }
            };
        }
        let Systems::Ready { game, minimap } = &mut self.systems else { return };
        profile::time("keys", || Self::handle_keys(&mut self.keys, minimap));
        let events = std::mem::take(&mut *lock(&ACTOR_EVENTS));
        profile::time("actor events", || {
            for (began, actor) in events {
                if began {
                    minimap.actor_began(actor);
                } else {
                    minimap.actor_ended(actor);
                }
            }
        });
        if let Some(player) = profile::time("player", || game.player(engine)) {
            profile::time("minimap", || minimap.update(&player, &self.keys));
        }
        if let (Some(changed), Some(path)) = (minimap.settings_to_save(), self.settings_path.as_deref()) {
            profile::time("settings save", || changed.save(path));
        }
    }

    fn world_unload(&mut self) {
        let _scope = profile::scope("world unload");
        lock(&ACTOR_EVENTS).clear();
        if let Systems::Ready { minimap, .. } = &mut self.systems {
            minimap.world_unload();
        }
    }
}

static CORE: Mutex<Option<Core>> = Mutex::new(None);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn core() -> MutexGuard<'static, Option<Core>> {
    lock(&CORE)
}

fn record_actor(actor: *mut c_void, began: bool) {
    let _scope = profile::scope(if began { "actor spawn hook" } else { "actor despawn hook" });
    let Some(actor) = Obj::from_raw(actor) else { return };
    let tracked = lock(&ACTOR_FILTER).iter().any(|&class| {
        Obj::from_raw(class as *mut c_void).is_some_and(|class| ue::is_a(actor, class))
    });
    if tracked {
        lock(&ACTOR_EVENTS).push((began, ue::Weak::new(actor)));
    }
}

fn guarded(entry: &str, work: impl FnOnce()) {
    if panic::catch_unwind(AssertUnwindSafe(work)).is_err() {
        host::error(&format!("{entry} failed, minimap stopped"));
        *core() = None;
    }
}

fn install_panic_logger() {
    panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map(|l| format!(" at {}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".to_string());
        host::error(&format!("panic: {message}{location}"));
    }));
}

#[unsafe(no_mangle)]
pub extern "C" fn smm_init(api: *const host::HostApi) -> bool {
    if api.is_null() {
        return false;
    }
    let api = unsafe { *api };
    if api.version != host::API_VERSION || !host::install(api) {
        return false;
    }
    install_panic_logger();
    let settings_path = settings::path();
    let settings = match settings_path.as_deref() {
        Some(path) => Settings::load(path),
        None => {
            host::warning("mod folder not found, using default settings");
            Settings::default()
        }
    };
    *core() = Some(Core::new(settings, settings_path));
    host::info(concat!("core ", env!("CARGO_PKG_VERSION"), " started"));
    true
}

#[unsafe(no_mangle)]
pub extern "C" fn smm_tick(engine: *mut c_void, _delta_seconds: f32) {
    guarded("tick", || {
        let started = Instant::now();
        let Some(engine) = Obj::from_raw(engine) else { return };
        let mut guard = core();
        let Some(core) = guard.as_mut() else { return };
        core.tick(engine);
        core.budget.record(started.elapsed());
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn smm_world_unload() {
    guarded("world unload", || {
        if let Some(core) = core().as_mut() {
            core.world_unload();
        }
    });
}

// Keys are polled in Rust now (keys.rs). Kept because the shim still links against it.
#[unsafe(no_mangle)]
pub extern "C" fn smm_input(_action: u32) {}

#[unsafe(no_mangle)]
pub extern "C" fn smm_actor_begin(actor: *mut c_void) {
    let _ = panic::catch_unwind(|| record_actor(actor, true));
}

#[unsafe(no_mangle)]
pub extern "C" fn smm_actor_end(actor: *mut c_void) {
    let _ = panic::catch_unwind(|| record_actor(actor, false));
}

#[unsafe(no_mangle)]
pub extern "C" fn smm_shutdown() {
    guarded("shutdown", || {
        *core() = None;
        host::info("core stopped");
    });
}
