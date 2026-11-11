#![forbid(unsafe_code)]

//! API argument validation helpers (`src/nvim/api/private/validate.c`).

use crate::memory::realloc_items;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NvimString {
    pub data: crate::globals::GraphPtr<u8>,
    pub size: usize,
}

impl NvimString {
    #[inline]
    pub const fn new() -> Self {
        Self {
            data: crate::globals::GraphPtr::NULL,
            size: 0,
        }
    }

    #[inline]
    pub const fn empty_cstr() -> Self {
        Self {
            data: crate::globals::GraphPtr::new(c"".as_ptr().cast_mut().cast()),
            size: 0,
        }
    }

    #[inline]
    pub const fn from_cstr(s: &std::ffi::CStr) -> Self {
        Self {
            data: crate::globals::GraphPtr::new(s.as_ptr().cast_mut().cast()),
            size: s.to_bytes().len(),
        }
    }

    #[inline]
    pub fn from_opt_cstr(s: Option<&std::ffi::CStr>) -> Self {
        match s {
            Some(s) => Self::from_cstr(s),
            None => Self::new(),
        }
    }

    #[inline]
    pub const fn from_bytes_len(bytes: &[u8], size: usize) -> Self {
        let ptr = if bytes.is_empty() {
            c"".as_ptr().cast::<u8>()
        } else {
            bytes.as_ptr()
        };
        Self {
            data: crate::globals::GraphPtr::new(ptr.cast_mut()),
            size,
        }
    }

    #[inline]
    pub const fn from_bytes(bytes: &[u8]) -> Self {
        Self::from_bytes_len(bytes, bytes.len())
    }

    #[inline]
    pub fn as_bytes(self) -> &'static [u8] {
        if self.is_empty() {
            c"".to_bytes()
        } else {
            self.data.ptr_slice(self.size)
        }
    }

    #[inline]
    pub fn slice(self, len: usize) -> &'static [u8] {
        let b = self.as_bytes();
        &b[..len.min(b.len())]
    }

    #[inline]
    pub fn as_bytes_with_nul_mut(&mut self) -> &mut [u8] {
        if self.data.is_null() {
            &mut []
        } else {
            self.data.ptr_slice_mut(self.size + 1)
        }
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.size == 0 || self.data.is_null()
    }

    #[inline]
    pub fn cstr_bytes(self) -> &'static [u8] {
        if self.data.is_null() {
            c"".to_bytes()
        } else if self.size == 0 {
            self.as_cstr().to_bytes()
        } else {
            crate::memory::bytes_until_nul(self.as_bytes())
        }
    }

    #[inline]
    pub fn opt_bytes(self) -> Option<&'static [u8]> {
        if self.data.is_null() {
            None
        } else {
            Some(self.cstr_bytes())
        }
    }

    #[inline]
    pub fn first_byte(self) -> u8 {
        self.cstr_bytes().first().copied().unwrap_or(0)
    }

    #[inline]
    pub fn as_cstr(self) -> &'static std::ffi::CStr {
        self.as_opt_cstr().unwrap_or(c"")
    }

    #[inline]
    pub fn as_opt_cstr(self) -> Option<&'static std::ffi::CStr> {
        if self.data.is_null() {
            return None;
        }
        let mut len = self.size;
        if *self.data.wrapping_add(len).borrow() != 0 {
            len = 0;
            while *self.data.wrapping_add(len).borrow() != 0 {
                len += 1;
            }
        }
        let slice = self.data.ptr_slice(len + 1);
        Some(crate::memory::bytes_as_cstr(slice))
    }

    #[inline]
    pub fn to_cstring(self) -> std::ffi::CString {
        self.as_cstr().to_owned()
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
    ErrorTypeNone = -1,
    ErrorTypeException = 0,
    ErrorTypeValidation = 1,
}

impl ErrorType {
    #[inline]
    pub const fn from_raw(v: i32) -> Option<Self> {
        match v {
            -1 => Some(Self::ErrorTypeNone),
            0 => Some(Self::ErrorTypeException),
            1 => Some(Self::ErrorTypeValidation),
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
            err_type: ErrorType::ErrorTypeNone,
            msg: None,
        }
    }

