//! Memory file block cache and swap file backing store (`src/nvim/memfile.c`).

use crate::buffer::{FileBuf, MF_DIRTY_YES, MF_DIRTY_YES_NOSYNC};
use crate::errors::*;
use crate::event::r#loop::loop_recursive_ptr;
use crate::fileio::{Off, SEEK_END, SEEK_SET, close, read_eintr, vim_lseek, write_eintr};
use crate::globals::*;
use crate::memline::{MFS_FLUSH, ml_get_buf, ml_open_file};

use crate::message::{emsg, iemsg, semsg};
use crate::os::fs::{
    FileInfo, O_CREAT, O_EXCL, O_NOFOLLOW, O_RDWR, O_TRUNC, os_fileinfo_blocksize, os_fileinfo_fd,
    os_fileinfo_link, os_fsync, os_open, os_remove, os_set_cloexec,
};
use crate::os::input::{os_breakcheck, os_char_avail};
use crate::path::FullName_save;
use crate::strings::{gettext, strerror};

use std::ptr;

pub type Blocknr = i64;
pub type Mfdirty = i32;

const MEMFILE_PAGE_SIZE: u32 = 4096;
pub(crate) const MIN_SWAP_PAGE_SIZE: u32 = 1048;
const MAX_SWAP_PAGE_SIZE: u64 = 50000;

pub(crate) const BH_DIRTY: u32 = 1;
const BH_LOCKED: u32 = 2;

const MF_DIRTY_NO: Mfdirty = 0;

pub(crate) const MFS_ALL: i32 = 1;
pub(crate) const MFS_STOP: i32 = 2;
pub(crate) const MFS_ZERO: i32 = 8;
#[cfg(windows)]
const S_IREAD: i32 = 0x0100;
#[cfg(windows)]
const S_IWRITE: i32 = 0x0080;
#[cfg(target_os = "linux")]
const S_IREAD: i32 = 0o400;
#[cfg(target_os = "linux")]
const S_IWRITE: i32 = 0o200;
#[cfg(not(any(windows, target_os = "linux")))]
const S_IREAD: i32 = 0o400;
#[cfg(not(any(windows, target_os = "linux")))]
const S_IWRITE: i32 = 0o200;

#[repr(C)]
pub struct Bhdr {
    pub bh_bnum: Blocknr,
    pub bh_data: Vec<u8>,
    pub bh_page_count: u32,
    pub bh_flags: u32,
}

impl Bhdr {
    #[inline]
    pub fn data_ptr(&mut self) -> *mut u8 {
        self.bh_data.as_mut_ptr()
    }
}

#[repr(C)]
pub struct Memfile {
    pub mf_fname: Option<std::ffi::CString>,
    pub mf_ffname: Option<std::ffi::CString>,
    pub mf_fd: i32,
    pub mf_flags: i32,
    pub mf_reopen: bool,
    pub mf_free_list: Vec<Bhdr>,
    pub mf_hash: std::collections::HashMap<i64, Box<Bhdr>>,
    pub mf_trans: std::collections::HashMap<i64, i64>,
    pub mf_blocknr_max: Blocknr,
    pub mf_blocknr_min: Blocknr,
    pub mf_neg_count: Blocknr,
    pub mf_infile_count: Blocknr,
    pub mf_page_size: u32,
    pub mf_dirty: Mfdirty,
}

#[inline]
fn perror_msg(msg: &std::ffi::CStr) {
    unsafe {
        let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        let _ = semsg!(c"%s: %s", gettext(msg.as_ptr()), strerror(err));
    }
}

#[inline]
pub fn pmap_get_bhdr(map: &std::collections::HashMap<i64, Box<Bhdr>>, key: i64) -> *mut Bhdr {
    map.get(&key)
        .map_or(ptr::null_mut(), |b| (&raw const **b).cast_mut())
}

