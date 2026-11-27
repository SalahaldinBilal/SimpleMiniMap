use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::c_void;
use std::fmt::Write as _;
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::host;
use crate::ue::{self, Function, Obj, Weak};

const TILE_PIXELS: u32 = 512;
const INCOMING: &str = "incoming";
const INDEX: &str = "index.txt";
const INDEX_DELAY: Duration = Duration::from_secs(5);
const WORKER_POLL: Duration = Duration::from_millis(100);
// How long the engine gets to write a captured tile's file before the save counts as failed.
const FILE_WAIT: Duration = Duration::from_secs(15);
const CAPTURE_WAIT: Duration = Duration::from_secs(10);
const MAX_WRITER_FAILURES: u32 = 3;
// Each save's frames are logged against the frames before it, to see whether saving costs a hitch.
const FRAMES_BEFORE: usize = 30;
const FRAMES_AFTER_STOP: u32 = 5;
// Halfway between captures, so a save's frames do not include a capture's.
const SAVE_DELAY: Duration = Duration::from_millis(300);
const PNG_FORMAT: u8 = 0;
const OVERRUN_SKIP: u8 = 1;
const PIXEL_FORMAT_B8G8R8A8: i32 = 0;
const STATE_ERROR: u8 = 0;
const STATE_STOPPED: u8 = 4;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CacheKey {
    pub x: i32,
    pub y: i32,
    pub level: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub fingerprint: Option<u64>,
    pub build: u64,
    pub saved_at: u64,
}

enum Job {
    Load { key: CacheKey, path: PathBuf },
    Collect { key: CacheKey, incoming: PathBuf, prefix: String, dest: PathBuf },
    WriteIndex { path: PathBuf, text: String },
    Clear { root: PathBuf, look: PathBuf, incoming: PathBuf },
    Prune { root: PathBuf, keep: Vec<String> },
}

enum Done {
    Loaded { key: CacheKey, png: Vec<u8>, decode: Duration },
    Missing { key: CacheKey, reason: Option<String> },
    Saved { key: CacheKey, saved_at: u64, bytes: u64, waited: Duration },
    SaveFailed { key: CacheKey, reason: String },
    Cleared,
    Pruned { removed: usize, tiles: usize, bytes: u64 },
    Failed(String),
}

/// A finished tile waiting for its turn to be written out.
struct PendingSave {
    key: CacheKey,
    target: Weak,
    ready_at: Instant,
}

/// One tile on its way to disk through the engine's media capture: copied off the GPU a few frames later,
/// without waiting for it, and written as PNG by the engine's image write queue.
struct Active {
    key: CacheKey,
    capture: Weak,
    started: Instant,
}

/// The engine's file media output and capture, to write render targets to disk without a readback.
struct Writer {
    owner: Weak,
    output_class: Obj,
    directory_path: Obj,
    image_options: Obj,
    file_path_offset: usize,
    write_options_offset: usize,
    format_offset: usize,
    quality_offset: usize,
    desired_size_offset: usize,
    desired_format_offset: usize,
    create_capture: Function,
    capture_target: Function,
    get_state: Function,
    options: Vec<u8>,
    failures: u32,
    /// The output and capture, made once and reused for every tile: a new capture costs the game about a frame.
    kept: Option<(Weak, Weak)>,
}

