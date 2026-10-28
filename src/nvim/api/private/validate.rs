//! API argument validation helpers (`src/nvim/api/private/validate.c`).

use std::ffi::{CStr, c_char, c_int};

use crate::memory::realloc_items;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NvimString {
    pub data: *mut c_char,
    pub size: usize,
}

impl NvimString {
    #[inline]
    pub const fn new() -> Self {
        Self {
            data: std::ptr::null_mut(),
            size: 0,
        }
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        if self.size == 0 || self.data.is_null() {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(self.data.cast::<u8>(), self.size) }
        }
    }
}

impl Default for NvimString {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorType {
    kErrorTypeNone = -1,
    kErrorTypeException = 0,
    kErrorTypeValidation = 1,
}

impl ErrorType {
    #[inline]
    pub const fn from_raw(v: c_int) -> Option<Self> {
        match v {
            -1 => Some(Self::kErrorTypeNone),
            0 => Some(Self::kErrorTypeException),
            1 => Some(Self::kErrorTypeValidation),
            _ => None,
        }
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct Error {
    pub err_type: ErrorType,
    pub msg: Option<std::ffi::CString>,
}

impl Error {
    #[inline]
    pub const fn new() -> Self {
        Self {
            err_type: ErrorType::kErrorTypeNone,
            msg: None,
        }
    }

    #[inline]
    pub fn msg_ptr(&self) -> *mut c_char {
        self.msg
            .as_ref()
            .map_or(std::ptr::null_mut(), |s| s.as_ptr().cast_mut())
    }
}

impl Default for Error {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectType {
    kObjectTypeNil = 0,
    kObjectTypeBoolean = 1,
    kObjectTypeInteger = 2,
    kObjectTypeFloat = 3,
    kObjectTypeString = 4,
    kObjectTypeArray = 5,
    kObjectTypeDict = 6,
    kObjectTypeLuaRef = 7,
    kObjectTypeUnset = 8,
    kObjectTypeBuffer = 9,
    kObjectTypeWindow = 10,
    kObjectTypeTabpage = 11,
}

impl ObjectType {
    #[inline]
    pub const fn from_raw(v: c_int) -> Option<Self> {
        match v {
            0 => Some(Self::kObjectTypeNil),
            1 => Some(Self::kObjectTypeBoolean),
            2 => Some(Self::kObjectTypeInteger),
            3 => Some(Self::kObjectTypeFloat),
            4 => Some(Self::kObjectTypeString),
            5 => Some(Self::kObjectTypeArray),
            6 => Some(Self::kObjectTypeDict),
            7 => Some(Self::kObjectTypeLuaRef),
            8 => Some(Self::kObjectTypeUnset),
            9 => Some(Self::kObjectTypeBuffer),
            10 => Some(Self::kObjectTypeWindow),
            11 => Some(Self::kObjectTypeTabpage),
            _ => None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Array {
    pub size: usize,
    pub capacity: usize,
    pub items: *mut Object,
}

impl Array {
    #[inline]
    pub const fn new() -> Self {
        Self {
            size: 0,
            capacity: 0,
            items: std::ptr::null_mut(),
        }
    }

    #[inline]
    pub fn as_slice(&self) -> &[Object] {
        if self.size == 0 || self.items.is_null() {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(self.items, self.size) }
        }
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [Object] {
        if self.size == 0 || self.items.is_null() {
            &mut []
        } else {
            unsafe { std::slice::from_raw_parts_mut(self.items, self.size) }
        }
    }

    #[inline]
    pub fn push(&mut self, value: Object) {
        if self.size == self.capacity {
            self.capacity = if self.capacity != 0 {
                self.capacity << 1
            } else {
                8
            };
            self.items = realloc_items(self.items, self.capacity);
        }
        unsafe {
            std::ptr::write(self.items.add(self.size), value);
        }
        self.size += 1;
    }
}

impl std::ops::Index<usize> for Array {
    type Output = Object;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        &self.as_slice()[index]
    }
}

impl std::ops::IndexMut<usize> for Array {
    #[inline]
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.as_mut_slice()[index]
    }
}

impl Default for Array {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Dict {
    pub size: usize,
    pub capacity: usize,
    pub items: *mut KeyValuePair,
}

impl Dict {
    #[inline]
    pub const fn new() -> Self {
        Self {
            size: 0,
            capacity: 0,
            items: std::ptr::null_mut(),
        }
    }

    #[inline]
    pub fn as_slice(&self) -> &[KeyValuePair] {
        if self.size == 0 || self.items.is_null() {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(self.items, self.size) }
        }
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [KeyValuePair] {
        if self.size == 0 || self.items.is_null() {
            &mut []
        } else {
            unsafe { std::slice::from_raw_parts_mut(self.items, self.size) }
        }
    }

    #[inline]
    pub fn push(&mut self, value: KeyValuePair) {
        if self.size == self.capacity {
            self.capacity = if self.capacity != 0 {
                self.capacity << 1
            } else {
                8
            };
            self.items = realloc_items(self.items, self.capacity);
        }
        unsafe {
            std::ptr::write(self.items.add(self.size), value);
        }
        self.size += 1;
    }
}

impl std::ops::Index<usize> for Dict {
    type Output = KeyValuePair;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        &self.as_slice()[index]
    }
}

impl std::ops::IndexMut<usize> for Dict {
    #[inline]
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.as_mut_slice()[index]
    }
}

impl Default for Dict {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub enum ObjectData {
    #[default]
    Nil,
    Boolean(bool),
    Integer(i64),
    Floating(f64),
    String(NvimString),
    Array(Array),
    Dict(Dict),
    LuaRef(c_int),
}

impl ObjectData {
    #[inline]
    pub const fn boolean(self) -> bool {
        match self {
            Self::Boolean(b) => b,
            Self::Integer(i) => i != 0,
            _ => false,
        }
    }

    #[inline]
    pub const fn integer(self) -> i64 {
        match self {
            Self::Integer(i) => i,
            Self::Boolean(b) => b as i64,
            Self::LuaRef(r) => r as i64,
            _ => 0,
        }
    }

    #[inline]
    pub fn integer_mut(&mut self) -> &mut i64 {
        match self {
            Self::Integer(i) => i,
            _ => std::process::abort(),
        }
    }

    #[inline]
    pub const fn floating(self) -> f64 {
        match self {
            Self::Floating(f) => f,
            Self::Integer(i) => i as f64,
            _ => 0.0,
        }
    }

    #[inline]
    pub const fn string(self) -> NvimString {
        match self {
            Self::String(s) => s,
            _ => NvimString::new(),
        }
    }

    #[inline]
    pub const fn array(self) -> Array {
        match self {
            Self::Array(a) => a,
            _ => Array::new(),
        }
    }

    #[inline]
    pub fn array_mut(&mut self) -> &mut Array {
        match self {
            Self::Array(a) => a,
            _ => std::process::abort(),
        }
    }

    #[inline]
    pub const fn dict(self) -> Dict {
        match self {
            Self::Dict(d) => d,
            _ => Dict::new(),
        }
    }

    #[inline]
    pub fn dict_mut(&mut self) -> &mut Dict {
        match self {
            Self::Dict(d) => d,
            _ => std::process::abort(),
        }
    }

    #[inline]
    pub const fn luaref(self) -> c_int {
        match self {
            Self::LuaRef(r) => r,
            Self::Integer(i) => i as c_int,
            _ => 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Object {
    pub obj_type: ObjectType,
    pub data: ObjectData,
}

impl Object {
    #[inline]
    pub const fn new() -> Self {
        Self {
            obj_type: ObjectType::kObjectTypeNil,
            data: ObjectData::Nil,
        }
    }
}

impl Default for Object {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct KeyValuePair {
    pub key: NvimString,
    pub value: Object,
}

impl KeyValuePair {
    #[inline]
    pub const fn new() -> Self {
        Self {
            key: NvimString::new(),
            value: Object::new(),
        }
    }
}

pub const fn object_typename(t: ObjectType) -> &'static str {
    match t {
        ObjectType::kObjectTypeUnset => "unset",
        ObjectType::kObjectTypeNil => "nil",
        ObjectType::kObjectTypeBoolean => "Boolean",
        ObjectType::kObjectTypeInteger => "Integer",
        ObjectType::kObjectTypeFloat => "Float",
        ObjectType::kObjectTypeString => "String",
        ObjectType::kObjectTypeArray => "Array",
        ObjectType::kObjectTypeDict => "Dict",
        ObjectType::kObjectTypeLuaRef => "Function",
        ObjectType::kObjectTypeBuffer => "Buffer",
        ObjectType::kObjectTypeWindow => "Window",
        ObjectType::kObjectTypeTabpage => "Tabpage",
    }
}

fn set_error_str(err: &mut Error, err_type: ErrorType, msg: &str) {
    let bytes = msg.as_bytes();
    let len = bytes.len().min((1024 * 1024) - 1);
    err.msg = Some(crate::memory::cstr_from_bytes(&bytes[..len]));
    err.err_type = err_type;
}

#[inline]
fn cstr_lossy<'a>(ptr: *const c_char) -> std::borrow::Cow<'a, str> {
    unsafe { CStr::from_ptr(ptr).to_string_lossy() }
}

/// Create `"Invalid …"` validation message on `err` (`api_err_invalid`).
///
/// # Safety
/// `err` and `name` must be valid pointers, and `val_s` must be null or a valid C string.
pub fn api_err_invalid<S1: crate::strings::IntoCCharPtr, S2: crate::strings::IntoCCharPtr>(
    err: &mut Error,
    name: S1,
    val_s: S2,
    val_n: i64,
    quote_val: bool,
) {
    let name = name.into_c_char_ptr();
    let val_s = val_s.into_c_char_ptr();
    let name_str = cstr_lossy(name);
    let has_space = name_str.as_bytes().contains(&b' ');

    let msg = if !val_s.is_null() && unsafe { *val_s == 0 } {
        if has_space {
            format!("Invalid {name_str}")
        } else {
            format!("Invalid '{name_str}'")
        }
    } else if val_s.is_null() {
        if has_space {
            format!("Invalid {name_str}: {val_n}")
        } else {
            format!("Invalid '{name_str}': {val_n}")
        }
    } else {
        let val_str = cstr_lossy(val_s);
        match (has_space, quote_val) {
            (true, true) => format!("Invalid {name_str}: '{val_str}'"),
            (true, false) => format!("Invalid {name_str}: {val_str}"),
            (false, true) => format!("Invalid '{name_str}': '{val_str}'"),
            (false, false) => format!("Invalid '{name_str}': {val_str}"),
        }
    };

    set_error_str(err, ErrorType::kErrorTypeValidation, &msg);
}

/// Create `"Invalid …: expected …"` validation message on `err` (`api_err_exp`).
///
/// # Safety
/// `err`, `name`, and `expected` must be valid pointers; `actual` must be null or a valid C string.
pub fn api_err_exp<
    S1: crate::strings::IntoCCharPtr,
    S2: crate::strings::IntoCCharPtr,
    S3: crate::strings::IntoCCharPtr,
>(
    err: &mut Error,
    name: S1,
    expected: S2,
    actual: S3,
) {
    let name = name.into_c_char_ptr();
    let expected = expected.into_c_char_ptr();
    let actual = actual.into_c_char_ptr();
    let name_str = cstr_lossy(name);
    let exp_str = cstr_lossy(expected);
    let has_space = name_str.as_bytes().contains(&b' ');

    let msg = if actual.is_null() {
        if has_space {
            format!("Invalid {name_str}: expected {exp_str}")
        } else {
            format!("Invalid '{name_str}': expected {exp_str}")
        }
    } else {
        let act_str = cstr_lossy(actual);
        if has_space {
            format!("Invalid {name_str}: expected {exp_str}, got {act_str}")
        } else {
            format!("Invalid '{name_str}': expected {exp_str}, got {act_str}")
        }
    };

    set_error_str(err, ErrorType::kErrorTypeValidation, &msg);
}

/// Create `"Required: …"` validation message on `err` (`api_err_required`).
///
/// # Safety
/// `err` and `name` must be valid pointers.
pub fn api_err_required(err: &mut Error, name: &std::ffi::CStr) {
    let name_str = cstr_lossy(name.as_ptr());
    let has_space = name_str.as_bytes().contains(&b' ');
    let msg = if has_space {
        format!("Required: {name_str}")
    } else {
        format!("Required: '{name_str}'")
    };
    set_error_str(err, ErrorType::kErrorTypeValidation, &msg);
}

/// Create `"Conflict: … not allowed with …"` validation message on `err` (`api_err_conflict`).
///
/// # Safety
/// `err`, `name`, and `name2` must be valid pointers.
pub fn api_err_conflict<S1: crate::strings::IntoCCharPtr, S2: crate::strings::IntoCCharPtr>(
    err: &mut Error,
    name: S1,
    name2: S2,
) {
    let name = name.into_c_char_ptr();
    let name2 = name2.into_c_char_ptr();
    let name_str = cstr_lossy(name);
    let name2_str = cstr_lossy(name2);
    let has_space2 = name2_str.as_bytes().contains(&b' ');
    let msg = if has_space2 {
        format!("Conflict: '{name_str}' not allowed with {name2_str}")
    } else {
        format!("Conflict: '{name_str}' not allowed with '{name2_str}'")
    };
    set_error_str(err, ErrorType::kErrorTypeValidation, &msg);
}

/// Validate that `arr` contains only `String` items (and optionally no embedded newlines)
/// (`check_string_array`).
///
/// # Safety
/// `arr`, `name`, and `err` must be valid.
pub fn check_string_array(
    arr: Array,
    name: *mut c_char,
    disallow_nl: bool,
    err: &mut Error,
) -> bool {
    let name_str = cstr_lossy(name);
    for i in 0..arr.size {
        let item = unsafe { &*arr.items.add(i) };
        if item.obj_type != ObjectType::kObjectTypeString {
            let msg = format!(
                "Invalid '{name_str}' item: expected String, got {}",
                object_typename(item.obj_type)
            );
            set_error_str(err, ErrorType::kErrorTypeValidation, &msg);
            return false;
        }
        if disallow_nl {
            let l = item.data.string();
            if l.as_bytes().contains(&b'\n') {
                let msg = format!("'{name_str}' item contains newlines");
                set_error_str(err, ErrorType::kErrorTypeValidation, &msg);
                return false;
            }
        }
    }
    true
}

#[inline]
pub fn api_err_range(err: &mut Error, name: &std::ffi::CStr) {
    api_err_invalid(err, name.as_ptr(), c"out of range", 0, false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::private::helpers::api_clear_error;

    #[test]
    fn formats_validation_error_messages() {
        let mut err = Error::new();

        api_err_invalid(&mut err, c"col", std::ptr::null(), -1, false);
            assert_eq!(err.err_type, ErrorType::kErrorTypeValidation);
            assert_eq!(err.msg.as_ref().unwrap().to_str().unwrap(), "Invalid 'col': -1");
            api_clear_error(&mut err);

            api_err_exp(
                &mut err,
                c"buffer id",
                c"Integer",
                c"String",
            );
            assert_eq!(
                err.msg.as_ref().unwrap().to_str().unwrap(),
                "Invalid buffer id: expected Integer, got String"
            );
            api_clear_error(&mut err);

            api_err_required(&mut err, c"width");
            assert_eq!(
                err.msg.as_ref().unwrap().to_str().unwrap(),
                "Required: 'width'"
            );
            api_clear_error(&mut err);

            api_err_conflict(&mut err, c"row", c"win");
            assert_eq!(
                err.msg.as_ref().unwrap().to_str().unwrap(),
                "Conflict: 'row' not allowed with 'win'"
            );
            api_clear_error(&mut err);
    }
}
