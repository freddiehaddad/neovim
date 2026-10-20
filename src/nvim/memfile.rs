//! Memory file block cache and swap file backing store (`src/nvim/memfile.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unused_mut,
    clippy::missing_safety_doc
)]

#[cfg(not(test))]
use crate::errors::*;
use crate::eval::deprecated::buf_T;
#[cfg(not(test))]
use crate::eval::fs::FileInfo;
#[cfg(not(test))]
use crate::event::r#loop::loop_recursive_ptr;
#[cfg(all(not(test), windows))]
use crate::fileio::{_close, _lseeki64};
#[cfg(not(windows))]
#[cfg(not(test))]
use crate::fileio::{close, lseek};
#[cfg(not(test))]
use crate::fileio::{read_eintr, write_eintr};
#[cfg(not(test))]
use crate::globals::*;
use crate::map::{
    Map, Set, map_del_int64_t_int64_t, map_del_int64_t_ptr_t, map_put_ref_int64_t_int64_t,
    map_put_ref_int64_t_ptr_t, map_ref_int64_t_int64_t, mh_get_int64_t, ptr_t,
};
use crate::map_glyph_cache::{MH_TOMBSTONE, MapHash};
#[cfg(not(test))]
use crate::memline::{ml_get_buf, ml_open_file};
use crate::memory::{xfree, xmalloc};
#[cfg(not(test))]
use crate::message::{emsg, iemsg, semsg};
#[cfg(not(test))]
use crate::os::fs::{
    os_fileinfo_blocksize, os_fileinfo_fd, os_fileinfo_link, os_fsync, os_open, os_remove,
    os_set_cloexec,
};
#[cfg(not(test))]
use crate::os::input::os_breakcheck;
use crate::os::input::os_char_avail;
#[cfg(not(test))]
use crate::path::FullName_save;
#[cfg(not(test))]
use crate::strings::{gettext, strerror};
use std::ffi::{c_char, c_int, c_uint, c_void};
use std::ptr;

pub type blocknr_T = i64;
pub type mfdirty_T = c_int;
pub use crate::fileio::off_T;

const OK: c_int = 1;
const FAIL: c_int = 0;

const MEMFILE_PAGE_SIZE: c_uint = 4096;
const MIN_SWAP_PAGE_SIZE: u64 = 1048;
const MAX_SWAP_PAGE_SIZE: u64 = 50000;

const BH_DIRTY: c_uint = 1;
const BH_LOCKED: c_uint = 2;

const MF_DIRTY_NO: mfdirty_T = 0;
const MF_DIRTY_YES: mfdirty_T = 1;
const MF_DIRTY_YES_NOSYNC: mfdirty_T = 2;

const MFS_ALL: c_int = 1;
const MFS_STOP: c_int = 2;
const MFS_FLUSH: c_int = 4;
const MFS_ZERO: c_int = 8;

const SEEK_SET: c_int = 0;
const SEEK_END: c_int = 2;

#[cfg(windows)]
const O_RDWR: c_int = 0x0002;
#[cfg(windows)]
const O_CREAT: c_int = 0x0100;
#[cfg(windows)]
const O_TRUNC: c_int = 0x0200;
#[cfg(windows)]
const O_EXCL: c_int = 0x0400;
#[cfg(windows)]
const O_NOFOLLOW: c_int = 0;
#[cfg(windows)]
const S_IREAD: c_int = 0x0100;
#[cfg(windows)]
const S_IWRITE: c_int = 0x0080;

#[cfg(target_os = "linux")]
const O_RDWR: c_int = 0o2;
#[cfg(target_os = "linux")]
const O_CREAT: c_int = 0o100;
#[cfg(target_os = "linux")]
const O_EXCL: c_int = 0o200;
#[cfg(target_os = "linux")]
const O_TRUNC: c_int = 0o1000;
#[cfg(all(
    target_os = "linux",
    not(any(target_arch = "arm", target_arch = "aarch64"))
))]
const O_NOFOLLOW: c_int = 0o400000;
#[cfg(all(target_os = "linux", any(target_arch = "arm", target_arch = "aarch64")))]
const O_NOFOLLOW: c_int = 0o100000;
#[cfg(target_os = "linux")]
const S_IREAD: c_int = 0o400;
#[cfg(target_os = "linux")]
const S_IWRITE: c_int = 0o200;

