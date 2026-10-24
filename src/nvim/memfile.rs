//! Memory file block cache and swap file backing store (`src/nvim/memfile.c`).

use crate::buffer::{MF_DIRTY_YES, MF_DIRTY_YES_NOSYNC, buf_T};
use crate::errors::*;
use crate::event::r#loop::loop_recursive_ptr;
use crate::fileio::{SEEK_END, SEEK_SET, close, off_T, read_eintr, vim_lseek, write_eintr};
use crate::globals::*;
use crate::memline::{MFS_FLUSH, ml_get_buf, ml_open_file};
use crate::memory::{xfree, xmalloc};
use crate::message::{emsg, iemsg, semsg};
use crate::os::fs::{
    FileInfo, O_CREAT, O_EXCL, O_NOFOLLOW, O_RDWR, O_TRUNC, os_fileinfo_blocksize, os_fileinfo_fd,
    os_fileinfo_link, os_fsync, os_open, os_remove, os_set_cloexec,
};
use crate::os::input::{os_breakcheck, os_char_avail};
use crate::path::FullName_save;
use crate::strings::{gettext, strerror};
use std::ffi::{c_char, c_int, c_uint, c_void};
use std::ptr;

pub type blocknr_T = i64;
pub type mfdirty_T = c_int;
use crate::math::{FAIL, OK};

const MEMFILE_PAGE_SIZE: c_uint = 4096;
pub(crate) const MIN_SWAP_PAGE_SIZE: c_uint = 1048;
const MAX_SWAP_PAGE_SIZE: u64 = 50000;

pub(crate) const BH_DIRTY: c_uint = 1;
const BH_LOCKED: c_uint = 2;

const MF_DIRTY_NO: mfdirty_T = 0;

pub(crate) const MFS_ALL: c_int = 1;
pub(crate) const MFS_STOP: c_int = 2;
pub(crate) const MFS_ZERO: c_int = 8;
#[cfg(windows)]
const S_IREAD: c_int = 0x0100;
#[cfg(windows)]
const S_IWRITE: c_int = 0x0080;
#[cfg(target_os = "linux")]
const S_IREAD: c_int = 0o400;
#[cfg(target_os = "linux")]
const S_IWRITE: c_int = 0o200;
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
    pub mf_hash: std::collections::HashMap<i64, *mut bhdr_T>,
    pub mf_trans: std::collections::HashMap<i64, i64>,
    pub mf_blocknr_max: blocknr_T,
    pub mf_blocknr_min: blocknr_T,
    pub mf_neg_count: blocknr_T,
    pub mf_infile_count: blocknr_T,
    pub mf_page_size: c_uint,
    pub mf_dirty: mfdirty_T,
}

#[inline]
unsafe fn perror_msg(msg: *const c_char) {
    unsafe {
        let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        let _ = semsg!(c"%s: %s".as_ptr(), gettext(msg), strerror(err));
    }
}

#[inline]
pub unsafe fn pmap_get_bhdr(
    map: *const std::collections::HashMap<i64, *mut bhdr_T>,
    key: i64,
) -> *mut bhdr_T {
    unsafe { (*map).get(&key).copied().unwrap_or(ptr::null_mut()) }
}

#[inline]
unsafe fn pmap_put_bhdr(
    map: *mut std::collections::HashMap<i64, *mut bhdr_T>,
    key: i64,
    val: *mut bhdr_T,
) {
    unsafe {
        (*map).insert(key, val);
    }
}

#[inline]
unsafe fn map_put_i64(map: *mut std::collections::HashMap<i64, i64>, key: i64, val: i64) {
    unsafe {
        (*map).insert(key, val);
    }
}