impl Writer {
    fn new(owner: Obj) -> Result<Self, String> {
        let output_class = ue::find("/Script/MediaIOCore.FileMediaOutput")?;
        let capture_class = ue::find("/Script/MediaIOCore.MediaCapture")?;
        let options_struct = ue::find("/Script/MediaIOCore.MediaCaptureOptions")?;
        let image_options = ue::find("/Script/ImageWriteQueue.ImageWriteOptions")?;
        let offset = |name| ue::property_offset(options_struct, name);
        let frames_offset = offset("NumberOfFramesToCapture")?;
        let mut options = vec![0u8; frames_offset + 4];
        options[offset("OverrunAction")?] = OVERRUN_SKIP;
        options[offset("bConvertToDesiredPixelFormat")?] = 1;
        options[offset("bAutostopOnCapture")?] = 1;
        options[frames_offset..frames_offset + 4].copy_from_slice(&1i32.to_le_bytes());
        for zero in ["Crop", "bResizeSourceBuffer", "bSkipFrameWhenRunningExpensiveTasks", "bForceAlphaToOneOnConversion", "bApplyLinearToSRGBConversion"] {
            options[offset(zero)?] = 0;
        }
        Ok(Self {
            owner: Weak::new(owner),
            directory_path: ue::find("/Script/CoreUObject.DirectoryPath")?,
            file_path_offset: ue::property_offset(output_class, "FilePath")?,
            write_options_offset: ue::property_offset(output_class, "WriteOptions")?,
            format_offset: ue::property_offset(image_options, "Format")?,
            quality_offset: ue::property_offset(image_options, "CompressionQuality")?,
            desired_size_offset: ue::property_offset(output_class, "DesiredSize")?,
            desired_format_offset: ue::property_offset(output_class, "DesiredPixelFormat")?,
            create_capture: Function::resolve(output_class, "CreateMediaCapture", &[])?,
            capture_target: Function::resolve(capture_class, "CaptureTextureRenderTarget2D", &["RenderTarget", "CaptureOptions"])?,
            get_state: Function::resolve(capture_class, "GetState", &[])?,
            image_options,
            output_class,
            options,
            failures: 0,
            kept: None,
        })
    }

    /// The kept output and capture, made when missing. True when they were made just now.
    fn objects(&mut self) -> Result<(Obj, Obj, bool), String> {
        if let Some((output, capture)) = self.kept
            && let (Some(output), Some(capture)) = (output.get(), capture.get())
        {
            return Ok((output, capture, false));
        }
        self.forget();
        let owner = self.owner.get().ok_or("the engine is gone")?;
        let output = ue::construct(self.output_class, owner)?;
        ue::set_rooted(output, true);
        let raw: *mut c_void = self.create_capture.call(output, |_| {}).returned();
        let Some(capture) = Obj::from_raw(raw) else {
            ue::set_rooted(output, false);
            return Err("CreateMediaCapture made no capture".into());
        };
        ue::set_rooted(capture, true);
        self.kept = Some((Weak::new(output), Weak::new(capture)));
        Ok((output, capture, true))
    }

    /// Lets the kept output and capture go, so the next save makes new ones.
    fn forget(&mut self) {
        for object in self.kept.take().into_iter().flat_map(|(output, capture)| [capture, output]).filter_map(Weak::get) {
            ue::set_rooted(object, false);
        }
    }

    /// Starts writing `target` to `<folder>\<prefix><frame number>.png`. Also says whether the capture was new.
    fn start(&mut self, key: CacheKey, target: Obj, folder: &str, prefix: &str) -> Result<(Active, bool), String> {
        let (output, capture, made) = self.objects()?;
        if let Err(reason) = self.configure(output, folder, prefix) {
            self.forget();
            return Err(reason);
        }
        let options = self.options.clone();
        let accepted = self
            .capture_target
            .call(capture, |a| {
                a.set_obj(0, Some(target));
                a.set_bytes(1, &options);
            })
            .returned::<u8>()
            != 0;
        if !accepted {
            self.forget();
            return Err("the media capture did not accept the tile".into());
        }
        Ok((Active { key, capture: Weak::new(capture), started: Instant::now() }, made))
    }

    fn configure(&mut self, output: Obj, folder: &str, prefix: &str) -> Result<(), String> {
        let base = output.raw().cast::<u8>();
        let options = unsafe { base.add(self.write_options_offset) };
        unsafe {
            options.add(self.format_offset).write(PNG_FORMAT);
            options.add(self.quality_offset).cast::<i32>().write_unaligned(0);
            output.write(self.desired_size_offset, [TILE_PIXELS as i32; 2]);
            output.write(self.desired_format_offset, PIXEL_FORMAT_B8G8R8A8);
        }
        let flags = unsafe {
            ue::set_struct_bool(self.image_options, options.cast(), "bOverwriteFile", true)
                && ue::set_struct_bool(self.image_options, options.cast(), "bAsync", true)
        };
        let set = flags
            && ue::set_bool_property(output, "bOverrideDesiredSize", true)
            && ue::set_bool_property(output, "bOverridePixelFormat", true)
            && ue::set_bool_property(output, "bInvertAlpha", false)
            && unsafe { ue::set_string(self.directory_path, base.add(self.file_path_offset).cast(), "Path", folder) }
            && unsafe { ue::set_string(self.output_class, output.raw(), "BaseFileName", prefix) };
        if set { Ok(()) } else { Err("the file media output could not be set up".into()) }
    }

