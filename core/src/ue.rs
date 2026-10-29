use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::NonNull;

use crate::host;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Obj(NonNull<c_void>);

// Only dereferenced inside engine callbacks, which all run on the game thread.
unsafe impl Send for Obj {}

impl Obj {
    pub fn from_raw(raw: *mut c_void) -> Option<Self> {
        NonNull::new(raw).map(Self)
    }

    pub fn raw(self) -> *mut c_void {
        self.0.as_ptr()
    }

    /// # Safety
    /// `offset` must be the offset of a `T` field of this live object.
    pub unsafe fn read<T: Copy>(self, offset: usize) -> T {
        unsafe { self.raw().cast::<u8>().add(offset).cast::<T>().read_unaligned() }
    }

    /// # Safety
    /// `offset` must be the offset of a `T` field of this live object.
    pub unsafe fn write<T: Copy>(self, offset: usize, value: T) {
        unsafe { self.raw().cast::<u8>().add(offset).cast::<T>().write_unaligned(value) }
    }

    /// # Safety
    /// `offset` must be the offset of an object pointer field of this live object.
    pub unsafe fn read_obj(self, offset: usize) -> Option<Obj> {
        Obj::from_raw(unsafe { self.read::<*mut c_void>(offset) })
    }

    pub fn name(self) -> String {
        let mut buffer = [0u8; 256];
        let len = unsafe { (host::api().object_name)(self.raw(), buffer.as_mut_ptr(), buffer.len()) };
        String::from_utf8_lossy(&buffer[..len.min(buffer.len())]).into_owned()
    }

    pub fn class(self) -> Option<Obj> {
        Obj::from_raw(unsafe { (host::api().object_class)(self.raw()) })
    }
}

pub fn find(path: &str) -> Result<Obj, String> {
    Obj::from_raw(unsafe { (host::api().find_object)(path.as_ptr(), path.len()) })
        .ok_or_else(|| format!("{path} not found"))
}

pub fn load(path: &str) -> Result<Obj, String> {
    Obj::from_raw(unsafe { (host::api().load_object)(path.as_ptr(), path.len()) })
        .ok_or_else(|| format!("{path} could not be loaded"))
}

pub fn construct(class: Obj, outer: Obj) -> Result<Obj, String> {
    Obj::from_raw(unsafe { (host::api().construct_object)(class.raw(), outer.raw()) })
        .ok_or_else(|| format!("could not construct a {}", class.name()))
}

pub type SoftObjectPtr = [u8; 0x30];

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Name(u64);

impl Name {
    pub fn new(text: &str) -> Self {
        Self(unsafe { (host::api().make_name)(text.as_ptr(), text.len()) })
    }

    pub fn text(self) -> String {
        read_string(|buffer, capacity| unsafe { (host::api().name_string)(self.0, buffer, capacity) })
    }
}

fn read_string(fill: impl FnOnce(*mut u8, usize) -> usize) -> String {
    let mut buffer = [0u8; 512];
    let len = fill(buffer.as_mut_ptr(), buffer.len()).min(buffer.len());
    String::from_utf8_lossy(&buffer[..len]).into_owned()
}

/// # Safety
/// `soft_object_ptr` must point at a live TSoftObjectPtr.
pub unsafe fn soft_path(soft_object_ptr: *const u8) -> String {
    read_string(|buffer, capacity| unsafe {
        (host::api().soft_object_path)(soft_object_ptr.cast(), buffer, capacity)
    })
}

pub fn is_a(object: Obj, class: Obj) -> bool {
    unsafe { (host::api().is_a)(object.raw(), class.raw()) }
}

/// First live (non-default) object of the class with this short name. Walks the whole object list.
pub fn find_instance(class_name: &str) -> Option<Obj> {
    Obj::from_raw(unsafe { (host::api().find_instance)(class_name.as_ptr(), class_name.len()) })
}

/// Copies a render target's pixels (BGRA) back from the GPU. Blocks until the GPU catches up.
pub fn read_render_target(world: Obj, render_target: Obj, pixels: &mut Vec<u32>) -> Result<(), String> {
    let capacity = pixels.capacity();
    let count = unsafe {
        (host::api().read_render_target)(
            world.raw(),
            render_target.raw(),
            pixels.as_mut_ptr(),
            i32::try_from(capacity).map_err(|_| "render target too large")?,
        )
    };
    let count = usize::try_from(count).map_err(|_| "ReadRenderTarget failed".to_string())?;
    unsafe { pixels.set_len(count.min(capacity)) };
    Ok(())
}

pub fn set_bool_property(object: Obj, name: &str, value: bool) -> bool {
    unsafe { (host::api().set_bool_property)(object.raw(), name.as_ptr(), name.len(), value) }
}