#[cfg(not(any(windows, target_os = "linux")))]
const O_RDWR: c_int = 0x0002;
#[cfg(not(any(windows, target_os = "linux")))]
const O_CREAT: c_int = 0x0200;
#[cfg(not(any(windows, target_os = "linux")))]
const O_TRUNC: c_int = 0x0400;
#[cfg(not(any(windows, target_os = "linux")))]
const O_EXCL: c_int = 0x0800;
#[cfg(not(any(windows, target_os = "linux")))]
const O_NOFOLLOW: c_int = 0x0100;
#[cfg(not(any(windows, target_os = "linux")))]
const S_IREAD: c_int = 0o400;
#[cfg(not(any(windows, target_os = "linux")))]
const S_IWRITE: c_int = 0o200;

#[repr(C)]
pub struct bhdr_T {
    pub bh_bnum: blocknr_T,
    pub bh_data: *mut c_void,
    pub bh_page_count: c_uint,
    pub bh_flags: c_uint,
}

#[repr(C)]
pub struct memfile_T {
    pub mf_fname: *mut c_char,
    pub mf_ffname: *mut c_char,
    pub mf_fd: c_int,
    pub mf_flags: c_int,
    pub mf_reopen: bool,
    pub mf_free_first: *mut bhdr_T,
    pub mf_hash: Map<i64, ptr_t>,
    pub mf_trans: Map<i64, i64>,
    pub mf_blocknr_max: blocknr_T,
    pub mf_blocknr_min: blocknr_T,
    pub mf_neg_count: blocknr_T,
    pub mf_infile_count: blocknr_T,
    pub mf_page_size: c_uint,
    pub mf_dirty: mfdirty_T,
}

#[inline]
unsafe fn vim_lseek(fd: c_int, offset: off_T, whence: c_int) -> off_T {
    #[cfg(all(not(test), windows))]
    unsafe {
        _lseeki64(fd, offset, whence)
    }
    #[cfg(all(not(test), not(windows)))]
    unsafe {
        lseek(fd, offset, whence)
    }
    #[cfg(test)]
    {
        let _ = (fd, offset, whence);
        -1
    }
}

#[inline]
unsafe fn close_fd(fd: c_int) -> c_int {
    #[cfg(all(not(test), windows))]
    unsafe {
        _close(fd)
    }
    #[cfg(all(not(test), not(windows)))]
    unsafe {
        close(fd)
    }
    #[cfg(test)]
    {
        let _ = fd;
        0
    }
}

#[inline]
unsafe fn perror_msg(msg: *const c_char) {
    #[cfg(not(test))]
    unsafe {
        let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        let _ = semsg!(c"%s: %s".as_ptr(), gettext(msg), strerror(err));
    }
    #[cfg(test)]
    let _ = msg;
}

#[inline]
const fn empty_map<K, V>() -> Map<K, V> {
    Map {
        set: Set {
            h: MapHash {
                n_buckets: 0,
                size: 0,
                n_occupied: 0,
                upper_bound: 0,
                n_keys: 0,
                keys_capacity: 0,
                hash: ptr::null_mut(),
            },
            keys: ptr::null_mut(),
        },
        values: ptr::null_mut(),
    }
}

#[inline]
unsafe fn pmap_get_bhdr(map: *mut Map<i64, ptr_t>, key: i64) -> *mut bhdr_T {
    unsafe {
        let k = mh_get_int64_t(&raw mut (*map).set, key);
        if k == MH_TOMBSTONE {
            ptr::null_mut()
        } else {
            (*(*map).values.add(k as usize)).cast::<bhdr_T>()
        }
    }
}

#[inline]
unsafe fn pmap_put_bhdr(map: *mut Map<i64, ptr_t>, key: i64, val: *mut bhdr_T) {
    unsafe {
        let slot = map_put_ref_int64_t_ptr_t(map, key, ptr::null_mut(), ptr::null_mut());
        *slot = val.cast();
    }
}

#[inline]
unsafe fn map_put_i64(map: *mut Map<i64, i64>, key: i64, val: i64) {
    unsafe {
        let slot = map_put_ref_int64_t_int64_t(map, key, ptr::null_mut(), ptr::null_mut());
        *slot = val;
    }
}

#[inline]
unsafe fn destroy_map<K, V>(map: *mut Map<K, V>) {
    unsafe {
        xfree((*map).set.keys.cast());
        xfree((*map).set.h.hash.cast());
        xfree((*map).values.cast());
        *map = empty_map();
    }
}