    fn state(&mut self, capture: Obj) -> u8 {
        self.get_state.call(capture, |_| {}).returned()
    }
}

/// Rendered terrain tiles kept on disk between sessions, one folder per look. Files are written by the engine
/// off the game thread, and read back and prepared for a cheap import by a worker thread.
pub struct TileCache {
    root: PathBuf,
    incoming: PathBuf,
    look: Option<String>,
    build: u64,
    index: HashMap<CacheKey, Entry>,
    index_changed: Option<Instant>,
    jobs: Sender<Job>,
    done: Receiver<Done>,
    loading: HashSet<CacheKey>,
    loaded: VecDeque<(CacheKey, Vec<u8>, Duration)>,
    pending: VecDeque<PendingSave>,
    /// Tiles from their queueing until written, with the fingerprint they were captured with.
    saving: HashMap<CacheKey, Option<u64>>,
    writer: Option<Writer>,
    active: Option<Active>,
    frames: VecDeque<Duration>,
    last_frame: Option<Instant>,
    watch: Option<Watch>,
}

impl TileCache {
    /// `owner` is what the save objects are made inside: something outside every world, like the engine.
    pub fn new(root: PathBuf, owner: Obj) -> Result<Self, String> {
        let incoming = root.join(INCOMING);
        fs::create_dir_all(&incoming).map_err(|error| format!("{} could not be made: {error}", incoming.display()))?;
        let writer = Writer::new(owner).inspect_err(|reason| host::warning(&format!("terrain tiles cannot be saved: {reason}"))).ok();
        let (jobs, worker_jobs) = mpsc::channel();
        let (worker_done, done) = mpsc::channel();
        thread::Builder::new()
            .name("smm-tiles".into())
            .spawn(move || run_worker(&worker_jobs, &worker_done))
            .map_err(|error| format!("the tile cache thread could not start: {error}"))?;
        Ok(Self {
            root,
            incoming,
            look: None,
            build: build_stamp(),
            index: HashMap::new(),
            index_changed: None,
            jobs,
            done,
            loading: HashSet::new(),
            loaded: VecDeque::new(),
            pending: VecDeque::new(),
            saving: HashMap::new(),
            writer,
            active: None,
            frames: VecDeque::new(),
            last_frame: None,
            watch: None,
        })
    }

    fn folder(&self) -> Option<PathBuf> {
        self.look.as_ref().map(|look| self.root.join(look))
    }

    fn tile_path(&self, key: CacheKey) -> Option<PathBuf> {
        self.folder().map(|folder| folder.join(key.level.to_string()).join(format!("{}_{}.png", key.x, key.y)))
    }

    /// Switches to the cache folder of a look, reading its index. The first call also removes the folders of
    /// looks no longer made.
    pub fn set_look(&mut self, look: &str, keep: &[String]) {
        if self.look.as_deref() == Some(look) {
            return;
        }
        let first = self.look.is_none();
        self.write_index();
        self.look = Some(look.to_string());
        self.loading.clear();
        self.loaded.clear();
        self.pending.clear();
        self.saving.retain(|key, _| self.active.as_ref().is_some_and(|active| active.key == *key));
        self.index = self.read_index();
        if first {
            let _ = self.jobs.send(Job::Prune { root: self.root.clone(), keep: keep.to_vec() });
        }
    }