    #[inline]
    pub fn msg_cstr(&self) -> Option<&std::ffi::CStr> {
        self.msg.as_deref()
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
    ObjectTypeNil = 0,
    ObjectTypeBoolean = 1,
    ObjectTypeInteger = 2,
    ObjectTypeFloat = 3,
    ObjectTypeString = 4,
    ObjectTypeArray = 5,
    ObjectTypeDict = 6,
    ObjectTypeLuaRef = 7,
    ObjectTypeUnset = 8,
    ObjectTypeBuffer = 9,
    ObjectTypeWindow = 10,
    ObjectTypeTabpage = 11,
}

impl ObjectType {
    #[inline]
    pub const fn from_raw(v: i32) -> Option<Self> {
        match v {
            0 => Some(Self::ObjectTypeNil),
            1 => Some(Self::ObjectTypeBoolean),
            2 => Some(Self::ObjectTypeInteger),
            3 => Some(Self::ObjectTypeFloat),
            4 => Some(Self::ObjectTypeString),
            5 => Some(Self::ObjectTypeArray),
            6 => Some(Self::ObjectTypeDict),
            7 => Some(Self::ObjectTypeLuaRef),
            8 => Some(Self::ObjectTypeUnset),
            9 => Some(Self::ObjectTypeBuffer),
            10 => Some(Self::ObjectTypeWindow),
            11 => Some(Self::ObjectTypeTabpage),
            _ => None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Array {
    pub size: usize,
    pub capacity: usize,
    pub items: crate::globals::GraphPtr<Object>,
}

impl Array {
    #[inline(always)]
    pub const fn new() -> Self {
        Self {
            size: 0,
            capacity: 0,
            items: crate::globals::GraphPtr::NULL,
        }
    }

    #[inline(always)]
    pub fn from_slice(slice: &mut [Object]) -> Self {
        let len = slice.len();
        Self {
            size: len,
            capacity: len,
            items: if len == 0 {
                crate::globals::GraphPtr::NULL
            } else {
                crate::globals::GraphPtr::new(slice.as_mut_ptr())
            },
        }
    }

    #[inline(always)]
    pub fn from_buf(buf: &mut [Object]) -> Self {
        let cap = buf.len();
        Self {
            size: 0,
            capacity: cap,
            items: if cap == 0 {
                crate::globals::GraphPtr::NULL
            } else {
                crate::globals::GraphPtr::new(buf.as_mut_ptr())
            },
        }
    }

    #[inline(always)]
    pub fn as_slice(&self) -> &[Object] {
        if self.size == 0 || self.items.is_null() {
            &[]
        } else {
            self.items.ptr_slice(self.size)
        }
    }

    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [Object] {
        if self.size == 0 || self.items.is_null() {
            &mut []
        } else {
            self.items.ptr_slice_mut(self.size)
        }
    }

    #[inline(always)]
    pub fn next_slot_mut(&mut self) -> &'static mut Object {
        debug_assert!(!self.items.is_null());
        self.items.wrapping_add(self.size).borrow_mut()
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

        *self.next_slot_mut() = value;

        self.size += 1;
    }
}

impl std::ops::Index<usize> for Array {
    type Output = Object;

    #[inline(always)]
    fn index(&self, index: usize) -> &Self::Output {
        debug_assert!(!self.items.is_null());
        self.items.wrapping_add(index).borrow()
    }
}

impl std::ops::IndexMut<usize> for Array {
    #[inline(always)]
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        debug_assert!(!self.items.is_null());
        self.items.wrapping_add(index).borrow_mut()
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
    pub items: crate::globals::GraphPtr<KeyValuePair>,
}

impl Dict {
    #[inline(always)]
    pub const fn new() -> Self {
        Self {
            size: 0,
            capacity: 0,
            items: crate::globals::GraphPtr::NULL,
        }
    }

    #[inline(always)]
    pub fn from_slice(slice: &mut [KeyValuePair]) -> Self {
        let len = slice.len();
        Self {
            size: len,
            capacity: len,
            items: if len == 0 {
                crate::globals::GraphPtr::NULL
            } else {
                crate::globals::GraphPtr::new(slice.as_mut_ptr())
            },
        }
    }

    #[inline(always)]
    pub fn from_buf(buf: &mut [KeyValuePair]) -> Self {
        let cap = buf.len();
        Self {
            size: 0,
            capacity: cap,
            items: if cap == 0 {
                crate::globals::GraphPtr::NULL
            } else {
                crate::globals::GraphPtr::new(buf.as_mut_ptr())
            },
        }
    }

    #[inline(always)]
    pub fn as_slice(&self) -> &[KeyValuePair] {
        if self.size == 0 || self.items.is_null() {
            &[]
        } else {
            self.items.ptr_slice(self.size)
        }
    }

    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [KeyValuePair] {
        if self.size == 0 || self.items.is_null() {
            &mut []
        } else {
            self.items.ptr_slice_mut(self.size)
        }
    }

    #[inline(always)]
    pub fn next_slot_mut(&mut self) -> &'static mut KeyValuePair {
        debug_assert!(!self.items.is_null());
        self.items.wrapping_add(self.size).borrow_mut()
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

        *self.next_slot_mut() = value;

        self.size += 1;
    }
}

impl std::ops::Index<usize> for Dict {
    type Output = KeyValuePair;