/// Open a new or existing memory block file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mf_open(fname: *mut c_char, flags: c_int) -> *mut memfile_T {
    unsafe {
        let mfp = xmalloc(size_of::<memfile_T>()).cast::<memfile_T>();

        if fname.is_null() {
            (*mfp).mf_fname = ptr::null_mut();
            (*mfp).mf_ffname = ptr::null_mut();
            (*mfp).mf_fd = -1;
        } else if !mf_do_open(mfp, fname, flags) {
            xfree(mfp.cast());
            return ptr::null_mut();
        }

        (*mfp).mf_free_first = ptr::null_mut();
        (*mfp).mf_dirty = MF_DIRTY_NO;
        (*mfp).mf_hash = empty_map();
        (*mfp).mf_trans = empty_map();
        (*mfp).mf_page_size = MEMFILE_PAGE_SIZE;

        #[cfg(not(test))]
        if (*mfp).mf_fd >= 0 {
            let mut file_info = FileInfo::default();
            if os_fileinfo_fd((*mfp).mf_fd, &raw mut file_info) {
                let blocksize = os_fileinfo_blocksize(&raw const file_info);
                if (MIN_SWAP_PAGE_SIZE..=MAX_SWAP_PAGE_SIZE).contains(&blocksize) {
                    (*mfp).mf_page_size = blocksize as c_uint;
                }
            }
        }

        let size: off_T;
        if (*mfp).mf_fd < 0 || (flags & (O_TRUNC | O_EXCL)) != 0 || {
            size = vim_lseek((*mfp).mf_fd, 0, SEEK_END);
            size <= 0
        } {
            (*mfp).mf_blocknr_max = 0;
        } else {
            assert!(
                (*mfp).mf_page_size > 0 && i64::from((*mfp).mf_page_size - 1) <= i64::MAX - size
            );
            (*mfp).mf_blocknr_max =
                (size + i64::from((*mfp).mf_page_size) - 1) / i64::from((*mfp).mf_page_size);
        }
        (*mfp).mf_blocknr_min = -1;
        (*mfp).mf_neg_count = 0;
        (*mfp).mf_infile_count = (*mfp).mf_blocknr_max;

        mfp
    }
}

/// Open a file for an existing memfile.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mf_open_file(mfp: *mut memfile_T, fname: *mut c_char) -> c_int {
    unsafe {
        if mf_do_open(mfp, fname, O_RDWR | O_CREAT | O_EXCL) {
            (*mfp).mf_dirty = MF_DIRTY_YES;
            OK
        } else {
            FAIL
        }
    }
}

/// Close a memory file and optionally delete the associated file.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mf_close(mfp: *mut memfile_T, del_file: bool) {
    if mfp.is_null() {
        return;
    }
    unsafe {
        if (*mfp).mf_fd >= 0 && close_fd((*mfp).mf_fd) < 0 {
            #[cfg(not(test))]
            emsg(gettext(e_swapclose.as_ptr()));
        }
        if del_file && !(*mfp).mf_fname.is_null() {
            #[cfg(not(test))]
            os_remove((*mfp).mf_fname);
        }

        for i in 0..((*mfp).mf_hash.set.h.n_keys as usize) {
            let hp = (*(*mfp).mf_hash.values.add(i)).cast::<bhdr_T>();
            mf_free_bhdr(hp);
        }
        while !(*mfp).mf_free_first.is_null() {
            xfree(mf_rem_free(mfp).cast());
        }
        destroy_map(&raw mut (*mfp).mf_hash);
        destroy_map(&raw mut (*mfp).mf_trans);
        mf_free_fnames(mfp);
        xfree(mfp.cast());
    }
}

/// Close the swap file for a memfile. Used when 'swapfile' is reset.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mf_close_file(buf: *mut buf_T, getlines: bool) {
    unsafe {
        let mfp = (*buf).b_ml.ml_mfp.cast::<memfile_T>();
        if mfp.is_null() || (*mfp).mf_fd < 0 {
            return;
        }

        if getlines {
            for lnum in 1..=(*buf).b_ml.ml_line_count {
                #[cfg(not(test))]
                ml_get_buf(buf, lnum);
                #[cfg(test)]
                let _ = lnum;
            }
        }

        if close_fd((*mfp).mf_fd) < 0 {
            #[cfg(not(test))]
            emsg(gettext(e_swapclose.as_ptr()));
        }
        (*mfp).mf_fd = -1;

        if !(*mfp).mf_fname.is_null() {
            #[cfg(not(test))]
            os_remove((*mfp).mf_fname);
            mf_free_fnames(mfp);
        }
    }
}