/// Open a new or existing memory block file.
pub fn mf_open(fname: Option<std::ffi::CString>, flags: i32) -> Option<Box<Memfile>> {
    {
        let mut mfp = Box::new(Memfile {
            mf_fname: None,
            mf_ffname: None,
            mf_fd: -1,
            mf_flags: 0,
            mf_reopen: false,
            mf_free_list: Vec::new(),
            mf_hash: std::collections::HashMap::new(),
            mf_trans: std::collections::HashMap::new(),
            mf_blocknr_max: 0,
            mf_blocknr_min: -1,
            mf_neg_count: 0,
            mf_infile_count: 0,
            mf_page_size: MEMFILE_PAGE_SIZE,
            mf_dirty: MF_DIRTY_NO,
        });

        if let Some(fname) = fname
            && !mf_do_open(&mut mfp, fname, flags)
        {
            return None;
        }

        if mfp.mf_fd >= 0 {
            let mut file_info = FileInfo::new();
            if os_fileinfo_fd(mfp.mf_fd, &mut file_info) {
                let blocksize = os_fileinfo_blocksize(&file_info);
                if (u64::from(MIN_SWAP_PAGE_SIZE)..=MAX_SWAP_PAGE_SIZE).contains(&blocksize) {
                    mfp.mf_page_size = blocksize as u32;
                }
            }
        }

        let size: Off;
        if mfp.mf_fd < 0 || (flags & (O_TRUNC | O_EXCL)) != 0 || {
            size = unsafe { vim_lseek(mfp.mf_fd, 0, SEEK_END) };
            size <= 0
        } {
            mfp.mf_blocknr_max = 0;
        } else {
            assert!(mfp.mf_page_size > 0 && i64::from(mfp.mf_page_size - 1) <= i64::MAX - size);
            mfp.mf_blocknr_max =
                (size + i64::from(mfp.mf_page_size) - 1) / i64::from(mfp.mf_page_size);
        }
        mfp.mf_blocknr_min = -1;
        mfp.mf_neg_count = 0;
        mfp.mf_infile_count = mfp.mf_blocknr_max;

        Some(mfp)
    }
}

/// Open a file for an existing memfile.
pub fn mf_open_file(mfp: &mut Memfile, fname: std::ffi::CString) -> bool {
    if mf_do_open(mfp, fname, O_RDWR | O_CREAT | O_EXCL) {
        mfp.mf_dirty = MF_DIRTY_YES;
        true
    } else {
        false
    }
}

/// Close a memory file and optionally delete the associated file.
pub fn mf_close(mfp: Option<Box<Memfile>>, del_file: bool) {
    let Some(mfp) = mfp else {
        return;
    };
    {
        if mfp.mf_fd >= 0 && unsafe { close(mfp.mf_fd) } < 0 {
            emsg(gettext(E_SWAPCLOSE));
        }
        if del_file && let Some(ref fname) = mfp.mf_fname {
            os_remove(fname.as_ptr());
        }

        drop(mfp);
    }
}

/// Close the swap file for a memfile. Used when 'swapfile' is reset.
pub fn mf_close_file(buf: &mut FileBuf, getlines: bool) {
    unsafe {
        let mfp = buf.b_ml.mfp_ptr();
        if mfp.is_null() || (*mfp).mf_fd < 0 {
            return;
        }

        if getlines {
            for lnum in 1..=buf.b_ml.ml_line_count {
                ml_get_buf(buf, lnum);
            }
        }

        if close((*mfp).mf_fd) < 0 {
            emsg(gettext(E_SWAPCLOSE));
        }
        (*mfp).mf_fd = -1;

        if let Some(ref fname) = (*mfp).mf_fname {
            os_remove(fname.as_ptr());
            mf_free_fnames(&mut *mfp);
        }
    }
}

/// Set new size for a memfile.
pub fn mf_new_page_size(mfp: &mut Memfile, new_size: u32) {
    mfp.mf_page_size = new_size;
}

