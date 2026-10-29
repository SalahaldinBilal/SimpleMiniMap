use std::ffi::c_void;
use std::sync::OnceLock;

pub const API_VERSION: u32 = 9;

// Field order must match SmmHostApi in shim/include/smm_host.h.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostApi {
    pub version: u32,
    pub log: unsafe extern "C" fn(level: u8, text: *const u8, len: usize),
    pub find_object: unsafe extern "C" fn(path: *const u8, len: usize) -> *mut c_void,
    pub load_object: unsafe extern "C" fn(path: *const u8, len: usize) -> *mut c_void,
    pub construct_object: unsafe extern "C" fn(class: *mut c_void, outer: *mut c_void) -> *mut c_void,
    pub find_function:
        unsafe extern "C" fn(object_or_struct: *mut c_void, name: *const u8, len: usize) -> *mut c_void,
    pub function_params_size: unsafe extern "C" fn(function: *mut c_void) -> i32,
    pub property_offset: unsafe extern "C" fn(ustruct: *mut c_void, name: *const u8, len: usize) -> i32,
    pub process_event: unsafe extern "C" fn(object: *mut c_void, function: *mut c_void, params: *mut c_void),
    pub weak_make: unsafe extern "C" fn(object: *mut c_void, out_index_and_serial: *mut i32),
    pub weak_get: unsafe extern "C" fn(index: i32, serial: i32) -> *mut c_void,
    pub object_name: unsafe extern "C" fn(object: *mut c_void, buffer: *mut u8, capacity: usize) -> usize,
    pub object_class: unsafe extern "C" fn(object: *mut c_void) -> *mut c_void,
    pub make_name: unsafe extern "C" fn(text: *const u8, len: usize) -> u64,
    pub map_entries: unsafe extern "C" fn(
        ustruct: *mut c_void,
        property: *const u8,
        len: usize,
        container: *mut c_void,
        out_pairs: *mut *mut c_void,
        capacity: i32,
        out_value_offset: *mut i32,
    ) -> i32,
    pub data_table_rows: unsafe extern "C" fn(table: *mut c_void, out_rows: *mut *mut c_void, capacity: i32) -> i32,
    pub soft_object_path: unsafe extern "C" fn(soft_object_ptr: *const c_void, buffer: *mut u8, capacity: usize) -> usize,
    pub name_string: unsafe extern "C" fn(name: u64, buffer: *mut u8, capacity: usize) -> usize,
    pub is_a: unsafe extern "C" fn(object: *mut c_void, class: *mut c_void) -> bool,
    pub bind_key: unsafe extern "C" fn(key: u8, modifiers: u8, action: u32),
    pub find_instance: unsafe extern "C" fn(class_name: *const u8, len: usize) -> *mut c_void,
    pub read_render_target:
        unsafe extern "C" fn(world: *mut c_void, render_target: *mut c_void, out_pixels: *mut u32, capacity: i32) -> i32,
    pub set_bool_property: unsafe extern "C" fn(object: *mut c_void, name: *const u8, len: usize, value: bool) -> bool,
    pub set_struct_bool:
        unsafe extern "C" fn(ustruct: *mut c_void, container: *mut c_void, name: *const u8, len: usize, value: bool) -> bool,
}

static HOST: OnceLock<HostApi> = OnceLock::new();

pub fn install(api: HostApi) -> bool {
    HOST.set(api).is_ok() || HOST.get().is_some()
}

pub fn api() -> &'static HostApi {
    HOST.get().expect("host api used before smm_init")
}

#[derive(Clone, Copy)]
#[repr(u8)]
pub enum Level {
    Info = 0,
    Warning = 1,
    Error = 2,
}

pub fn log(level: Level, message: &str) {
    if let Some(api) = HOST.get() {
        unsafe { (api.log)(level as u8, message.as_ptr(), message.len()) };
    }
}

pub fn info(message: &str) {
    log(Level::Info, message);
}

pub fn warning(message: &str) {
    log(Level::Warning, message);
}

pub fn error(message: &str) {
    log(Level::Error, message);
}
