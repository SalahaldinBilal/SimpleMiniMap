use std::collections::HashMap;
use std::ffi::c_void;
use std::time::{Duration, Instant};

use crate::host;
use crate::profile;
use crate::ue::{self, Function, Name, Obj, SoftObjectPtr, Weak};

const LOAD_DIRECTLY_AFTER: Duration = Duration::from_secs(5);
const NO_LINK: i32 = -1;

pub enum Load<T = Obj> {
    Ready(T),
    Waiting,
    Failed,
}

impl<T> Load<T> {
    pub fn ready(self) -> Option<T> {
        match self {
            Load::Ready(value) => Some(value),
            Load::Waiting | Load::Failed => None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SoftObjectPath {
    package: Name,
    asset: Name,
    sub_path: [u64; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LatentActionInfo {
    linkage: i32,
    uuid: i32,
    execution_function: Name,
    callback_target: *mut c_void,
}

#[derive(Clone, Copy)]
enum Entry {
    Loaded(Weak),
    Requested { soft: SoftObjectPtr, since: Instant },
    Failed,
}

/// Game assets by path, loaded in the background through the engine's own Blueprint async load.
pub struct Assets {
    system_library: Obj,
    to_soft: Function,
    soft_to_object: Function,
    load_async: Option<Function>,
    no_function: Name,
    next_id: i32,
    entries: HashMap<String, Entry>,
}

impl Assets {
    pub fn new(engine: Obj) -> Result<Self, String> {
        let system_library = ue::find("/Script/Engine.Default__KismetSystemLibrary")?;
        let engine_class = ue::find("/Script/Engine.Engine")?;
        let asset_manager = unsafe { engine.read_obj(ue::property_offset(engine_class, "AssetManager")?) };
        // LoadAsset is a fatal engine error without an asset manager.
        let load_async = match asset_manager {
            Some(_) => Some(Function::resolve(
                system_library,
                "LoadAsset",
                &["WorldContextObject", "Asset", "LatentInfo"],
            )?),
            None => {
                host::warning("the game has no asset manager, assets load on the game thread");
                None
            }
        };
        Ok(Self {
            to_soft: Function::resolve(system_library, "Conv_SoftObjPathToSoftObjRef", &["SoftObjectPath"])?,
            soft_to_object: Function::resolve(system_library, "Conv_SoftObjectReferenceToObject", &["SoftObject"])?,
            system_library,
            load_async,
            no_function: Name::new("None"),
            next_id: 0,
            entries: HashMap::new(),
        })
    }

    /// The asset if it is in memory. Otherwise starts a background load and reports `Waiting` until it lands.
    pub fn get(&mut self, world: Obj, path: &str) -> Load {
        match self.entries.get(path).copied() {
            Some(Entry::Loaded(weak)) => {
                if let Some(object) = weak.get() {
                    return Load::Ready(object);
                }
            }
            Some(Entry::Requested { soft, since }) => return self.poll(path, soft, since),
            Some(Entry::Failed) => return Load::Failed,
            None => {}
        }

        let _scope = profile::scope("start asset load");
        let Some(soft) = self.soft(path) else { return self.load_now(path) };
        if let Some(object) = self.in_memory(soft) {
            return self.loaded(path, object);
        }
        let Some(load_async) = self.load_async.as_mut() else { return self.load_now(path) };
        self.next_id = self.next_id.wrapping_add(1);
        let info = LatentActionInfo {
            linkage: NO_LINK,
            uuid: self.next_id,
            execution_function: self.no_function,
            callback_target: world.raw(),
        };
        load_async.call(self.system_library, |a| {
            a.set_obj(0, Some(world));
            a.set(1, soft);
            a.set(2, info);
        });
        self.entries.insert(path.to_owned(), Entry::Requested { soft, since: Instant::now() });
        Load::Waiting
    }

    /// Background loads die with the world that started them.
    pub fn world_unload(&mut self) {
        self.entries.retain(|_, entry| !matches!(entry, Entry::Requested { .. }));
    }

    fn poll(&mut self, path: &str, soft: SoftObjectPtr, since: Instant) -> Load {
        if let Some(object) = self.in_memory(soft) {
            host::info(&format!(
                "{} loaded in the background in {} ms",
                short_name(path),
                since.elapsed().as_millis()
            ));
            return self.loaded(path, object);
        }
        if since.elapsed() < LOAD_DIRECTLY_AFTER {
            return Load::Waiting;
        }
        host::warning(&format!("{} did not load in the background, loading it directly", short_name(path)));
        self.load_now(path)
    }

    fn soft(&mut self, path: &str) -> Option<SoftObjectPtr> {
        let (package, asset) = path.rsplit_once('.')?;
        let path = SoftObjectPath {
            package: Name::new(package),
            asset: Name::new(asset),
            sub_path: [0; 2],
        };
        Some(self.to_soft.call(self.system_library, |a| a.set(0, path)).returned())
    }

    fn in_memory(&mut self, soft: SoftObjectPtr) -> Option<Obj> {
        let object: *mut c_void = self.soft_to_object.call(self.system_library, |a| a.set(0, soft)).returned();
        Obj::from_raw(object)
    }

    fn loaded(&mut self, path: &str, object: Obj) -> Load {
        self.entries.insert(path.to_owned(), Entry::Loaded(Weak::new(object)));
        Load::Ready(object)
    }

    fn load_now(&mut self, path: &str) -> Load {
        match profile::time("direct asset load", || ue::load(path)) {
            Ok(object) => self.loaded(path, object),
            Err(reason) => {
                host::warning(&format!("asset unavailable: {reason}"));
                self.entries.insert(path.to_owned(), Entry::Failed);
                Load::Failed
            }
        }
    }
}

fn short_name(path: &str) -> &str {
    path.rsplit_once('.').map_or(path, |(_, name)| name)
}