/// Get a new block.
pub fn mf_new(mfp: &mut Memfile, negative: bool, page_count: u32) -> *mut Bhdr {
    let bnum: Blocknr = if !negative
        && let Some(freep) = mfp.mf_free_list.last_mut()
        && freep.bh_page_count >= page_count
    {
        if freep.bh_page_count > page_count {
            let b = freep.bh_bnum;
            freep.bh_bnum += i64::from(page_count);
            freep.bh_page_count -= page_count;
            b
        } else {
            let free_blk = mfp.mf_free_list.pop().unwrap();
            free_blk.bh_bnum
        }
    } else if negative {
        let b = mfp.mf_blocknr_min;
        mfp.mf_blocknr_min -= 1;
        mfp.mf_neg_count += 1;
        b
    } else {
        let b = mfp.mf_blocknr_max;
        mfp.mf_blocknr_max += i64::from(page_count);
        b
    };

    let mut boxed = mf_alloc_bhdr(mfp, bnum, page_count, BH_LOCKED | BH_DIRTY);
    let hp: *mut Bhdr = &raw mut *boxed;
    mfp.mf_dirty = MF_DIRTY_YES;
    mfp.mf_hash.insert(bnum, boxed);
    unsafe { &mut *hp }
}

/// Get existing block `nr` with `page_count` pages.
pub fn mf_get(mfp: &mut Memfile, nr: Blocknr, page_count: u32) -> *mut Bhdr {
    {
        if nr >= mfp.mf_blocknr_max || nr <= mfp.mf_blocknr_min {
            return ptr::null_mut();
        }

        let mut hp = pmap_get_bhdr(&mfp.mf_hash, nr);
        if hp.is_null() {
            if nr < 0 || nr >= mfp.mf_infile_count || page_count == 0 {
                return ptr::null_mut();
            }

            let mut boxed = mf_alloc_bhdr(mfp, nr, page_count, BH_LOCKED);
            hp = &raw mut *boxed;
            if !mf_read(mfp, unsafe { &mut *hp }) {
                return ptr::null_mut();
            }
            mfp.mf_hash.insert(nr, boxed);
        } else {
            unsafe {
                (*hp).bh_flags |= BH_LOCKED;
            }
        }

        hp
    }
}

/// Release the block `*hp`.
pub fn mf_put(mfp: &mut Memfile, hp: *mut Bhdr, dirty: bool, infile: bool) {
    {
        let mut flags = unsafe { (*hp).bh_flags };

        if (flags & BH_LOCKED) == 0 {
            iemsg(gettext(c"E293: Block was not locked"));
        }
        flags &= !BH_LOCKED;
        if dirty {
            flags |= BH_DIRTY;
            if mfp.mf_dirty != MF_DIRTY_YES_NOSYNC {
                mfp.mf_dirty = MF_DIRTY_YES;
            }
        }
        unsafe {
            (*hp).bh_flags = flags;
        }
        if infile {
            let _ = mf_trans_add(mfp, unsafe { &mut *hp });
        }
    }
}

/// Signal block as no longer used (may put it in the free list).
pub fn mf_free(mfp: &mut Memfile, hp: &mut Bhdr) {
    let bnum = hp.bh_bnum;
    let page_count = hp.bh_page_count;
    let _ = mfp.mf_hash.remove(&bnum);
    if bnum < 0 {
        mfp.mf_neg_count -= 1;
    } else {
        mfp.mf_free_list.push(Bhdr {
            bh_bnum: bnum,
            bh_data: Vec::new(),
            bh_page_count: page_count,
            bh_flags: 0,
        });
    }
}

/// Sync memory file to disk.
pub fn mf_sync(mfp: &mut Memfile, flags: i32) -> bool {
    unsafe {
        let got_int_save = g_st().got_int;

        if mfp.mf_fd < 0 {
            mfp.mf_dirty = MF_DIRTY_NO;
            return false;
        }

        {
            g_st().got_int = false;
        }

        let mut status = true;
        let mut completed_all = true;
        let blocks: Vec<*mut Bhdr> = mfp
            .mf_hash
            .values()
            .map(|b| (&raw const **b).cast_mut())
            .collect();

        for hp in blocks {
            if ((flags & MFS_ALL) != 0 || (*hp).bh_bnum >= 0)
                && ((*hp).bh_flags & BH_DIRTY) != 0
                && (status || ((*hp).bh_bnum >= 0 && (*hp).bh_bnum < mfp.mf_infile_count))
            {
                if (flags & MFS_ZERO) != 0 && (*hp).bh_bnum != 0 {
                    continue;
                }
                if !mf_write(mfp, &mut *hp) {
                    if !status {
                        completed_all = false;
                        break;
                    }
                    status = false;
                }
                if (flags & MFS_STOP) != 0 {
                    if os_char_avail() {
                        completed_all = false;
                        break;
                    }
                } else {
                    if *loop_recursive_ptr(&mut g_st().main_loop) == 0 {
                        os_breakcheck();
                    }
                }
                if g_st().got_int {
                    completed_all = false;
                    break;
                }
            }
        }

        if completed_all || !status {
            mfp.mf_dirty = MF_DIRTY_NO;
        }

        if (flags & MFS_FLUSH) != 0 && os_fsync(mfp.mf_fd) != 0 {
            status = false;
        }

        {
            g_st().got_int |= got_int_save;
        }

        status
    }
}