/// Sets a bool, bitfield or not, inside a struct value of type `ustruct` at `container`.
///
/// # Safety
/// `container` must point at a live value of that struct.
pub unsafe fn set_struct_bool(ustruct: Obj, container: *mut c_void, name: &str, value: bool) -> bool {
    unsafe { (host::api().set_struct_bool)(ustruct.raw(), container, name.as_ptr(), name.len(), value) }
}

fn fill_pointers(
    out: &mut Vec<*mut c_void>,
    mut fetch: impl FnMut(*mut *mut c_void, i32) -> i32,
) -> Option<()> {
    let mut capacity = out.capacity().max(64);
    loop {
        out.clear();
        out.reserve(capacity);
        let available = out.capacity();
        let count = usize::try_from(fetch(out.as_mut_ptr(), i32::try_from(available).ok()?)).ok()?;
        if count <= available {
            unsafe { out.set_len(count) };
            return Some(());
        }
        capacity = count;
    }
}

/// Pairs of the map property `property` on `container`; returns the value offset inside each pair.
pub fn map_entries(owner: Obj, property: &str, container: *mut c_void, pairs: &mut Vec<*mut c_void>) -> Result<usize, String> {
    let mut value_offset = 0i32;
    fill_pointers(pairs, |out, capacity| unsafe {
        (host::api().map_entries)(
            owner.raw(),
            property.as_ptr(),
            property.len(),
            container,
            out,
            capacity,
            &mut value_offset,
        )
    })
    .ok_or_else(|| format!("{} has no readable map {property}", owner.name()))?;
    usize::try_from(value_offset).map_err(|_| format!("{property} has a negative value offset"))
}

pub fn data_table_rows(table: Obj, rows: &mut Vec<*mut c_void>) -> Result<(), String> {
    fill_pointers(rows, |out, capacity| unsafe { (host::api().data_table_rows)(table.raw(), out, capacity) })
        .ok_or_else(|| format!("{} rows could not be read", table.name()))
}

/// # Safety
/// `base + offset` must hold a `T`.
pub unsafe fn read_at<T: Copy>(base: *mut c_void, offset: usize) -> T {
    unsafe { base.cast::<u8>().add(offset).cast::<T>().read_unaligned() }
}

