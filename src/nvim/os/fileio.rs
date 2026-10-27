//! Buffered file descriptor reading and writing (`src/nvim/os/fileio.c`).

#[cfg(unix)]
use std::ffi::c_void;
use std::ffi::{c_char, c_int};

use crate::fileio::O_RDONLY;
use crate::memory::ARENA_BLOCK_SIZE;
use crate::os::fs::{
    O_APPEND, O_CREAT, O_EXCL, O_NOFOLLOW, O_TRUNC, O_WRONLY, UV_EINVAL, UV_EIO, UV_ENOTSUP,
    UV_EROFS, os_close, os_file_mkdir, os_fsync, os_open, os_open_stdin_fd, os_read, os_write,
};
#[cfg(unix)]
use crate::os::fs::{iovec, os_readv};

pub const K_FILE_READ_ONLY: c_int = 1;
pub const K_FILE_CREATE: c_int = 2;
pub const K_FILE_WRITE_ONLY: c_int = 4;
pub const K_FILE_NO_SYMLINK: c_int = 8;
pub const K_FILE_CREATE_ONLY: c_int = 16;
pub const K_FILE_TRUNCATE: c_int = 32;
pub const K_FILE_APPEND: c_int = 64;
pub const K_FILE_NON_BLOCKING: c_int = 128;
pub const K_FILE_MK_DIR: c_int = 256;

#[repr(C)]
#[derive(Debug)]
pub struct FileDescriptor {
    pub fd: c_int,
    pub buffer: Vec<c_char>,
    pub read_pos: usize,
    pub write_pos: usize,
    pub wr: bool,
    pub eof: bool,
    pub non_blocking: bool,
    pub bytes_read: u64,
}

impl FileDescriptor {
    pub const fn new() -> Self {
        Self {
            fd: 0,
            buffer: Vec::new(),
            read_pos: 0,
            write_pos: 0,
            wr: false,
            eof: false,
            non_blocking: false,
            bytes_read: 0,
        }
    }
}