/// Set new size for a memfile.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mf_new_page_size(mfp: *mut memfile_T, new_size: c_uint) {
    unsafe {
        (*mfp).mf_page_size = new_size;
    }
}

/// Get a new block.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mf_new(
    mfp: *mut memfile_T,
    negative: bool,
    page_count: c_uint,
) -> *mut bhdr_T {
    unsafe {
        let freep = (*mfp).mf_free_first;
        let hp: *mut bhdr_T =
            if !negative && !freep.is_null() && (*freep).bh_page_count >= page_count {
                if (*freep).bh_page_count > page_count {
                    let hp = mf_alloc_bhdr(mfp, page_count);
                    (*hp).bh_bnum = (*freep).bh_bnum;
                    (*freep).bh_bnum += i64::from(page_count);
                    (*freep).bh_page_count -= page_count;
                    hp
                } else {
                    let p = xmalloc(((*mfp).mf_page_size as usize) * (page_count as usize));
                    let hp = mf_rem_free(mfp);
                    (*hp).bh_data = p;
                    hp
                }
            } else {
                let hp = mf_alloc_bhdr(mfp, page_count);
                if negative {
                    (*hp).bh_bnum = (*mfp).mf_blocknr_min;
                    (*mfp).mf_blocknr_min -= 1;
                    (*mfp).mf_neg_count += 1;
                } else {
                    (*hp).bh_bnum = (*mfp).mf_blocknr_max;
                    (*mfp).mf_blocknr_max += i64::from(page_count);
                }
                hp
            };

        (*hp).bh_flags = BH_LOCKED | BH_DIRTY;
        (*mfp).mf_dirty = MF_DIRTY_YES;
        (*hp).bh_page_count = page_count;
        pmap_put_bhdr(&raw mut (*mfp).mf_hash, (*hp).bh_bnum, hp);

        ptr::write_bytes(
            (*hp).bh_data.cast::<u8>(),
            0,
            ((*mfp).mf_page_size as usize) * (page_count as usize),
        );

        hp
    }
}

/// Get existing block `nr` with `page_count` pages.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mf_get(
    mfp: *mut memfile_T,
    nr: blocknr_T,
    page_count: c_uint,
) -> *mut bhdr_T {
    unsafe {
        if nr >= (*mfp).mf_blocknr_max || nr <= (*mfp).mf_blocknr_min {
            return ptr::null_mut();
        }

        let mut hp = pmap_get_bhdr(&raw mut (*mfp).mf_hash, nr);
        if hp.is_null() {
            if nr < 0 || nr >= (*mfp).mf_infile_count {
                return ptr::null_mut();
            }

            if page_count > 0 {
                hp = mf_alloc_bhdr(mfp, page_count);
            }
            if hp.is_null() {
                return ptr::null_mut();
            }

            (*hp).bh_bnum = nr;
            (*hp).bh_flags = 0;
            (*hp).bh_page_count = page_count;
            if mf_read(mfp, hp) == FAIL {
                mf_free_bhdr(hp);
                return ptr::null_mut();
            }
        } else {
            let _ = map_del_int64_t_ptr_t(&raw mut (*mfp).mf_hash, (*hp).bh_bnum, ptr::null_mut());
        }

        (*hp).bh_flags |= BH_LOCKED;
        pmap_put_bhdr(&raw mut (*mfp).mf_hash, (*hp).bh_bnum, hp);

        hp
    }
}

/// Release the block `*hp`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mf_put(mfp: *mut memfile_T, hp: *mut bhdr_T, dirty: bool, infile: bool) {
    unsafe {
        let mut flags = (*hp).bh_flags;

        if (flags & BH_LOCKED) == 0 {
            #[cfg(not(test))]
            iemsg(gettext(c"E293: Block was not locked".as_ptr()));
        }
        flags &= !BH_LOCKED;
        if dirty {
            flags |= BH_DIRTY;
            if (*mfp).mf_dirty != MF_DIRTY_YES_NOSYNC {
                (*mfp).mf_dirty = MF_DIRTY_YES;
            }
        }
        (*hp).bh_flags = flags;
        if infile {
            let _ = mf_trans_add(mfp, hp);
        }
    }
}

