//! API argument validation helpers (`src/nvim/api/private/validate.c`).

use std::ffi::{CStr, c_char, c_int};

use crate::kvec::KVec;
use crate::memory::xmallocz;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NvimString {
    pub data: *mut c_char,
    pub size: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum ErrorType {
    kErrorTypeNone = -1,
    kErrorTypeException = 0,
    kErrorTypeValidation = 1,
}

#[repr(C)]
#[derive(Debug)]
pub struct Error {
    pub err_type: ErrorType,
    pub msg: *mut c_char,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types)]
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

pub type Array = KVec<Object>;
pub type Dict = KVec<KeyValuePair>;

#[repr(C)]
#[derive(Clone, Copy)]
pub union ObjectData {
    pub boolean: bool,
    pub integer: i64,
    pub floating: f64,
    pub string: NvimString,
    pub array: Array,
    pub dict: Dict,
    pub luaref: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Object {
    pub obj_type: ObjectType,
    pub data: ObjectData,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyValuePair {
    pub key: NvimString,
    pub value: Object,
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

unsafe fn set_error_str(err: *mut Error, err_type: ErrorType, msg: &str) {
    let bytes = msg.as_bytes();
    let len = bytes.len().min((1024 * 1024) - 1);
    let buf = unsafe { xmallocz(len).cast::<u8>() };
    unsafe {
        if len > 0 {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, len);
        }
        *buf.add(len) = 0;
        (*err).msg = buf.cast::<c_char>();
        (*err).err_type = err_type;
    }
}

#[inline]
unsafe fn cstr_lossy<'a>(ptr: *const c_char) -> std::borrow::Cow<'a, str> {
    unsafe { CStr::from_ptr(ptr).to_string_lossy() }
}

/// Create `"Invalid …"` validation message on `err` (`api_err_invalid`).
///
/// # Safety
/// `err` and `name` must be valid pointers, and `val_s` must be null or a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn api_err_invalid(
    err: *mut Error,
    name: *const c_char,
    val_s: *const c_char,
    val_n: i64,
    quote_val: bool,
) {
    let name_str = unsafe { cstr_lossy(name) };
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
        let val_str = unsafe { cstr_lossy(val_s) };
        match (has_space, quote_val) {
            (true, true) => format!("Invalid {name_str}: '{val_str}'"),
            (true, false) => format!("Invalid {name_str}: {val_str}"),
            (false, true) => format!("Invalid '{name_str}': '{val_str}'"),
            (false, false) => format!("Invalid '{name_str}': {val_str}"),
        }
    };

    unsafe {
        set_error_str(err, ErrorType::kErrorTypeValidation, &msg);
    }
}

/// Create `"Invalid …: expected …"` validation message on `err` (`api_err_exp`).
///
/// # Safety
/// `err`, `name`, and `expected` must be valid pointers; `actual` must be null or a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn api_err_exp(
    err: *mut Error,
    name: *const c_char,
    expected: *const c_char,
    actual: *const c_char,
) {
    let name_str = unsafe { cstr_lossy(name) };
    let exp_str = unsafe { cstr_lossy(expected) };
    let has_space = name_str.as_bytes().contains(&b' ');

    let msg = if actual.is_null() {
        if has_space {
            format!("Invalid {name_str}: expected {exp_str}")
        } else {
            format!("Invalid '{name_str}': expected {exp_str}")
        }
    } else {
        let act_str = unsafe { cstr_lossy(actual) };
        if has_space {
            format!("Invalid {name_str}: expected {exp_str}, got {act_str}")
        } else {
            format!("Invalid '{name_str}': expected {exp_str}, got {act_str}")
        }
    };

    unsafe {
        set_error_str(err, ErrorType::kErrorTypeValidation, &msg);
    }
}

/// Create `"Required: …"` validation message on `err` (`api_err_required`).
///
/// # Safety
/// `err` and `name` must be valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn api_err_required(err: *mut Error, name: *const c_char) {
    let name_str = unsafe { cstr_lossy(name) };
    let has_space = name_str.as_bytes().contains(&b' ');
    let msg = if has_space {
        format!("Required: {name_str}")
    } else {
        format!("Required: '{name_str}'")
    };
    unsafe {
        set_error_str(err, ErrorType::kErrorTypeValidation, &msg);
    }
}