impl Default for FileDescriptor {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
pub fn file_space(fp: &FileDescriptor) -> usize {
    fp.buffer.len().saturating_sub(fp.write_pos)
}

/// Open a file and initialize `ret_fp` (`file_open`).
///
/// # Safety
/// `ret_fp` and `fname` must be valid pointers.
pub unsafe fn file_open(
    ret_fp: *mut FileDescriptor,
    fname: *const c_char,
    flags: c_int,
    mode: c_int,
) -> c_int {
    let mut os_open_flags: c_int = 0;
    let mut wr: Option<bool> = None;

    if (flags & K_FILE_WRITE_ONLY) != 0 {
        os_open_flags |= O_WRONLY;
        wr = Some(true);
    }
    if (flags & K_FILE_CREATE_ONLY) != 0 {
        os_open_flags |= O_CREAT | O_EXCL | O_WRONLY;
        wr = Some(true);
    }
    if (flags & K_FILE_CREATE) != 0 {
        assert_eq!(flags & K_FILE_CREATE_ONLY, 0);
        os_open_flags |= O_CREAT | O_WRONLY;
        wr = Some(true);
    }
    if (flags & K_FILE_TRUNCATE) != 0 {
        assert_eq!(flags & K_FILE_CREATE_ONLY, 0);
        os_open_flags |= O_TRUNC | O_WRONLY;
        wr = Some(true);
    }
    if (flags & K_FILE_APPEND) != 0 {
        assert_eq!(flags & K_FILE_CREATE_ONLY, 0);
        os_open_flags |= O_APPEND | O_WRONLY;
        wr = Some(true);
    }
    if (flags & K_FILE_READ_ONLY) != 0 {
        assert_ne!(wr, Some(true));
        os_open_flags |= O_RDONLY;
    }
    if O_NOFOLLOW != 0 {
        if (flags & K_FILE_NO_SYMLINK) != 0 {
            os_open_flags |= O_NOFOLLOW;
        }
        if (flags & K_FILE_MK_DIR) != 0 {
            assert_eq!(flags & K_FILE_CREATE_ONLY, 0);
            os_open_flags |= O_CREAT | O_WRONLY;
        }
    }

    if (flags & K_FILE_MK_DIR) != 0 {
        let mkdir_ret = unsafe { os_file_mkdir(fname.cast_mut(), 0o755) };
        if mkdir_ret < 0 {
            return mkdir_ret;
        }
    }

    let fd = unsafe { os_open(fname, os_open_flags, mode) };
    if fd < 0 {
        return fd;
    }
    unsafe { file_open_fd(ret_fp, fd, flags) }
}

/// Wrap `fd` in a `FileDescriptor` (`file_open_fd`).
///
/// # Safety
/// `ret_fp` must be a valid writable pointer.
pub unsafe fn file_open_fd(ret_fp: *mut FileDescriptor, fd: c_int, flags: c_int) -> c_int {
    let fp = unsafe { &mut *ret_fp };
    fp.wr = (flags
        & (K_FILE_CREATE
            | K_FILE_CREATE_ONLY
            | K_FILE_TRUNCATE
            | K_FILE_APPEND
            | K_FILE_WRITE_ONLY))
        != 0;
    fp.non_blocking = (flags & K_FILE_NON_BLOCKING) != 0;
    assert!(!fp.wr || !fp.non_blocking);
    fp.fd = fd;
    fp.eof = false;
    fp.buffer = vec![0; ARENA_BLOCK_SIZE];
    fp.read_pos = 0;
    fp.write_pos = 0;
    fp.bytes_read = 0;
    0
}

/// Open standard input as a `FileDescriptor` (`file_open_stdin`).
///
/// # Safety
/// `fp` must be a valid writable pointer.
pub unsafe fn file_open_stdin(fp: *mut FileDescriptor) -> c_int {
    let fd = unsafe { os_open_stdin_fd() };
    unsafe { file_open_fd(fp, fd, K_FILE_READ_ONLY | K_FILE_NON_BLOCKING) }
}

/// Open an in-memory buffer `data[..len]` for reading (`file_open_buffer`).
///
/// # Safety
/// `ret_fp` must be a valid writable pointer, and `data` must be valid for `len` bytes.
pub unsafe fn file_open_buffer(ret_fp: *mut FileDescriptor, data: *mut c_char, len: usize) {
    let fp = unsafe { &mut *ret_fp };
    fp.wr = false;
    fp.non_blocking = false;
    fp.fd = -1;
    fp.eof = true;
    fp.buffer = if data.is_null() || len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(data, len).to_vec() }
    };
    fp.read_pos = 0;
    fp.write_pos = fp.buffer.len();
    fp.bytes_read = 0;
}

/// Close `fp` and free its buffer (`file_close`).
///
/// # Safety
/// `fp` must be a valid writable pointer to an initialized `FileDescriptor`.
pub unsafe fn file_close(fp: *mut FileDescriptor, do_fsync: bool) -> c_int {
    let fp_ref = unsafe { &mut *fp };
    if fp_ref.fd < 0 {
        return 0;
    }

    let flush_error = if do_fsync {
        unsafe { file_fsync(fp) }
    } else {
        unsafe { file_flush(fp) }
    };
    let close_error = unsafe { os_close(fp_ref.fd) };
    fp_ref.buffer = Vec::new();
    fp_ref.read_pos = 0;
    fp_ref.write_pos = 0;
    if close_error != 0 {
        return close_error;
    }
    flush_error
}