/// Signal block as no longer used (may put it in the free list).
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mf_free(mfp: *mut memfile_T, hp: *mut bhdr_T) {
    unsafe {
        xfree((*hp).bh_data);
        let _ = map_del_int64_t_ptr_t(&raw mut (*mfp).mf_hash, (*hp).bh_bnum, ptr::null_mut());
        if (*hp).bh_bnum < 0 {
            xfree(hp.cast());
            (*mfp).mf_neg_count -= 1;
        } else {
            mf_ins_free(mfp, hp);
        }
    }
}

/// Sync memory file to disk.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mf_sync(mfp: *mut memfile_T, flags: c_int) -> c_int {
    unsafe {
        #[cfg(not(test))]
        let got_int_save = got_int;

        if (*mfp).mf_fd < 0 {
            (*mfp).mf_dirty = MF_DIRTY_NO;
            return FAIL;
        }

        #[cfg(not(test))]
        {
            got_int = false;
        }

        let mut status = OK;
        let mut hp: *mut bhdr_T = ptr::null_mut();

        let mut i: u32 = 0;
        while i < (*mfp).mf_hash.set.h.n_keys {
            hp = (*(*mfp).mf_hash.values.add(i as usize)).cast::<bhdr_T>();
            i += 1;

            if ((flags & MFS_ALL) != 0 || (*hp).bh_bnum >= 0)
                && ((*hp).bh_flags & BH_DIRTY) != 0
                && (status == OK || ((*hp).bh_bnum >= 0 && (*hp).bh_bnum < (*mfp).mf_infile_count))
            {
                if (flags & MFS_ZERO) != 0 && (*hp).bh_bnum != 0 {
                    continue;
                }
                if mf_write(mfp, hp) == FAIL {
                    if status == FAIL {
                        break;
                    }
                    status = FAIL;
                }
                if (flags & MFS_STOP) != 0 {
                    if os_char_avail() {
                        break;
                    }
                } else {
                    #[cfg(not(test))]
                    if *loop_recursive_ptr((&raw mut main_loop).cast()) == 0 {
                        os_breakcheck();
                    }
                }
                #[cfg(not(test))]
                if got_int {
                    break;
                }
            }
        }

        if hp.is_null() || status == FAIL {
            (*mfp).mf_dirty = MF_DIRTY_NO;
        }

        #[cfg(not(test))]
        if (flags & MFS_FLUSH) != 0 && os_fsync((*mfp).mf_fd) != 0 {
            status = FAIL;
        }

        #[cfg(not(test))]
        {
            got_int |= got_int_save;
        }

        status
    }
}

/// Set dirty flag for all blocks in memory file with a positive block number.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mf_set_dirty(mfp: *mut memfile_T) {
    unsafe {
        for i in 0..((*mfp).mf_hash.set.h.n_keys as usize) {
            let hp = (*(*mfp).mf_hash.values.add(i)).cast::<bhdr_T>();
            if (*hp).bh_bnum > 0 {
                (*hp).bh_flags |= BH_DIRTY;
            }
        }
        (*mfp).mf_dirty = MF_DIRTY_YES;
    }
}

/// Release as many blocks as possible.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mf_release_all() -> bool {
    let mut retval = false;
    #[cfg(not(test))]
    unsafe {
        let mut buf = firstbuf;
        while !buf.is_null() {
            let mfp = (*buf).b_ml.ml_mfp.cast::<memfile_T>();
            if !mfp.is_null() {
                if (*mfp).mf_fd < 0 && (*buf).b_may_swap {
                    ml_open_file(buf);
                }

                if (*mfp).mf_fd >= 0 {
                    let mut i: usize = 0;
                    while i < ((*mfp).mf_hash.set.h.size as usize) {
                        let hp = (*(*mfp).mf_hash.values.add(i)).cast::<bhdr_T>();
                        if ((*hp).bh_flags & BH_LOCKED) == 0
                            && (((*hp).bh_flags & BH_DIRTY) == 0 || mf_write(mfp, hp) != FAIL)
                        {
                            let _ = map_del_int64_t_ptr_t(
                                &raw mut (*mfp).mf_hash,
                                (*hp).bh_bnum,
                                ptr::null_mut(),
                            );
                            mf_free_bhdr(hp);
                            retval = true;
                        } else {
                            i += 1;
                        }
                    }
                }
            }
            buf = (*buf).b_next;
        }
    }
    retval
}