    fn read_index(&self) -> HashMap<CacheKey, Entry> {
        let mut index = HashMap::new();
        let Some(folder) = self.folder() else { return index };
        let Ok(text) = fs::read_to_string(folder.join(INDEX)) else { return index };
        let mut skipped = 0;
        for line in text.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')) {
            match parse_line(line) {
                Some((key, entry)) => {
                    index.insert(key, entry);
                }
                None => skipped += 1,
            }
        }
        let stale = index.values().filter(|entry| entry.build != self.build).count();
        host::info(&format!(
            "terrain cache {}: {} tiles, {stale} from another game build{}",
            self.look.as_deref().unwrap_or_default(),
            index.len(),
            if skipped > 0 { format!(", {skipped} bad index lines skipped") } else { String::new() }
        ));
        index
    }

    fn write_index(&mut self) {
        if self.index_changed.take().is_none() {
            return;
        }
        let Some(folder) = self.folder() else { return };
        let mut text = format!("# SimpleMiniMap terrain cache, look {}\n", self.look.as_deref().unwrap_or_default());
        let mut keys: Vec<&CacheKey> = self.index.keys().collect();
        keys.sort_by_key(|key| (key.level, key.x, key.y));
        for key in keys {
            let entry = self.index[key];
            let fingerprint = entry.fingerprint.map_or_else(|| "-".to_string(), |value| format!("{value:016x}"));
            let _ = writeln!(text, "C {} {} {} b={:016x} f={fingerprint} t={}", key.level, key.x, key.y, entry.build, entry.saved_at);
        }
        let _ = self.jobs.send(Job::WriteIndex { path: folder.join(INDEX), text });
    }

    pub fn entry(&self, key: CacheKey) -> Option<Entry> {
        self.index.get(&key).copied()
    }

    /// Whether a cached tile was made by another game build, or with other structures in it than now.
    pub fn stale(&self, entry: Entry, fingerprint: Option<u64>) -> bool {
        entry.build != self.build || matches!((entry.fingerprint, fingerprint), (Some(saved), Some(now)) if saved != now)
    }

    /// Asks for a tile from disk. False when it is not cached.
    pub fn request(&mut self, key: CacheKey) -> bool {
        if self.loading.contains(&key) {
            return true;
        }
        let Some(path) = self.index.contains_key(&key).then(|| self.tile_path(key)).flatten() else { return false };
        self.loading.insert(key);
        let _ = self.jobs.send(Job::Load { key, path });
        true
    }

    pub fn is_loading(&self, key: CacheKey) -> bool {
        self.loading.contains(&key)
    }

    /// The next tile read from disk, as an uncompressed PNG, and how long the worker took to prepare it.
    pub fn take_loaded(&mut self) -> Option<(CacheKey, Vec<u8>, Duration)> {
        self.loaded.pop_front()
    }

    pub fn queue_save(&mut self, key: CacheKey, target: Obj, fingerprint: Option<u64>) {
        if self.writer.is_none() || self.look.is_none() {
            return;
        }
        self.pending.retain(|save| save.key != key);
        self.pending.push_back(PendingSave { key, target: Weak::new(target), ready_at: Instant::now() + SAVE_DELAY });
        self.saving.insert(key, fingerprint);
    }

    pub fn is_saving(&self, key: CacheKey) -> bool {
        self.saving.contains_key(&key)
    }

    /// Deletes every cached tile, of every look.
    pub fn clear(&mut self) {
        self.abandon(false);
        self.index.clear();
        self.index_changed = None;
        self.loading.clear();
        self.loaded.clear();
        if let Some(look) = self.folder() {
            let _ = self.jobs.send(Job::Clear { root: self.root.clone(), look, incoming: self.incoming.clone() });
        }
    }

    /// Drops saves in flight: their tiles are going away with the world. The capture goes too, as it still
    /// points at a tile of the old world.
    pub fn world_unload(&mut self) {
        self.abandon(true);
        self.write_index();
    }

    fn abandon(&mut self, forget_capture: bool) {
        self.active = None;
        if forget_capture && let Some(writer) = &mut self.writer {
            writer.forget();
        }
        self.pending.clear();
        self.saving.clear();
        self.watch = None;
    }

    /// Runs every frame: takes in the worker's results, moves saves along, and writes the index when due.
    pub fn tick(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.last_frame.replace(now) {
            let length = now - last;
            self.watch_frame(length);
            self.frames.push_back(length);
            if self.frames.len() > FRAMES_BEFORE {
                self.frames.pop_front();
            }
        }

        while let Ok(done) = self.done.try_recv() {
            self.receive(done);
        }
        self.advance_save(now);
        if self.index_changed.is_some_and(|since| now.duration_since(since) >= INDEX_DELAY) {
            self.write_index();
        }
    }

    fn receive(&mut self, done: Done) {
        match done {
            Done::Loaded { key, png, decode } => {
                if self.loading.remove(&key) {
                    self.loaded.push_back((key, png, decode));
                }
            }
            Done::Missing { key, reason } => {
                self.loading.remove(&key);
                if self.index.remove(&key).is_some() {
                    self.index_changed.get_or_insert_with(Instant::now);
                }
                if let Some(reason) = reason {
                    host::warning(&format!("cached terrain tile {},{} level {} dropped: {reason}", key.x, key.y, key.level));
                }
            }
            Done::Saved { key, saved_at, bytes, waited } => {
                if let Some(fingerprint) = self.saving.remove(&key) {
                    self.index.insert(key, Entry { fingerprint, build: self.build, saved_at });
                    self.index_changed.get_or_insert_with(Instant::now);
                }
                host::info(&format!(
                    "terrain tile {},{} level {} saved, {} KB, file written {:.0} ms after the capture finished",
                    key.x,
                    key.y,
                    key.level,
                    bytes / 1024,
                    waited.as_secs_f64() * 1000.0
                ));
            }
            Done::SaveFailed { key, reason } => {
                self.saving.remove(&key);
                host::warning(&format!("terrain tile {},{} level {} not saved: {reason}", key.x, key.y, key.level));
            }
            Done::Cleared => host::info("terrain cache cleared"),
            Done::Pruned { removed, tiles, bytes } => host::info(&format!(
                "terrain cache on disk: {tiles} tiles, {:.1} MB{}",
                bytes as f64 / (1024.0 * 1024.0),
                if removed > 0 { format!(", {removed} folders of old looks removed") } else { String::new() }
            )),
            Done::Failed(reason) => host::warning(&format!("terrain cache: {reason}")),
        }
    }

    fn advance_save(&mut self, now: Instant) {
        if let Some(active) = &self.active {
            let Some(writer) = self.writer.as_mut() else { return };
            let state = active.capture.get().map(|capture| writer.state(capture));
            let finished = state == Some(STATE_STOPPED);
            let failed = state.is_none_or(|state| state == STATE_ERROR) || now.duration_since(active.started) > CAPTURE_WAIT;
            if !finished && !failed {
                return;
            }
            let active = self.active.take().expect("checked above");
            if !finished {
                writer.forget();
            }
            if let Some(watch) = &mut self.watch {
                watch.frames_after_stop = Some(FRAMES_AFTER_STOP);
            }
            let key = active.key;
            if finished {
                writer.failures = 0;
                if let Some(dest) = self.tile_path(key) {
                    let _ = self.jobs.send(Job::Collect { key, incoming: self.incoming.clone(), prefix: self.prefix(key), dest });
                }
            } else {
                self.saving.remove(&key);
                self.writer_failed(&format!(
                    "the media capture {}",
                    if state.is_none() { "went away" } else if state == Some(STATE_ERROR) { "failed" } else { "never finished" }
                ));
            }
        }
        if self.active.is_some() {
            return;
        }
        while self.pending.front().is_some_and(|save| now >= save.ready_at) {
            let Some(save) = self.pending.pop_front() else { return };
            let Some(target) = save.target.get() else {
                self.saving.remove(&save.key);
                continue;
            };
            let prefix = self.prefix(save.key);
            let folder = self.incoming.to_string_lossy().into_owned();
            let Some(writer) = self.writer.as_mut() else { return };
            let started = Instant::now();
            let result = writer.start(save.key, target, &folder, &prefix);
            let cost = started.elapsed();
            match result {
                Ok((active, made)) => {
                    self.watch = Some(Watch { key: save.key, typical: self.typical_frame(), frames: Vec::new(), frames_after_stop: None });
                    host::info(&format!(
                        "terrain tile {},{} level {} save started with {} capture, {:.2} ms on the game thread",
                        save.key.x,
                        save.key.y,
                        save.key.level,
                        if made { "a new" } else { "the kept" },
                        cost.as_secs_f64() * 1000.0
                    ));
                    self.active = Some(active);
                }
                Err(reason) => {
                    self.saving.remove(&save.key);
                    self.writer_failed(&reason);
                }
            }
            return;
        }
    }

    fn writer_failed(&mut self, reason: &str) {
        host::warning(&format!("terrain tile not saved: {reason}"));
        let Some(writer) = self.writer.as_mut() else { return };
        writer.failures += 1;
        if writer.failures >= MAX_WRITER_FAILURES {
            host::warning("terrain tiles are not saved for the rest of this session");
            writer.forget();
            self.writer = None;
            self.pending.clear();
            self.saving.clear();
        }
    }

    fn prefix(&self, key: CacheKey) -> String {
        format!("{}_{}_{}_{}_", self.look.as_deref().unwrap_or_default(), key.level, key.x, key.y)
    }

    /// Logs every frame of a save, from its start to a few frames after the capture stopped, against the
    /// median frame just before it.
    fn watch_frame(&mut self, length: Duration) {
        let Some(watch) = &mut self.watch else { return };
        watch.frames.push(length);
        if let Some(left) = &mut watch.frames_after_stop {
            if *left > 0 {
                *left -= 1;
                return;
            }
        } else {
            return;
        }
        let watch = self.watch.take().expect("checked above");
        let frames: Vec<String> = watch.frames.iter().map(|length| format!("{:.1}", length.as_secs_f64() * 1000.0)).collect();
        host::info(&format!(
            "terrain tile {},{} level {} save frames: {} ms (median {:.1} ms over the {} frames before)",
            watch.key.x,
            watch.key.y,
            watch.key.level,
            frames.join(", "),
            watch.typical.as_secs_f64() * 1000.0,
            FRAMES_BEFORE
        ));
    }

    fn typical_frame(&self) -> Duration {
        let mut lengths: Vec<Duration> = self.frames.iter().copied().collect();
        lengths.sort_unstable();
        lengths.get(lengths.len() / 2).copied().unwrap_or_default()
    }
}