/// Flush `fp` and run `fsync()` (`file_fsync`).
///
/// # Safety
/// `fp` must be a valid writable pointer to an initialized `FileDescriptor`.
pub unsafe fn file_fsync(fp: *mut FileDescriptor) -> c_int {
    let fp_ref = unsafe { &mut *fp };
    if !fp_ref.wr {
        return 0;
    }
    let flush_error = unsafe { file_flush(fp) };
    if flush_error != 0 {
        return flush_error;
    }
    let fsync_error = unsafe { os_fsync(fp_ref.fd) };
    if fsync_error != UV_EINVAL && fsync_error != UV_EROFS && fsync_error != UV_ENOTSUP {
        return fsync_error;
    }
    0
}

/// Flush buffered writes in `fp` to disk (`file_flush`).
///
/// # Safety
/// `fp` must be a valid writable pointer to an initialized `FileDescriptor`.
pub unsafe fn file_flush(fp: *mut FileDescriptor) -> c_int {
    let fp = unsafe { &mut *fp };
    if !fp.wr {
        return 0;
    }

    let to_write = fp.write_pos.saturating_sub(fp.read_pos);
    if to_write == 0 {
        return 0;
    }
    let read_ptr = unsafe { fp.buffer.as_ptr().add(fp.read_pos) };
    let wres = unsafe { os_write(fp.fd, read_ptr, to_write, fp.non_blocking) };
    fp.read_pos = 0;
    fp.write_pos = 0;
    if wres != to_write as isize {
        return if wres >= 0 { UV_EIO } else { wres as c_int };
    }
    0
}

/// Read up to `size` bytes from `fp` into `ret_buf` (`file_read`).
///
/// # Safety
/// `fp` must be a valid readable `FileDescriptor`, and `ret_buf` must be writable for `size` bytes.
pub unsafe fn file_read(fp: *mut FileDescriptor, ret_buf: *mut c_char, size: usize) -> isize {
    let fp = unsafe { &mut *fp };
    assert!(!fp.wr);
    let buffered = fp.write_pos.saturating_sub(fp.read_pos);
    let from_buffer = buffered.min(size);
    if from_buffer > 0 {
        unsafe {
            std::ptr::copy_nonoverlapping(
                fp.buffer.as_ptr().add(fp.read_pos).cast::<u8>(),
                ret_buf.cast::<u8>(),
                from_buffer,
            );
        }
    }

    #[cfg(unix)]
    let mut buf = unsafe { ret_buf.add(from_buffer) };
    #[cfg(not(unix))]
    let buf = unsafe { ret_buf.add(from_buffer) };
    let mut read_remaining = size - from_buffer;
    if read_remaining == 0 {
        fp.bytes_read += from_buffer as u64;
        fp.read_pos += from_buffer;
        return from_buffer as isize;
    }

    fp.read_pos = 0;
    fp.write_pos = 0;

    #[cfg(unix)]
    {
        let mut called_read = false;
        while read_remaining > 0 {
            if fp.eof || (called_read && fp.non_blocking) {
                break;
            }
            let write_ptr = unsafe { fp.buffer.as_mut_ptr().add(fp.write_pos) };
            let mut iov = [
                iovec {
                    iov_base: buf.cast::<c_void>(),
                    iov_len: read_remaining,
                },
                iovec {
                    iov_base: write_ptr.cast::<c_void>(),
                    iov_len: ARENA_BLOCK_SIZE,
                },
            ];
            let r_ret = unsafe {
                os_readv(
                    fp.fd,
                    &mut fp.eof,
                    iov.as_mut_ptr(),
                    iov.len(),
                    fp.non_blocking,
                )
            };
            if r_ret > 0 {
                let r_usize = r_ret as usize;
                if r_usize > read_remaining {
                    fp.write_pos += r_usize - read_remaining;
                    read_remaining = 0;
                } else {
                    buf = unsafe { buf.add(r_usize) };
                    read_remaining -= r_usize;
                }
            } else if r_ret < 0 {
                return r_ret;
            }
            called_read = true;
        }
    }

    #[cfg(not(unix))]
    {
        if !fp.eof {
            if read_remaining >= ARENA_BLOCK_SIZE {
                let r_ret =
                    unsafe { os_read(fp.fd, &mut fp.eof, buf, read_remaining, fp.non_blocking) };
                if r_ret >= 0 {
                    read_remaining -= r_ret as usize;
                } else {
                    return r_ret;
                }
            } else {
                let write_ptr = unsafe { fp.buffer.as_mut_ptr().add(fp.write_pos) };
                let r_ret = unsafe {
                    os_read(
                        fp.fd,
                        &mut fp.eof,
                        write_ptr,
                        ARENA_BLOCK_SIZE,
                        fp.non_blocking,
                    )
                };
                if r_ret < 0 {
                    return r_ret;
                }
                let r_usize = r_ret as usize;
                fp.write_pos += r_usize;
                let to_copy = r_usize.min(read_remaining);
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        fp.buffer.as_ptr().add(fp.read_pos).cast::<u8>(),
                        buf.cast::<u8>(),
                        to_copy,
                    );
                }
                fp.read_pos += to_copy;
                read_remaining -= to_copy;
            }
        }
    }

    let total_read = size - read_remaining;
    fp.bytes_read += total_read as u64;
    total_read as isize
}