unsafe fn mf_alloc_bhdr(mfp: *mut memfile_T, page_count: c_uint) -> *mut bhdr_T {
    unsafe {
        let hp = xmalloc(size_of::<bhdr_T>()).cast::<bhdr_T>();
        (*hp).bh_data = xmalloc(((*mfp).mf_page_size as usize) * (page_count as usize));
        (*hp).bh_page_count = page_count;
        hp
    }
}

unsafe fn mf_free_bhdr(hp: *mut bhdr_T) {
    unsafe {
        xfree((*hp).bh_data);
        xfree(hp.cast());
    }
}

unsafe fn mf_ins_free(mfp: *mut memfile_T, hp: *mut bhdr_T) {
    unsafe {
        (*hp).bh_data = (*mfp).mf_free_first.cast();
        (*mfp).mf_free_first = hp;
    }
}

unsafe fn mf_rem_free(mfp: *mut memfile_T) -> *mut bhdr_T {
    unsafe {
        let hp = (*mfp).mf_free_first;
        (*mfp).mf_free_first = (*hp).bh_data.cast();
        hp
    }
}

unsafe fn mf_read(mfp: *mut memfile_T, hp: *mut bhdr_T) -> c_int {
    unsafe {
        if (*mfp).mf_fd < 0 {
            return FAIL;
        }

        let page_size = (*mfp).mf_page_size;
        let offset = i64::from(page_size).wrapping_mul((*hp).bh_bnum);
        if vim_lseek((*mfp).mf_fd, offset, SEEK_SET) != offset {
            perror_msg(c"E294: Seek error in swap file read".as_ptr());
            return FAIL;
        }
        assert!((*hp).bh_page_count <= c_uint::MAX / page_size);
        let size = (page_size * (*hp).bh_page_count) as usize;
        #[cfg(not(test))]
        let nread = read_eintr((*mfp).mf_fd, (*hp).bh_data, size);
        #[cfg(test)]
        let nread: isize = -1;
        if (nread as usize) != size {
            perror_msg(c"E295: Read error in swap file".as_ptr());
            return FAIL;
        }

        OK
    }
}

unsafe fn mf_write(mfp: *mut memfile_T, hp: *mut bhdr_T) -> c_int {
    unsafe {
        if (*mfp).mf_fd < 0 && !(*mfp).mf_reopen {
            return FAIL;
        }

        if (*hp).bh_bnum < 0 && mf_trans_add(mfp, hp) == FAIL {
            return FAIL;
        }

        let page_size = (*mfp).mf_page_size;

        loop {
            let mut nr = (*hp).bh_bnum;
            let hp2: *mut bhdr_T = if nr > (*mfp).mf_infile_count {
                nr = (*mfp).mf_infile_count;
                pmap_get_bhdr(&raw mut (*mfp).mf_hash, nr)
            } else {
                hp
            };

            let offset = i64::from(page_size).wrapping_mul(nr);
            let page_count = if hp2.is_null() {
                1
            } else {
                (*hp2).bh_page_count
            };
            let size = (page_size * page_count) as usize;

            for attempt in 1..=2 {
                if (*mfp).mf_fd >= 0 {
                    if vim_lseek((*mfp).mf_fd, offset, SEEK_SET) != offset {
                        perror_msg(c"E296: Seek error in swap file write".as_ptr());
                        return FAIL;
                    }
                    let data = if hp2.is_null() {
                        (*hp).bh_data
                    } else {
                        (*hp2).bh_data
                    };
                    #[cfg(not(test))]
                    let nwritten = write_eintr((*mfp).mf_fd, data, size);
                    #[cfg(test)]
                    let nwritten = {
                        let _ = data;
                        -1isize
                    };
                    if (nwritten as usize) == size {
                        break;
                    }
                }

                if attempt == 1 {
                    if (*mfp).mf_fd >= 0 {
                        let _ = close_fd((*mfp).mf_fd);
                    }
                    #[cfg(not(test))]
                    {
                        (*mfp).mf_fd =
                            os_open((*mfp).mf_fname, (*mfp).mf_flags, S_IREAD | S_IWRITE);
                    }
                    (*mfp).mf_reopen = (*mfp).mf_fd < 0;
                }
                if attempt == 2 || (*mfp).mf_fd < 0 {
                    #[cfg(not(test))]
                    {
                        if !did_swapwrite_msg {
                            emsg(gettext(c"E297: Write error in swap file".as_ptr()));
                        }
                        did_swapwrite_msg = true;
                    }
                    return FAIL;
                }
            }

            #[cfg(not(test))]
            {
                did_swapwrite_msg = false;
            }
            if !hp2.is_null() {
                (*hp2).bh_flags &= !BH_DIRTY;
            }
            if nr + i64::from(page_count) > (*mfp).mf_infile_count {
                (*mfp).mf_infile_count = nr + i64::from(page_count);
            }
            if nr == (*hp).bh_bnum {
                break;
            }
        }
        OK
    }
}