struct Watch {
    key: CacheKey,
    typical: Duration,
    frames: Vec<Duration>,
    frames_after_stop: Option<u32>,
}

impl Drop for TileCache {
    fn drop(&mut self) {
        self.abandon(true);
        self.write_index();
    }
}

fn parse_line(line: &str) -> Option<(CacheKey, Entry)> {
    let mut parts = line.split_whitespace();
    if parts.next()? != "C" {
        return None;
    }
    let level = parts.next()?.parse().ok()?;
    let x = parts.next()?.parse().ok()?;
    let y = parts.next()?.parse().ok()?;
    let (mut build, mut fingerprint, mut saved_at) = (None, None, None);
    for part in parts {
        let (name, value) = part.split_once('=')?;
        match name {
            "b" => build = Some(u64::from_str_radix(value, 16).ok()?),
            "f" => fingerprint = Some(if value == "-" { None } else { Some(u64::from_str_radix(value, 16).ok()?) }),
            "t" => saved_at = Some(value.parse().ok()?),
            _ => {}
        }
    }
    Some((CacheKey { x, y, level }, Entry { fingerprint: fingerprint?, build: build?, saved_at: saved_at? }))
}

/// Changes when the game is patched: the game exe's size and modified time.
fn build_stamp() -> u64 {
    let metadata = std::env::current_exe().and_then(fs::metadata);
    let Ok(metadata) = metadata else { return 0 };
    let modified = metadata.modified().ok().and_then(|time| time.duration_since(UNIX_EPOCH).ok()).map_or(0, |time| time.as_secs());
    let mut hash = Fnv::new();
    hash.write(&metadata.len().to_le_bytes());
    hash.write(&modified.to_le_bytes());
    hash.finish()
}