pub fn property_offset(owner: Obj, name: &str) -> Result<usize, String> {
    let offset = unsafe { (host::api().property_offset)(owner.raw(), name.as_ptr(), name.len()) };
    usize::try_from(offset).map_err(|_| format!("{} has no property {name}", owner.name()))
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Weak {
    index: i32,
    serial: i32,
}

impl Weak {
    pub fn new(object: Obj) -> Self {
        let mut out = [0i32; 2];
        unsafe { (host::api().weak_make)(object.raw(), out.as_mut_ptr()) };
        Self {
            index: out[0],
            serial: out[1],
        }
    }

    pub fn get(self) -> Option<Obj> {
        Obj::from_raw(unsafe { (host::api().weak_get)(self.index, self.serial) })
    }
}

pub struct Function {
    name: &'static str,
    function: Obj,
    size: usize,
    params: Vec<usize>,
    return_offset: Option<usize>,
    buffer: Vec<u128>,
}

impl Function {
    pub fn resolve(owner: Obj, name: &'static str, params: &[&str]) -> Result<Self, String> {
        let function = Obj::from_raw(unsafe { (host::api().find_function)(owner.raw(), name.as_ptr(), name.len()) })
            .ok_or_else(|| format!("{} has no function {name}", owner.name()))?;
        let size = unsafe { (host::api().function_params_size)(function.raw()) };
        let size = usize::try_from(size).map_err(|_| format!("{name} has no parameter size"))?;
        let params = params
            .iter()
            .map(|param| property_offset(function, param))
            .collect::<Result<Vec<_>, _>>()?;
        let return_offset = property_offset(function, "ReturnValue").ok();
        Ok(Self {
            name,
            function,
            size,
            params,
            return_offset,
            buffer: vec![0; size.div_ceil(size_of::<u128>()).max(1)],
        })
    }

    pub fn call(&mut self, target: Obj, args: impl FnOnce(&mut Args<'_>)) -> &mut Self {
        self.buffer.fill(0);
        let mut filled = Args {
            name: self.name,
            bytes: unsafe { std::slice::from_raw_parts_mut(self.buffer.as_mut_ptr().cast::<u8>(), self.size) },
            offsets: &self.params,
        };
        args(&mut filled);
        unsafe { (host::api().process_event)(target.raw(), self.function.raw(), self.buffer.as_mut_ptr().cast()) };
        self
    }

    /// Raw bytes of a parameter, up to `max` and never past the end of the parameter block.
    pub fn param_bytes(&self, index: usize, max: usize) -> Vec<u8> {
        let offset = self.params[index];
        let len = max.min(self.size.saturating_sub(offset));
        let bytes = unsafe { std::slice::from_raw_parts(self.buffer.as_ptr().cast::<u8>(), self.size) };
        bytes[offset..offset + len].to_vec()
    }

    pub fn param<T: Copy>(&self, index: usize) -> T {
        let offset = self.params[index];
        assert!(offset + size_of::<T>() <= self.size, "{} parameter {index} read out of bounds", self.name);
        unsafe { self.buffer.as_ptr().cast::<u8>().add(offset).cast::<T>().read_unaligned() }
    }

    /// A field of a struct parameter, `field` bytes into it.
    pub fn param_field<T: Copy>(&self, index: usize, field: usize) -> T {
        let offset = self.params[index] + field;
        assert!(offset + size_of::<T>() <= self.size, "{} parameter {index} field read out of bounds", self.name);
        unsafe { self.buffer.as_ptr().cast::<u8>().add(offset).cast::<T>().read_unaligned() }
    }

    pub fn returned<T: Copy>(&self) -> T {
        let offset = self
            .return_offset
            .unwrap_or_else(|| panic!("{} has no return value", self.name));
        assert!(offset + size_of::<T>() <= self.size, "{} return value read out of bounds", self.name);
        unsafe { self.buffer.as_ptr().cast::<u8>().add(offset).cast::<T>().read_unaligned() }
    }
}

pub struct Args<'a> {
    name: &'static str,
    bytes: &'a mut [u8],
    offsets: &'a [usize],
}

impl Args<'_> {
    pub fn set<T: Copy>(&mut self, index: usize, value: T) {
        let offset = self.offsets[index];
        assert!(offset + size_of::<T>() <= self.bytes.len(), "{} argument {index} out of bounds", self.name);
        unsafe { self.bytes.as_mut_ptr().add(offset).cast::<T>().write_unaligned(value) };
    }

    pub fn set_obj(&mut self, index: usize, object: Option<Obj>) {
        self.set(index, object.map_or(std::ptr::null_mut(), Obj::raw));
    }

    pub fn set_bytes(&mut self, index: usize, value: &[u8]) {
        let offset = self.offsets[index];
        let len = value.len().min(self.bytes.len().saturating_sub(offset));
        self.bytes[offset..offset + len].copy_from_slice(&value[..len]);
    }

    /// Points a TArray<uint8> parameter at `data`. The engine only reads it, so it never frees our memory.
    pub fn set_byte_array(&mut self, index: usize, data: &[u8]) {
        let num = i32::try_from(data.len()).expect("byte array too large");
        let array = ScriptArray {
            data: data.as_ptr().cast_mut().cast(),
            num,
            max: num,
        };
        self.set(index, array);
    }

    /// Points an FString parameter at `text`, which must end in a 0. The engine copies it.
    pub fn set_string(&mut self, index: usize, text: &[u16]) {
        let num = i32::try_from(text.len()).expect("string too long");
        let array = ScriptArray {
            data: text.as_ptr().cast_mut().cast(),
            num,
            max: num,
        };
        self.set(index, array);
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ScriptArray {
    data: *mut c_void,
    num: i32,
    max: i32,
}

/// An FText, copied as raw bytes. Never destroyed, so the text it shares is never freed under the engine.
pub type Text = [u8; 0x18];

/// An FString the engine returned. Its memory belongs to the engine and is left alone.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct EngineString {
    data: *const u16,
    num: i32,
    max: i32,
}

impl EngineString {
    pub fn text(self) -> String {
        let Ok(len) = usize::try_from(self.num) else { return String::new() };
        if self.data.is_null() || len == 0 {
            return String::new();
        }
        let chars = unsafe { std::slice::from_raw_parts(self.data, len) };
        String::from_utf16_lossy(chars.strip_suffix(&[0]).unwrap_or(chars))
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Margin {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Margin {
    pub fn all(value: f32) -> Self {
        Self { left: value, top: value, right: value, bottom: value }
    }

    pub fn xy(x: f32, y: f32) -> Self {
        Self { left: x, top: y, right: x, bottom: y }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ChildSize {
    pub value: f32,
    pub rule: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SlateColor {
    pub color: LinearColor,
    pub rule: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Guid {
    pub parts: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Anchors {
    pub min: Vec2,
    pub max: Vec2,
}

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Rotator {
    pub pitch: f64,
    pub yaw: f64,
    pub roll: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct LinearColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}