unsafe fn mf_trans_add(mfp: *mut memfile_T, hp: *mut bhdr_T) -> c_int {
    unsafe {
        if (*hp).bh_bnum >= 0 {
            return OK;
        }

        let freep = (*mfp).mf_free_first;
        let page_count = (*hp).bh_page_count;
        let new_bnum: blocknr_T = if !freep.is_null() && (*freep).bh_page_count >= page_count {
            let bnum = (*freep).bh_bnum;
            if (*freep).bh_page_count > page_count {
                (*freep).bh_bnum += i64::from(page_count);
                (*freep).bh_page_count -= page_count;
            } else {
                let rem = mf_rem_free(mfp);
                xfree(rem.cast());
            }
            bnum
        } else {
            let bnum = (*mfp).mf_blocknr_max;
            (*mfp).mf_blocknr_max += i64::from(page_count);
            bnum
        };

        let old_bnum = (*hp).bh_bnum;
        let _ = map_del_int64_t_ptr_t(&raw mut (*mfp).mf_hash, (*hp).bh_bnum, ptr::null_mut());
        (*hp).bh_bnum = new_bnum;
        pmap_put_bhdr(&raw mut (*mfp).mf_hash, new_bnum, hp);

        map_put_i64(&raw mut (*mfp).mf_trans, old_bnum, new_bnum);

        OK
    }
}

/// Lookup translation from trans list and delete the entry.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mf_trans_del(mfp: *mut memfile_T, old_nr: blocknr_T) -> blocknr_T {
    unsafe {
        let num = map_ref_int64_t_int64_t(&raw mut (*mfp).mf_trans, old_nr, ptr::null_mut());
        if num.is_null() {
            return old_nr;
        }

        (*mfp).mf_neg_count -= 1;
        let new_bnum = *num;

        let _ = map_del_int64_t_int64_t(&raw mut (*mfp).mf_trans, old_nr, ptr::null_mut());

        new_bnum
    }
}

/// Frees `mf_fname` and `mf_ffname`.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mf_free_fnames(mfp: *mut memfile_T) {
    unsafe {
        xfree((*mfp).mf_fname.cast());
        (*mfp).mf_fname = ptr::null_mut();
        xfree((*mfp).mf_ffname.cast());
        (*mfp).mf_ffname = ptr::null_mut();
    }
}

/// Set the simple file name and the full file name of memfile's swapfile.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mf_set_fnames(mfp: *mut memfile_T, fname: *mut c_char) {
    unsafe {
        (*mfp).mf_fname = fname;
        #[cfg(not(test))]
        {
            (*mfp).mf_ffname = FullName_save((*mfp).mf_fname, false);
        }
        #[cfg(test)]
        {
            (*mfp).mf_ffname = ptr::null_mut();
        }
    }
}

/// Make name of memfile's swapfile a full path.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mf_fullname(mfp: *mut memfile_T) {
    if mfp.is_null() {
        return;
    }
    unsafe {
        if (*mfp).mf_fname.is_null() || (*mfp).mf_ffname.is_null() {
            return;
        }

        xfree((*mfp).mf_fname.cast());
        (*mfp).mf_fname = (*mfp).mf_ffname;
        (*mfp).mf_ffname = ptr::null_mut();
    }
}

/// Return true if there are any translations pending for memfile.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn mf_need_trans(mfp: *mut memfile_T) -> bool {
    unsafe { !(*mfp).mf_fname.is_null() && (*mfp).mf_neg_count > 0 }
}