/// FNV-1a, the same in every session, unlike the standard library's seeded hasher.
pub struct Fnv(u64);

impl Fnv {
    pub fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |time| time.as_secs())
}

struct Collect {
    key: CacheKey,
    incoming: PathBuf,
    prefix: String,
    dest: PathBuf,
    deadline: Instant,
    started: Instant,
    last_size: Option<u64>,
}

fn run_worker(jobs: &Receiver<Job>, done: &Sender<Done>) {
    let mut collecting: Vec<Collect> = Vec::new();
    loop {
        let job = if collecting.is_empty() {
            match jobs.recv() {
                Ok(job) => Some(job),
                Err(_) => return,
            }
        } else {
            match jobs.recv_timeout(WORKER_POLL) {
                Ok(job) => Some(job),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        };
        let worked = panic::catch_unwind(AssertUnwindSafe(|| {
            if let Some(job) = job {
                handle(job, &mut collecting, done);
            }
            collecting.retain_mut(|collect| !poll_collect(collect, done));
        }));
        if worked.is_err() {
            collecting.clear();
            let _ = done.send(Done::Failed("the tile cache thread hit an error".into()));
        }
    }
}

fn handle(job: Job, collecting: &mut Vec<Collect>, done: &Sender<Done>) {
    match job {
        Job::Load { key, path } => {
            let started = Instant::now();
            let result = match fs::read(&path) {
                Ok(bytes) => decode_tile(&bytes).map(|rgba| stored_png(TILE_PIXELS, TILE_PIXELS, &rgba)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let _ = done.send(Done::Missing { key, reason: None });
                    return;
                }
                Err(error) => Err(error.to_string()),
            };
            let _ = done.send(match result {
                Ok(png) => Done::Loaded { key, png, decode: started.elapsed() },
                Err(reason) => {
                    let _ = fs::remove_file(&path);
                    Done::Missing { key, reason: Some(reason) }
                }
            });
        }
        Job::Collect { key, incoming, prefix, dest } => {
            let now = Instant::now();
            collecting.retain(|collect| collect.key != key);
            collecting.push(Collect { key, incoming, prefix, dest, deadline: now + FILE_WAIT, started: now, last_size: None });
        }
        Job::WriteIndex { path, text } => {
            let temporary = path.with_extension("tmp");
            if let Err(error) = fs::write(&temporary, text).and_then(|()| fs::rename(&temporary, &path)) {
                let _ = done.send(Done::Failed(format!("the index could not be written: {error}")));
            }
        }
        Job::Clear { root, look, incoming } => {
            collecting.clear();
            let _ = fs::remove_dir_all(&root);
            let made = fs::create_dir_all(&look).and_then(|()| fs::create_dir_all(&incoming));
            let _ = done.send(match made {
                Ok(()) => Done::Cleared,
                Err(error) => Done::Failed(format!("the cache folder could not be made again: {error}")),
            });
        }
        Job::Prune { root, keep } => {
            let mut removed = 0;
            let (mut tiles, mut bytes) = (0, 0);
            for entry in fs::read_dir(&root).into_iter().flatten().flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let path = entry.path();
                if name == INCOMING {
                    for file in fs::read_dir(&path).into_iter().flatten().flatten() {
                        let _ = fs::remove_file(file.path());
                    }
                } else if keep.contains(&name) {
                    count_files(&path, &mut tiles, &mut bytes);
                } else if path.is_dir() && fs::remove_dir_all(&path).is_ok() {
                    removed += 1;
                }
            }
            let _ = done.send(Done::Pruned { removed, tiles, bytes });
        }
    }
}