/// Set dirty flag for all blocks in memory file with a positive block number.
pub fn mf_set_dirty(mfp: &mut Memfile) {
    for hp in mfp.mf_hash.values_mut() {
        if hp.bh_bnum > 0 {
            hp.bh_flags |= BH_DIRTY;
        }
    }
    mfp.mf_dirty = MF_DIRTY_YES;
}

/// Release as many blocks as possible.
pub fn mf_release_all() -> bool {
    let mut retval = false;
    unsafe {
        for buf in for_all_buffers_mut() {
            let mfp = buf.b_ml.mfp_ptr();
            if !mfp.is_null() {
                if (*mfp).mf_fd < 0 && buf.b_may_swap {
                    ml_open_file(buf);
                }

                if (*mfp).mf_fd >= 0 {
                    let blocks: Vec<*mut Bhdr> = (*mfp)
                        .mf_hash
                        .values()
                        .map(|b| (&raw const **b).cast_mut())
                        .collect();
                    for hp in blocks {
                        if ((*hp).bh_flags & BH_LOCKED) == 0
                            && (((*hp).bh_flags & BH_DIRTY) == 0 || mf_write(&mut *mfp, &mut *hp))
                        {
                            let _ = (*mfp).mf_hash.remove(&(*hp).bh_bnum);
                            retval = true;
                        }
                    }
                }
            }
        }
    }
    retval
}

fn mf_alloc_bhdr(mfp: &mut Memfile, bnum: Blocknr, page_count: u32, flags: u32) -> Box<Bhdr> {
    let len = (mfp.mf_page_size as usize) * (page_count as usize);
    Box::new(Bhdr {
        bh_bnum: bnum,
        bh_data: vec![0u8; len],
        bh_page_count: page_count,
        bh_flags: flags,
    })
}

fn mf_read(mfp: &mut Memfile, hp: &mut Bhdr) -> bool {
    {
        if mfp.mf_fd < 0 {
            return false;
        }

        let page_size = mfp.mf_page_size;
        let offset = i64::from(page_size).wrapping_mul(hp.bh_bnum);
        if unsafe { vim_lseek(mfp.mf_fd, offset, SEEK_SET) } != offset {
            perror_msg(c"E294: Seek error in swap file read");
            return false;
        }
        assert!(hp.bh_page_count <= u32::MAX / page_size);
        let size = (page_size * hp.bh_page_count) as usize;
        let nread = read_eintr(mfp.mf_fd, hp.data_ptr().cast(), size);
        if (nread as usize) != size {
            perror_msg(c"E295: Read error in swap file");
            return false;
        }

        true
    }
}