unsafe fn mf_do_open(mfp: *mut memfile_T, fname: *mut c_char, mut flags: c_int) -> bool {
    unsafe {
        mf_set_fnames(mfp, fname);
        assert!(!(*mfp).mf_fname.is_null());

        #[cfg(not(test))]
        {
            let mut file_info = FileInfo::default();
            if (flags & O_CREAT) != 0 && os_fileinfo_link((*mfp).mf_fname, &raw mut file_info) {
                (*mfp).mf_fd = -1;
                emsg(gettext(
                    c"E300: Swap file already exists (symlink attack?)".as_ptr(),
                ));
            } else {
                flags |= O_NOFOLLOW;
                (*mfp).mf_flags = flags;
                (*mfp).mf_fd = os_open((*mfp).mf_fname, flags, S_IREAD | S_IWRITE);
            }
        }
        #[cfg(test)]
        {
            let _ = &mut flags;
            (*mfp).mf_fd = -1;
        }

        if (*mfp).mf_fd < 0 {
            mf_free_fnames(mfp);
            return false;
        }

        #[cfg(not(test))]
        os_set_cloexec((*mfp).mf_fd);

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_only_memfile_allocates_gets_puts_and_frees_blocks() {
        unsafe {
            let mfp = mf_open(ptr::null_mut(), 0);
            assert!(!mfp.is_null());
            assert_eq!((*mfp).mf_page_size, 4096);

            let hp1 = mf_new(mfp, true, 1);
            assert!(!hp1.is_null());
            assert_eq!((*hp1).bh_bnum, -1);
            mf_put(mfp, hp1, true, true);
            assert_eq!(mf_trans_del(mfp, -1), 0);

            let hp2 = mf_get(mfp, 0, 1);
            assert_eq!(hp2, hp1);
            mf_put(mfp, hp2, false, false);
            mf_free(mfp, hp2);

            mf_close(mfp, false);
        }
    }

    #[test]
    fn mf_hash_inserts_expands_removes_and_frees_50000_items() {
        const TEST_COUNT: usize = 50_000;
        const fn index_to_key(i: usize) -> blocknr_T {
            (i as blocknr_T) ^ 15167
        }

        unsafe {
            let mut ht: Map<i64, ptr_t> = empty_map();
            let mut items: Vec<*mut bhdr_T> = Vec::with_capacity(TEST_COUNT);

            for i in 0..TEST_COUNT {
                assert_eq!(ht.set.h.size as usize, i);
                let key = index_to_key(i);
                assert!(pmap_get_bhdr(&raw mut ht, key).is_null());

                let item = xmalloc(size_of::<bhdr_T>()).cast::<bhdr_T>();
                ptr::write(
                    item,
                    bhdr_T {
                        bh_bnum: key,
                        bh_data: ptr::null_mut(),
                        bh_page_count: 1,
                        bh_flags: 0,
                    },
                );
                pmap_put_bhdr(&raw mut ht, key, item);
                assert_eq!(pmap_get_bhdr(&raw mut ht, key), item);
                let num_buckets = ht.set.h.n_buckets as usize;
                assert!(num_buckets.is_power_of_two());
                items.push(item);
            }

            for i in 0..TEST_COUNT {
                let key = index_to_key(i);
                let item = pmap_get_bhdr(&raw mut ht, key);
                assert!(!item.is_null());
                assert_eq!((*item).bh_bnum, key);
            }

            for i in 0..TEST_COUNT {
                if i % 100 < 70 {
                    let key = index_to_key(i);
                    let item = pmap_get_bhdr(&raw mut ht, key);
                    assert!(!item.is_null());
                    assert_eq!((*item).bh_bnum, key);

                    map_del_int64_t_ptr_t(&raw mut ht, key, ptr::null_mut());
                    assert!(pmap_get_bhdr(&raw mut ht, key).is_null());

                    pmap_put_bhdr(&raw mut ht, key, item);
                    assert_eq!(pmap_get_bhdr(&raw mut ht, key), item);

                    map_del_int64_t_ptr_t(&raw mut ht, key, ptr::null_mut());
                    assert!(pmap_get_bhdr(&raw mut ht, key).is_null());

                    xfree(item.cast());
                }
            }

            for i in 0..TEST_COUNT {
                let key = index_to_key(i);
                let item = pmap_get_bhdr(&raw mut ht, key);
                if i % 100 < 70 {
                    assert!(item.is_null());
                } else {
                    assert!(!item.is_null());
                    assert_eq!((*item).bh_bnum, key);
                    xfree(item.cast());
                }
            }

            destroy_map(&raw mut ht);
        }
    }
}