    #[inline(always)]
    fn index(&self, index: usize) -> &Self::Output {
        debug_assert!(!self.items.is_null());
        self.items.wrapping_add(index).borrow()
    }
}

impl std::ops::IndexMut<usize> for Dict {
    #[inline(always)]
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        debug_assert!(!self.items.is_null());
        self.items.wrapping_add(index).borrow_mut()
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
    LuaRef(i32),
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
    pub const fn luaref(self) -> i32 {
        match self {
            Self::LuaRef(r) => r,
            Self::Integer(i) => i as i32,
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
            obj_type: ObjectType::ObjectTypeNil,
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
        ObjectType::ObjectTypeUnset => "unset",
        ObjectType::ObjectTypeNil => "nil",
        ObjectType::ObjectTypeBoolean => "Boolean",
        ObjectType::ObjectTypeInteger => "Integer",
        ObjectType::ObjectTypeFloat => "Float",
        ObjectType::ObjectTypeString => "String",
        ObjectType::ObjectTypeArray => "Array",
        ObjectType::ObjectTypeDict => "Dict",
        ObjectType::ObjectTypeLuaRef => "Function",
        ObjectType::ObjectTypeBuffer => "Buffer",
        ObjectType::ObjectTypeWindow => "Window",
        ObjectType::ObjectTypeTabpage => "Tabpage",
    }
}

fn set_error_str(err: &mut Error, err_type: ErrorType, msg: &str) {
    let bytes = msg.as_bytes();
    let len = bytes.len().min((1024 * 1024) - 1);
    err.msg = Some(crate::memory::bytes_to_cstring(&bytes[..len]));
    err.err_type = err_type;
}

#[inline]
/// Create `"Invalid …"` validation message on `err` (`api_err_invalid`).
pub fn api_err_invalid(
    err: &mut Error,
    name: &std::ffi::CStr,
    val_s: Option<&std::ffi::CStr>,
    val_n: i64,
    quote_val: bool,
) {
    let name_str = name.to_string_lossy();
    let has_space = name_str.as_bytes().contains(&b' ');

    let msg = match val_s {
        Some(vs) if vs.to_bytes().is_empty() => {
            if has_space {
                format!("Invalid {name_str}")
            } else {
                format!("Invalid '{name_str}'")
            }
        }
        None => {
            if has_space {
                format!("Invalid {name_str}: {val_n}")
            } else {
                format!("Invalid '{name_str}': {val_n}")
            }
        }
        Some(vs) => {
            let val_str = vs.to_string_lossy();
            match (has_space, quote_val) {
                (true, true) => format!("Invalid {name_str}: '{val_str}'"),
                (true, false) => format!("Invalid {name_str}: {val_str}"),
                (false, true) => format!("Invalid '{name_str}': '{val_str}'"),
                (false, false) => format!("Invalid '{name_str}': {val_str}"),
            }
        }
    };

    set_error_str(err, ErrorType::ErrorTypeValidation, &msg);
}

/// Create `"Invalid …: expected …"` validation message on `err` (`api_err_exp`).
pub fn api_err_exp(
    err: &mut Error,
    name: &std::ffi::CStr,
    expected: &std::ffi::CStr,
    actual: Option<&std::ffi::CStr>,
) {
    let name_str = name.to_string_lossy();
    let exp_str = expected.to_string_lossy();
    let has_space = name_str.as_bytes().contains(&b' ');

    let msg = match actual {
        None => {
            if has_space {
                format!("Invalid {name_str}: expected {exp_str}")
            } else {
                format!("Invalid '{name_str}': expected {exp_str}")
            }
        }
        Some(actual) => {
            let act_str = actual.to_string_lossy();
            if has_space {
                format!("Invalid {name_str}: expected {exp_str}, got {act_str}")
            } else {
                format!("Invalid '{name_str}': expected {exp_str}, got {act_str}")
            }
        }
    };

    set_error_str(err, ErrorType::ErrorTypeValidation, &msg);
}

/// Create `"Required: …"` validation message on `err` (`api_err_required`).
pub fn api_err_required(err: &mut Error, name: &std::ffi::CStr) {
    let name_str = name.to_string_lossy();
    let has_space = name_str.as_bytes().contains(&b' ');
    let msg = if has_space {
        format!("Required: {name_str}")
    } else {
        format!("Required: '{name_str}'")
    };
    set_error_str(err, ErrorType::ErrorTypeValidation, &msg);
}

/// Create `"Conflict: … not allowed with …"` validation message on `err` (`api_err_conflict`).
pub fn api_err_conflict(err: &mut Error, name: &std::ffi::CStr, name2: &std::ffi::CStr) {
    let name_str = name.to_string_lossy();
    let name2_str = name2.to_string_lossy();
    let has_space2 = name2_str.as_bytes().contains(&b' ');
    let msg = if has_space2 {
        format!("Conflict: '{name_str}' not allowed with {name2_str}")
    } else {
        format!("Conflict: '{name_str}' not allowed with '{name2_str}'")
    };
    set_error_str(err, ErrorType::ErrorTypeValidation, &msg);
}

/// Validate that `arr` contains only `String` items (and optionally no embedded newlines)
/// (`check_string_array`).
pub fn check_string_array(
    arr: Array,
    name: &std::ffi::CStr,
    disallow_nl: bool,
    err: &mut Error,
) -> bool {
    let name_str = name.to_string_lossy();
    for i in 0..arr.size {
        let item = &arr[i];
        if item.obj_type != ObjectType::ObjectTypeString {
            let msg = format!(
                "Invalid '{name_str}' item: expected String, got {}",
                object_typename(item.obj_type)
            );
            set_error_str(err, ErrorType::ErrorTypeValidation, &msg);
            return false;
        }
        if disallow_nl {
            let l = item.data.string();
            if l.as_bytes().contains(&b'\n') {
                let msg = format!("'{name_str}' item contains newlines");
                set_error_str(err, ErrorType::ErrorTypeValidation, &msg);
                return false;
            }
        }
    }
    true
}

#[inline]
pub fn api_err_range(err: &mut Error, name: &std::ffi::CStr) {
    api_err_invalid(err, name, Some(c"out of range"), 0, false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::private::helpers::api_clear_error;

    #[test]
    fn formats_validation_error_messages() {
        let mut err = Error::new();

        api_err_invalid(&mut err, c"col", None, -1, false);
        assert_eq!(err.err_type, ErrorType::ErrorTypeValidation);
        assert_eq!(
            err.msg.as_ref().unwrap().to_str().unwrap(),
            "Invalid 'col': -1"
        );
        api_clear_error(&mut err);

        api_err_exp(&mut err, c"buffer id", c"Integer", Some(c"String"));
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