fn mf_write(mfp: &mut Memfile, hp: &mut Bhdr) -> bool {
    unsafe {
        if mfp.mf_fd < 0 && !mfp.mf_reopen {
            return false;
        }

        if hp.bh_bnum < 0 && !mf_trans_add(&mut *mfp, hp) {
            return false;
        }

        let page_size = mfp.mf_page_size;

        loop {
            let mut nr = hp.bh_bnum;
            let hp2: *mut Bhdr = if nr > mfp.mf_infile_count {
                nr = mfp.mf_infile_count;
                pmap_get_bhdr(&mfp.mf_hash, nr)
            } else {
                (&raw mut *hp)
            };

            let offset = i64::from(page_size).wrapping_mul(nr);
            let page_count = if hp2.is_null() {
                1
            } else {
                (*hp2).bh_page_count
            };
            let size = (page_size * page_count) as usize;

            for attempt in 1..=2 {
                if mfp.mf_fd >= 0 {
                    if vim_lseek(mfp.mf_fd, offset, SEEK_SET) != offset {
                        perror_msg(c"E296: Seek error in swap file write");
                        return false;
                    }
                    let data = if hp2.is_null() {
                        hp.data_ptr()
                    } else {
                        (*hp2).data_ptr()
                    };
                    let nwritten = write_eintr(mfp.mf_fd, data.cast(), size);
                    if (nwritten as usize) == size {
                        break;
                    }
                }

                if attempt == 1 {
                    if mfp.mf_fd >= 0 {
                        let _ = close(mfp.mf_fd);
                    }
                    {
                        mfp.mf_fd = os_open(
                            mfp.mf_fname.as_ref().unwrap().as_ptr(),
                            mfp.mf_flags,
                            S_IREAD | S_IWRITE,
                        );
                    }
                    mfp.mf_reopen = mfp.mf_fd < 0;
                }
                if attempt == 2 || mfp.mf_fd < 0 {
                    {
                        if !g_st().did_swapwrite_msg {
                            emsg(gettext(c"E297: Write error in swap file"));
                        }
                        g_st().did_swapwrite_msg = true;
                    }
                    return false;
                }
            }

            {
                g_st().did_swapwrite_msg = false;
            }
            if !hp2.is_null() {
                (*hp2).bh_flags &= !BH_DIRTY;
            }
            if nr + i64::from(page_count) > mfp.mf_infile_count {
                mfp.mf_infile_count = nr + i64::from(page_count);
            }
            if nr == hp.bh_bnum {
                break;
            }
        }
        true
    }
}

fn mf_trans_add(mfp: &mut Memfile, hp: &mut Bhdr) -> bool {
    {
        if hp.bh_bnum >= 0 {
            return true;
        }

        let page_count = hp.bh_page_count;
        let new_bnum: Blocknr = if let Some(freep) = mfp.mf_free_list.last_mut()
            && freep.bh_page_count >= page_count
        {
            let bnum = freep.bh_bnum;
            if freep.bh_page_count > page_count {
                freep.bh_bnum += i64::from(page_count);
                freep.bh_page_count -= page_count;
            } else {
                let _ = mfp.mf_free_list.pop();
            }
            bnum
        } else {
            let bnum = mfp.mf_blocknr_max;
            mfp.mf_blocknr_max += i64::from(page_count);
            bnum
        };

        let old_bnum = hp.bh_bnum;
        if let Some(mut boxed) = mfp.mf_hash.remove(&old_bnum) {
            boxed.bh_bnum = new_bnum;
            mfp.mf_hash.insert(new_bnum, boxed);
        }

        mfp.mf_trans.insert(old_bnum, new_bnum);

        true
    }
}

/// Lookup translation from trans list and delete the entry.
pub fn mf_trans_del(mfp: &mut Memfile, old_nr: Blocknr) -> Blocknr {
    let Some(new_bnum) = mfp.mf_trans.remove(&old_nr) else {
        return old_nr;
    };

    mfp.mf_neg_count -= 1;

    new_bnum
}

/// Frees `mf_fname` and `mf_ffname`.
pub fn mf_free_fnames(mfp: &mut Memfile) {
    mfp.mf_fname = None;
    mfp.mf_ffname = None;
}

/// Set the simple file name and the full file name of memfile's swapfile.
pub fn mf_set_fnames(mfp: &mut Memfile, fname: std::ffi::CString) {
    mfp.mf_ffname = FullName_save(fname.as_ptr(), false);
    mfp.mf_fname = Some(fname);
}

/// Make name of memfile's swapfile a full path.
pub fn mf_fullname(mut mfp: Option<&mut Memfile>) {
    if mfp.is_none() {
        return;
    }
    {
        if mfp.as_deref_mut().unwrap().mf_fname.is_none()
            || mfp.as_deref_mut().unwrap().mf_ffname.is_none()
        {
            return;
        }

        mfp.unwrap().mf_fname = mfp.as_deref_mut().unwrap().mf_ffname.take();
    }
}