/// Create `"Conflict: … not allowed with …"` validation message on `err` (`api_err_conflict`).
///
/// # Safety
/// `err`, `name`, and `name2` must be valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn api_err_conflict(
    err: *mut Error,
    name: *const c_char,
    name2: *const c_char,
) {
    let name_str = unsafe { cstr_lossy(name) };
    let name2_str = unsafe { cstr_lossy(name2) };
    let has_space2 = name2_str.as_bytes().contains(&b' ');
    let msg = if has_space2 {
        format!("Conflict: '{name_str}' not allowed with {name2_str}")
    } else {
        format!("Conflict: '{name_str}' not allowed with '{name2_str}'")
    };
    unsafe {
        set_error_str(err, ErrorType::kErrorTypeValidation, &msg);
    }
}

/// Validate that `arr` contains only `String` items (and optionally no embedded newlines)
/// (`check_string_array`).
///
/// # Safety
/// `arr`, `name`, and `err` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn check_string_array(
    arr: Array,
    name: *mut c_char,
    disallow_nl: bool,
    err: *mut Error,
) -> bool {
    let name_str = unsafe { cstr_lossy(name) };
    for i in 0..arr.size {
        let item = unsafe { &*arr.items.add(i) };
        if item.obj_type != ObjectType::kObjectTypeString {
            let msg = format!(
                "Invalid '{name_str}' item: expected String, got {}",
                object_typename(item.obj_type)
            );
            unsafe {
                set_error_str(err, ErrorType::kErrorTypeValidation, &msg);
            }
            return false;
        }
        if disallow_nl {
            let l = unsafe { item.data.string };
            if l.size > 0 && !l.data.is_null() {
                let bytes = unsafe { std::slice::from_raw_parts(l.data.cast::<u8>(), l.size) };
                if bytes.contains(&b'\n') {
                    let msg = format!("'{name_str}' item contains newlines");
                    unsafe {
                        set_error_str(err, ErrorType::kErrorTypeValidation, &msg);
                    }
                    return false;
                }
            }
        }
    }
    true
}

#[inline]
#[allow(clippy::missing_safety_doc)]
pub unsafe fn api_err_range(err: *mut Error, name: *const c_char) {
    unsafe {
        api_err_invalid(err, name, c"out of range".as_ptr(), 0, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::xfree;
    use std::ffi::c_void;

    #[test]
    fn formats_validation_error_messages() {
        let mut err = Error {
            err_type: ErrorType::kErrorTypeNone,
            msg: std::ptr::null_mut(),
        };

        unsafe {
            api_err_invalid(&mut err, c"col".as_ptr(), std::ptr::null(), -1, false);
            assert_eq!(err.err_type, ErrorType::kErrorTypeValidation);
            assert_eq!(
                CStr::from_ptr(err.msg).to_str().unwrap(),
                "Invalid 'col': -1"
            );
            xfree(err.msg.cast::<c_void>());

            api_err_exp(
                &mut err,
                c"buffer id".as_ptr(),
                c"Integer".as_ptr(),
                c"String".as_ptr(),
            );
            assert_eq!(
                CStr::from_ptr(err.msg).to_str().unwrap(),
                "Invalid buffer id: expected Integer, got String"
            );
            xfree(err.msg.cast::<c_void>());

            api_err_required(&mut err, c"width".as_ptr());
            assert_eq!(
                CStr::from_ptr(err.msg).to_str().unwrap(),
                "Required: 'width'"
            );
            xfree(err.msg.cast::<c_void>());

            api_err_conflict(&mut err, c"row".as_ptr(), c"win".as_ptr());
            assert_eq!(
                CStr::from_ptr(err.msg).to_str().unwrap(),
                "Conflict: 'row' not allowed with 'win'"
            );
            xfree(err.msg.cast::<c_void>());
        }
    }
}