fn count_files(folder: &Path, tiles: &mut usize, bytes: &mut u64) {
    for entry in fs::read_dir(folder).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            count_files(&path, tiles, bytes);
        } else if path.extension().is_some_and(|extension| extension == "png") {
            *tiles += 1;
            *bytes += entry.metadata().map_or(0, |metadata| metadata.len());
        }
    }
}

/// Waits for the engine to finish writing a captured tile, checks it, and moves it into the cache. True when done.
fn poll_collect(collect: &mut Collect, done: &Sender<Done>) -> bool {
    let now = Instant::now();
    let found = fs::read_dir(&collect.incoming)
        .into_iter()
        .flatten()
        .flatten()
        .find(|entry| entry.file_name().to_string_lossy().starts_with(&collect.prefix));
    let Some(found) = found else {
        if now >= collect.deadline {
            let _ = done.send(Done::SaveFailed { key: collect.key, reason: "the engine wrote no file".into() });
            return true;
        }
        return false;
    };
    let path = found.path();
    let size = found.metadata().map_or(0, |metadata| metadata.len());
    // Written in one go by the engine's image write queue, so a size that held for one poll is finished.
    if collect.last_size != Some(size) {
        collect.last_size = Some(size);
        return false;
    }
    let checked = fs::read(&path).map_err(|error| error.to_string()).and_then(|bytes| decode_tile(&bytes).map(|_| bytes.len()));
    let result = checked.and_then(|bytes| {
        if let Some(parent) = collect.dest.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let _ = fs::remove_file(&collect.dest);
        fs::rename(&path, &collect.dest).map_err(|error| error.to_string())?;
        Ok(bytes as u64)
    });
    match result {
        Ok(bytes) => {
            let _ = done.send(Done::Saved { key: collect.key, saved_at: unix_now(), bytes, waited: now - collect.started });
            true
        }
        Err(_) if now < collect.deadline => false,
        Err(reason) => {
            let _ = fs::remove_file(&path);
            let _ = done.send(Done::SaveFailed { key: collect.key, reason });
            true
        }
    }
}