/// Try to read `size` already-buffered bytes in-place (`file_try_read_buffered`).
///
/// # Safety
/// `fp` must be a valid writable pointer to a `FileDescriptor`.
pub unsafe fn file_try_read_buffered(fp: *mut FileDescriptor, size: usize) -> *mut c_char {
    let fp = unsafe { &mut *fp };
    if fp.write_pos.saturating_sub(fp.read_pos) >= size {
        let ret = unsafe { fp.buffer.as_mut_ptr().add(fp.read_pos) };
        fp.read_pos += size;
        fp.bytes_read += size as u64;
        ret
    } else {
        std::ptr::null_mut()
    }
}

/// Write `size` bytes from `buf` to `fp` (`file_write`).
///
/// # Safety
/// `fp` must be a valid writable `FileDescriptor`, and `buf` must be valid for `size` bytes.
pub unsafe fn file_write(fp: *mut FileDescriptor, buf: *const c_char, size: usize) -> isize {
    let fp_ref = unsafe { &mut *fp };
    assert!(fp_ref.wr);
    if size < file_space(fp_ref) {
        if size > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    buf.cast::<u8>(),
                    fp_ref.buffer.as_mut_ptr().add(fp_ref.write_pos).cast::<u8>(),
                    size,
                );
            }
            fp_ref.write_pos += size;
        }
        return size as isize;
    }

    let status = unsafe { file_flush(fp) };
    if status < 0 {
        return status as isize;
    }

    if size < ARENA_BLOCK_SIZE {
        if size > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    buf.cast::<u8>(),
                    fp_ref.buffer.as_mut_ptr().add(fp_ref.write_pos).cast::<u8>(),
                    size,
                );
            }
            fp_ref.write_pos += size;
        }
        return size as isize;
    }

    let wres = unsafe { os_write(fp_ref.fd, buf, size, fp_ref.non_blocking) };
    if wres != size as isize && wres >= 0 {
        UV_EIO as isize
    } else {
        wres
    }
}