/// Open a new or existing memory block file.
pub unsafe fn mf_open(fname: *mut c_char, flags: c_int) -> *mut memfile_T {
    unsafe {
        let mfp = Box::into_raw(Box::new(memfile_T {
            mf_fname: ptr::null_mut(),
            mf_ffname: ptr::null_mut(),
            mf_fd: -1,
            mf_flags: 0,
            mf_reopen: false,
            mf_free_first: ptr::null_mut(),
            mf_hash: std::collections::HashMap::new(),
            mf_trans: std::collections::HashMap::new(),
            mf_blocknr_max: 0,
            mf_blocknr_min: -1,
            mf_neg_count: 0,
            mf_infile_count: 0,
            mf_page_size: MEMFILE_PAGE_SIZE,
            mf_dirty: MF_DIRTY_NO,
        }));

        if !fname.is_null() && !mf_do_open(mfp, fname, flags) {
            drop(Box::from_raw(mfp));
            return ptr::null_mut();
        }

        if (*mfp).mf_fd >= 0 {
            let mut file_info = FileInfo::default();
            if os_fileinfo_fd((*mfp).mf_fd, &raw mut file_info) {
                let blocksize = os_fileinfo_blocksize(&raw const file_info);
                if (u64::from(MIN_SWAP_PAGE_SIZE)..=MAX_SWAP_PAGE_SIZE).contains(&blocksize) {
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
pub unsafe fn mf_open_file(mfp: *mut memfile_T, fname: *mut c_char) -> c_int {
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
pub unsafe fn mf_close(mfp: *mut memfile_T, del_file: bool) {
    if mfp.is_null() {
        return;
    }
    unsafe {
        if (*mfp).mf_fd >= 0 && close((*mfp).mf_fd) < 0 {
            emsg(gettext(e_swapclose.as_ptr()));
        }
        if del_file && !(*mfp).mf_fname.is_null() {
            os_remove((*mfp).mf_fname);
        }

        for (_, hp) in (*mfp).mf_hash.drain() {
            mf_free_bhdr(hp);
        }
        while !(*mfp).mf_free_first.is_null() {
            drop(Box::from_raw(mf_rem_free(mfp)));
        }
        mf_free_fnames(mfp);
        drop(Box::from_raw(mfp));
    }
}

/// Close the swap file for a memfile. Used when 'swapfile' is reset.
pub unsafe fn mf_close_file(buf: *mut buf_T, getlines: bool) {
    unsafe {
        let mfp = (*buf).b_ml.ml_mfp.cast::<memfile_T>();
        if mfp.is_null() || (*mfp).mf_fd < 0 {
            return;
        }

        if getlines {
            for lnum in 1..=(*buf).b_ml.ml_line_count {
                ml_get_buf(buf, lnum);
            }
        }

        if close((*mfp).mf_fd) < 0 {
            emsg(gettext(e_swapclose.as_ptr()));
        }
        (*mfp).mf_fd = -1;

        if !(*mfp).mf_fname.is_null() {
            os_remove((*mfp).mf_fname);
            mf_free_fnames(mfp);
        }
    }
}

/// Set new size for a memfile.
pub unsafe fn mf_new_page_size(mfp: *mut memfile_T, new_size: c_uint) {
    unsafe {
        (*mfp).mf_page_size = new_size;
    }
}

/// Get a new block.
pub unsafe fn mf_new(mfp: *mut memfile_T, negative: bool, page_count: c_uint) -> *mut bhdr_T {
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
pub unsafe fn mf_get(mfp: *mut memfile_T, nr: blocknr_T, page_count: c_uint) -> *mut bhdr_T {
    unsafe {
        if nr >= (*mfp).mf_blocknr_max || nr <= (*mfp).mf_blocknr_min {
            return ptr::null_mut();
        }

        let mut hp = pmap_get_bhdr(&raw const (*mfp).mf_hash, nr);
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
            let _ = (*mfp).mf_hash.remove(&(*hp).bh_bnum);
        }

        (*hp).bh_flags |= BH_LOCKED;
        pmap_put_bhdr(&raw mut (*mfp).mf_hash, (*hp).bh_bnum, hp);

        hp
    }
}

/// Release the block `*hp`.
pub unsafe fn mf_put(mfp: *mut memfile_T, hp: *mut bhdr_T, dirty: bool, infile: bool) {
    unsafe {
        let mut flags = (*hp).bh_flags;

        if (flags & BH_LOCKED) == 0 {
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
pub unsafe fn mf_free(mfp: *mut memfile_T, hp: *mut bhdr_T) {
    unsafe {
        xfree((*hp).bh_data);
        let _ = (*mfp).mf_hash.remove(&(*hp).bh_bnum);
        if (*hp).bh_bnum < 0 {
            xfree(hp.cast());
            (*mfp).mf_neg_count -= 1;
        } else {
            mf_ins_free(mfp, hp);
        }
    }
}

/// Sync memory file to disk.
pub unsafe fn mf_sync(mfp: *mut memfile_T, flags: c_int) -> c_int {
    unsafe {
        let got_int_save = got_int;

        if (*mfp).mf_fd < 0 {
            (*mfp).mf_dirty = MF_DIRTY_NO;
            return FAIL;
        }

        {
            got_int = false;
        }

        let mut status = OK;
        let mut completed_all = true;
        let blocks: Vec<*mut bhdr_T> = (*mfp).mf_hash.values().copied().collect();

        for hp in blocks {
            if ((flags & MFS_ALL) != 0 || (*hp).bh_bnum >= 0)
                && ((*hp).bh_flags & BH_DIRTY) != 0
                && (status == OK || ((*hp).bh_bnum >= 0 && (*hp).bh_bnum < (*mfp).mf_infile_count))
            {
                if (flags & MFS_ZERO) != 0 && (*hp).bh_bnum != 0 {
                    continue;
                }
                if mf_write(mfp, hp) == FAIL {
                    if status == FAIL {
                        completed_all = false;
                        break;
                    }
                    status = FAIL;
                }
                if (flags & MFS_STOP) != 0 {
                    if os_char_avail() {
                        completed_all = false;
                        break;
                    }
                } else {
                    if *loop_recursive_ptr((&raw mut main_loop).cast()) == 0 {
                        os_breakcheck();
                    }
                }
                if got_int {
                    completed_all = false;
                    break;
                }
            }
        }

        if completed_all || status == FAIL {
            (*mfp).mf_dirty = MF_DIRTY_NO;
        }

        if (flags & MFS_FLUSH) != 0 && os_fsync((*mfp).mf_fd) != 0 {
            status = FAIL;
        }

        {
            got_int |= got_int_save;
        }

        status
    }
}

/// Set dirty flag for all blocks in memory file with a positive block number.
pub unsafe fn mf_set_dirty(mfp: *mut memfile_T) {
    unsafe {
        for &hp in (*mfp).mf_hash.values() {
            if (*hp).bh_bnum > 0 {
                (*hp).bh_flags |= BH_DIRTY;
            }
        }
        (*mfp).mf_dirty = MF_DIRTY_YES;
    }
}

/// Release as many blocks as possible.
pub unsafe fn mf_release_all() -> bool {
    let mut retval = false;
    unsafe {
        let mut buf = firstbuf;
        while !buf.is_null() {
            let mfp = (*buf).b_ml.ml_mfp.cast::<memfile_T>();
            if !mfp.is_null() {
                if (*mfp).mf_fd < 0 && (*buf).b_may_swap {
                    ml_open_file(buf);
                }

                if (*mfp).mf_fd >= 0 {
                    let blocks: Vec<*mut bhdr_T> = (*mfp).mf_hash.values().copied().collect();
                    for hp in blocks {
                        if ((*hp).bh_flags & BH_LOCKED) == 0
                            && (((*hp).bh_flags & BH_DIRTY) == 0 || mf_write(mfp, hp) != FAIL)
                        {
                            let _ = (*mfp).mf_hash.remove(&(*hp).bh_bnum);
                            mf_free_bhdr(hp);
                            retval = true;
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
        Box::into_raw(Box::new(bhdr_T {
            bh_bnum: 0,
            bh_data: xmalloc(((*mfp).mf_page_size as usize) * (page_count as usize)),
            bh_page_count: page_count,
            bh_flags: 0,
        }))
    }
}

unsafe fn mf_free_bhdr(hp: *mut bhdr_T) {
    unsafe {
        xfree((*hp).bh_data);
        drop(Box::from_raw(hp));
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
        let nread = read_eintr((*mfp).mf_fd, (*hp).bh_data, size);
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
                pmap_get_bhdr(&raw const (*mfp).mf_hash, nr)
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
                    let nwritten = write_eintr((*mfp).mf_fd, data, size);
                    if (nwritten as usize) == size {
                        break;
                    }
                }

                if attempt == 1 {
                    if (*mfp).mf_fd >= 0 {
                        let _ = close((*mfp).mf_fd);
                    }
                    {
                        (*mfp).mf_fd =
                            os_open((*mfp).mf_fname, (*mfp).mf_flags, S_IREAD | S_IWRITE);
                    }
                    (*mfp).mf_reopen = (*mfp).mf_fd < 0;
                }
                if attempt == 2 || (*mfp).mf_fd < 0 {
                    {
                        if !did_swapwrite_msg {
                            emsg(gettext(c"E297: Write error in swap file".as_ptr()));
                        }
                        did_swapwrite_msg = true;
                    }
                    return FAIL;
                }
            }

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
        let _ = (*mfp).mf_hash.remove(&(*hp).bh_bnum);
        (*hp).bh_bnum = new_bnum;
        pmap_put_bhdr(&raw mut (*mfp).mf_hash, new_bnum, hp);

        map_put_i64(&raw mut (*mfp).mf_trans, old_bnum, new_bnum);

        OK
    }
}

/// Lookup translation from trans list and delete the entry.
pub unsafe fn mf_trans_del(mfp: *mut memfile_T, old_nr: blocknr_T) -> blocknr_T {
    unsafe {
        let Some(new_bnum) = (*mfp).mf_trans.remove(&old_nr) else {
            return old_nr;
        };

        (*mfp).mf_neg_count -= 1;

        new_bnum
    }
}

/// Frees `mf_fname` and `mf_ffname`.
pub unsafe fn mf_free_fnames(mfp: *mut memfile_T) {
    unsafe {
        xfree((*mfp).mf_fname.cast());
        (*mfp).mf_fname = ptr::null_mut();
        xfree((*mfp).mf_ffname.cast());
        (*mfp).mf_ffname = ptr::null_mut();
    }
}

/// Set the simple file name and the full file name of memfile's swapfile.
pub unsafe fn mf_set_fnames(mfp: *mut memfile_T, fname: *mut c_char) {
    unsafe {
        (*mfp).mf_fname = fname;
        {
            (*mfp).mf_ffname = FullName_save((*mfp).mf_fname, false);
        }
    }
}

/// Make name of memfile's swapfile a full path.
pub unsafe fn mf_fullname(mfp: *mut memfile_T) {
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
pub unsafe fn mf_need_trans(mfp: *mut memfile_T) -> bool {
    unsafe { !(*mfp).mf_fname.is_null() && (*mfp).mf_neg_count > 0 }
}

unsafe fn mf_do_open(mfp: *mut memfile_T, fname: *mut c_char, mut flags: c_int) -> bool {
    unsafe {
        mf_set_fnames(mfp, fname);
        assert!(!(*mfp).mf_fname.is_null());

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

        if (*mfp).mf_fd < 0 {
            mf_free_fnames(mfp);
            return false;
        }

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
            let mut ht: std::collections::HashMap<i64, *mut bhdr_T> =
                std::collections::HashMap::new();
            let mut items: Vec<*mut bhdr_T> = Vec::with_capacity(TEST_COUNT);

            for i in 0..TEST_COUNT {
                assert_eq!(ht.len(), i);
                let key = index_to_key(i);
                assert!(pmap_get_bhdr(&raw const ht, key).is_null());

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
                assert_eq!(pmap_get_bhdr(&raw const ht, key), item);
                items.push(item);
            }

            for i in 0..TEST_COUNT {
                let key = index_to_key(i);
                let item = pmap_get_bhdr(&raw const ht, key);
                assert!(!item.is_null());
                assert_eq!((*item).bh_bnum, key);
            }

            for i in 0..TEST_COUNT {
                if i % 100 < 70 {
                    let key = index_to_key(i);
                    let item = pmap_get_bhdr(&raw const ht, key);
                    assert!(!item.is_null());
                    assert_eq!((*item).bh_bnum, key);

                    ht.remove(&key);
                    assert!(pmap_get_bhdr(&raw const ht, key).is_null());

                    pmap_put_bhdr(&raw mut ht, key, item);
                    assert_eq!(pmap_get_bhdr(&raw const ht, key), item);

                    ht.remove(&key);
                    assert!(pmap_get_bhdr(&raw const ht, key).is_null());

                    xfree(item.cast());
                }
            }

            for i in 0..TEST_COUNT {
                let key = index_to_key(i);
                let item = pmap_get_bhdr(&raw const ht, key);
                if i % 100 < 70 {
                    assert!(item.is_null());
                } else {
                    assert!(!item.is_null());
                    assert_eq!((*item).bh_bnum, key);
                    xfree(item.cast());
                }
            }
        }
    }
}
