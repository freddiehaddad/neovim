//! Lua CJSON module (`src/cjson/`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clashing_extern_declarations,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

pub mod fpconv;
pub mod lua_cjson;
pub mod strbuf;

#[cfg(test)]
mod tests {
    use std::ffi::c_char;
    use std::ptr;

    use crate::cjson::fpconv::fpconv_strtod;
    use crate::cjson::strbuf::{
        strbuf_append_char, strbuf_append_string, strbuf_free, strbuf_init, strbuf_length,
        strbuf_string, strbuf_t,
    };

    #[test]
    fn test_strbuf_and_fpconv_strtod() {
        unsafe {
            let mut sb: strbuf_t = std::mem::zeroed();
            strbuf_init(&mut sb, 0);
            strbuf_append_string(&mut sb, c"hello".as_ptr());
            strbuf_append_char(&mut sb, b'!' as c_char);
            assert_eq!(strbuf_length(&mut sb), 6);
            let mut len = 0usize;
            let ptr = strbuf_string(&mut sb, &mut len);
            let bytes = std::slice::from_raw_parts(ptr.cast::<u8>(), len);
            assert_eq!(bytes, b"hello!");
            strbuf_free(&mut sb);

            let mut endptr: *mut c_char = ptr::null_mut();
            let v = fpconv_strtod(c"3.25".as_ptr(), &mut endptr);
            assert!((v - 3.25).abs() < f64::EPSILON);
        }
    }
}