/// Skip `size` bytes forward in `fp` (`file_skip`).
///
/// # Safety
/// `fp` must be a valid readable `FileDescriptor`.
pub unsafe fn file_skip(fp: *mut FileDescriptor, size: usize) -> isize {
    let fp = unsafe { &mut *fp };
    assert!(!fp.wr);
    let buffered = fp.write_pos.saturating_sub(fp.read_pos);
    let from_buffer = buffered.min(size);
    let mut skip_remaining = size - from_buffer;
    if skip_remaining == 0 {
        fp.read_pos += from_buffer;
        fp.bytes_read += from_buffer as u64;
        return from_buffer as isize;
    }

    fp.read_pos = 0;
    fp.write_pos = 0;
    let mut called_read = false;
    while skip_remaining > 0 {
        if fp.eof || (called_read && fp.non_blocking) {
            break;
        }
        let r_ret = unsafe {
            os_read(
                fp.fd,
                &mut fp.eof,
                fp.buffer.as_mut_ptr(),
                ARENA_BLOCK_SIZE,
                fp.non_blocking,
            )
        };
        if r_ret < 0 {
            return r_ret;
        }
        let r_usize = r_ret as usize;
        if r_usize > skip_remaining {
            fp.read_pos = skip_remaining;
            fp.write_pos = r_usize;
            fp.bytes_read += size as u64;
            return size as isize;
        }
        skip_remaining -= r_usize;
        called_read = true;
    }

    let skipped = size - skip_remaining;
    fp.bytes_read += skipped as u64;
    skipped as isize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_buffer_read_and_skip() {
        let mut data = *b"0123456789abcdef";
        let mut fp = FileDescriptor::new();

        unsafe {
            file_open_buffer(&mut fp, data.as_mut_ptr().cast::<c_char>(), data.len());
            let chunk = file_try_read_buffered(&mut fp, 4);
            assert!(!chunk.is_null());
            assert_eq!(std::slice::from_raw_parts(chunk.cast::<u8>(), 4), b"0123");

            assert_eq!(file_skip(&mut fp, 6), 6);
            let mut out = [0u8; 6];
            assert_eq!(file_read(&mut fp, out.as_mut_ptr().cast::<c_char>(), 6), 6);
            assert_eq!(&out, b"abcdef");
            assert_eq!(fp.bytes_read, 16);
            assert_eq!(file_close(&mut fp, false), 0);
        }
    }

    #[test]
    fn file_open_write_flush_read_and_skip_on_disk() {
        let mut fcontents = Vec::with_capacity(4096);
        for _ in 0..16 {
            for b in 0..=255u8 {
                fcontents.push(b);
            }
        }
        let tmp = std::env::temp_dir().join(format!("nvim_os_fileio_test_{}", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        let c_path = std::ffi::CString::new(tmp.to_string_lossy().as_bytes()).unwrap();

        unsafe {
            let mut fp = FileDescriptor::new();
            assert_eq!(
                file_open(&mut fp, c_path.as_ptr(), K_FILE_CREATE_ONLY, 0o600),
                0
            );
            assert!(fp.wr);
            assert_eq!(
                file_write(&mut fp, fcontents.as_ptr().cast(), fcontents.len()),
                fcontents.len() as isize
            );
            assert_eq!(file_flush(&mut fp), 0);
            assert_eq!(file_fsync(&mut fp), 0);
            assert_eq!(file_close(&mut fp, true), 0);

            assert!(file_open(&mut fp, c_path.as_ptr(), K_FILE_CREATE_ONLY, 0o600) < 0);

            assert_eq!(
                file_open(&mut fp, c_path.as_ptr(), K_FILE_READ_ONLY, 0o600),
                0
            );
            assert!(!fp.wr);
            assert_eq!(file_skip(&mut fp, 3), 3);
            let mut buf3 = [0u8; 3];
            assert_eq!(file_read(&mut fp, buf3.as_mut_ptr().cast(), 3), 3);
            assert_eq!(&buf3, &fcontents[3..6]);
            assert_eq!(file_close(&mut fp, false), 0);

            assert_eq!(
                file_open(&mut fp, c_path.as_ptr(), K_FILE_READ_ONLY, 0o600),
                0
            );
            let mut all = vec![0u8; fcontents.len()];
            assert_eq!(
                file_read(&mut fp, all.as_mut_ptr().cast(), all.len()),
                fcontents.len() as isize
            );
            assert_eq!(all, fcontents);
            assert_eq!(file_close(&mut fp, false), 0);
        }
        let _ = std::fs::remove_file(&tmp);
    }
}