/// Return true if there are any translations pending for memfile.
pub fn mf_need_trans(mfp: &mut Memfile) -> bool {
    mfp.mf_fname.is_some() && mfp.mf_neg_count > 0
}

fn mf_do_open(mfp: &mut Memfile, fname: std::ffi::CString, mut flags: i32) -> bool {
    mf_set_fnames(mfp, fname);
    let fname_ptr = mfp.mf_fname.as_ref().unwrap().as_ptr();

    {
        let mut file_info = FileInfo::new();
        if (flags & O_CREAT) != 0 && os_fileinfo_link(fname_ptr, &mut file_info) {
            mfp.mf_fd = -1;
            emsg(gettext(c"E300: Swap file already exists (symlink attack?)"));
        } else {
            flags |= O_NOFOLLOW;
            mfp.mf_flags = flags;
            mfp.mf_fd = os_open(fname_ptr, flags, S_IREAD | S_IWRITE);
        }
    }

    if mfp.mf_fd < 0 {
        mf_free_fnames(mfp);
        return false;
    }

    os_set_cloexec(mfp.mf_fd);

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_only_memfile_allocates_gets_puts_and_frees_blocks() {
        {
            let mut mfp_box = mf_open(None, 0).expect("mf_open failed");
            let mfp = &mut *mfp_box;
            assert_eq!(mfp.mf_page_size, 4096);

            let hp1 = mf_new(mfp, true, 1);
            assert!(!hp1.is_null());
            assert_eq!(unsafe { (*hp1).bh_bnum }, -1);
            mf_put(&mut *mfp, hp1, true, true);
            assert_eq!(mf_trans_del(mfp, -1), 0);

            let hp2 = mf_get(mfp, 0, 1);
            assert_eq!(hp2, hp1);
            mf_put(&mut *mfp, hp2, false, false);
            mf_free(mfp, unsafe { &mut *hp2 });

            mf_close(Some(mfp_box), false);
        }
    }

    #[test]
    fn mf_hash_inserts_expands_removes_and_frees_50000_items() {
        const TEST_COUNT: usize = 50_000;
        const fn index_to_key(i: usize) -> Blocknr {
            (i as Blocknr) ^ 15167
        }

        {
            let mut ht: std::collections::HashMap<i64, Box<Bhdr>> =
                std::collections::HashMap::new();
            let mut items: Vec<*mut Bhdr> = Vec::with_capacity(TEST_COUNT);

            for i in 0..TEST_COUNT {
                assert_eq!(ht.len(), i);
                let key = index_to_key(i);
                assert!(pmap_get_bhdr(&ht, key).is_null());

                let mut boxed = Box::new(Bhdr {
                    bh_bnum: key,
                    bh_data: Vec::new(),
                    bh_page_count: 1,
                    bh_flags: 0,
                });
                let item: *mut Bhdr = &raw mut *boxed;
                ht.insert(key, boxed);
                assert_eq!(pmap_get_bhdr(&ht, key), item);
                items.push(item);
            }

            for i in 0..TEST_COUNT {
                let key = index_to_key(i);
                let item = pmap_get_bhdr(&ht, key);
                assert!(!item.is_null());
                assert_eq!(unsafe { (*item).bh_bnum }, key);
            }

            for i in 0..TEST_COUNT {
                if i % 100 < 70 {
                    let key = index_to_key(i);
                    let item = pmap_get_bhdr(&ht, key);
                    assert!(!item.is_null());
                    assert_eq!(unsafe { (*item).bh_bnum }, key);

                    let removed = ht.remove(&key).unwrap();
                    assert!(pmap_get_bhdr(&ht, key).is_null());

                    ht.insert(key, removed);
                    assert_eq!(pmap_get_bhdr(&ht, key), item);

                    ht.remove(&key);
                    assert!(pmap_get_bhdr(&ht, key).is_null());
                }
            }

            for i in 0..TEST_COUNT {
                let key = index_to_key(i);
                let item = pmap_get_bhdr(&ht, key);
                if i % 100 < 70 {
                    assert!(item.is_null());
                } else {
                    assert!(!item.is_null());
                    assert_eq!(unsafe { (*item).bh_bnum }, key);
                }
            }
        }
    }
}