fn decode_tile(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).map_err(|error| error.to_string())?;
    if image.width() != TILE_PIXELS || image.height() != TILE_PIXELS {
        return Err(format!("{} by {} instead of {TILE_PIXELS} px", image.width(), image.height()));
    }
    Ok(image.to_rgba8().into_raw())
}

/// An RGBA PNG with no compression and no filters, so decoding it costs about a copy.
pub fn stored_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let row = width as usize * 4;
    let mut raw = Vec::with_capacity((row + 1) * height as usize);
    for line in rgba.chunks_exact(row).take(height as usize) {
        raw.push(0);
        raw.extend_from_slice(line);
    }
    let mut zlib = Vec::with_capacity(raw.len() + raw.len() / 65_535 * 5 + 16);
    zlib.extend_from_slice(&[0x78, 0x01]);
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (index, block) in blocks.iter().enumerate() {
        zlib.push(u8::from(index + 1 == blocks.len()));
        let len = block.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    for (kind, data) in [(b"IHDR", header.as_slice()), (b"IDAT", zlib.as_slice()), (b"IEND", &[][..])] {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = png.len();
        png.extend_from_slice(kind);
        png.extend_from_slice(data);
        let crc = crc32(&png[start..]);
        png.extend_from_slice(&crc.to_be_bytes());
    }
    png
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &byte in chunk {
            a += u32::from(byte);
            b += a;
        }
        a %= 65_521;
        b %= 65_521;
    }
    (b << 16) | a
}

fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = [0u32; 256];
        for (index, slot) in table.iter_mut().enumerate() {
            let mut value = index as u32;
            for _ in 0..8 {
                value = if value & 1 != 0 { 0xEDB8_8320 ^ (value >> 1) } else { value >> 1 };
            }
            *slot = value;
        }
        table
    });
    !data.iter().fold(!0u32, |crc, &byte| table[((crc ^ u32::from(byte)) & 0xFF) as usize] ^ (crc >> 8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_png_decodes_to_the_same_pixels() {
        let rgba: Vec<u8> = (0..TILE_PIXELS * TILE_PIXELS * 4).map(|index| (index.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        let png = stored_png(TILE_PIXELS, TILE_PIXELS, &rgba);
        assert_eq!(decode_tile(&png).expect("the stored PNG decodes"), rgba);
    }
}
