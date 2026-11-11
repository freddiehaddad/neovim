#![forbid(unsafe_code)]

//! Memory file block cache and swap file backing store (`src/nvim/memfile.c`).

use crate::buffer::{FileBuf, MF_DIRTY_YES, MF_DIRTY_YES_NOSYNC};
use crate::errors::*;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BhdrId(usize);

#[repr(C)]
pub struct Bhdr {
    pub bh_bnum: Blocknr,
    pub bh_data: Vec<u8>,
    pub bh_page_count: u32,
    pub bh_flags: u32,
}

impl Bhdr {
    #[inline]
    pub fn data_slice_mut(&mut self) -> &mut [u8] {
        &mut self.bh_data
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
    pub mf_blocks: Vec<Option<Bhdr>>,
    pub mf_free_slots: Vec<usize>,
    pub mf_hash: std::collections::HashMap<i64, BhdrId>,
    pub mf_trans: std::collections::HashMap<i64, i64>,
    pub mf_blocknr_max: Blocknr,
    pub mf_blocknr_min: Blocknr,
    pub mf_neg_count: Blocknr,
    pub mf_infile_count: Blocknr,
    pub mf_page_size: u32,
    pub mf_dirty: Mfdirty,
}

impl Memfile {
    #[inline(always)]
    pub fn bhdr(&self, id: BhdrId) -> &Bhdr {
        self.mf_blocks[id.0].as_ref().expect("invalid BhdrId")
    }

    #[inline(always)]
    pub fn bhdr_mut(&mut self, id: BhdrId) -> &mut Bhdr {
        self.mf_blocks[id.0].as_mut().expect("invalid BhdrId")
    }

    #[inline(always)]
    pub fn find_bhdr(&self, bnum: Blocknr) -> Option<BhdrId> {
        self.mf_hash.get(&bnum).copied()
    }

    pub fn insert_bhdr(&mut self, bhdr: Bhdr) -> BhdrId {
        let bnum = bhdr.bh_bnum;
        let id = if let Some(slot) = self.mf_free_slots.pop() {
            self.mf_blocks[slot] = Some(bhdr);
            BhdrId(slot)
        } else {
            let slot = self.mf_blocks.len();
            self.mf_blocks.push(Some(bhdr));
            BhdrId(slot)
        };
        self.mf_hash.insert(bnum, id);
        id
    }

    pub fn remove_bhdr(&mut self, id: BhdrId) -> Option<Bhdr> {
        let bhdr = self.mf_blocks[id.0].take()?;
        self.mf_hash.remove(&bhdr.bh_bnum);
        self.mf_free_slots.push(id.0);
        Some(bhdr)
    }

    pub fn copy_bhdr_data(
        &mut self,
        src_id: BhdrId,
        src_start: usize,
        dst_id: BhdrId,
        dst_start: usize,
        len: usize,
    ) {
        if src_id == dst_id {
            self.bhdr_mut(src_id)
                .bh_data
                .copy_within(src_start..src_start + len, dst_start);
        } else if src_id.0 < dst_id.0 {
            let (left, right) = self.mf_blocks.split_at_mut(dst_id.0);
            let src = &left[src_id.0].as_ref().unwrap().bh_data[src_start..src_start + len];
            let dst = &mut right[0].as_mut().unwrap().bh_data[dst_start..dst_start + len];
            dst.copy_from_slice(src);
        } else {
            let (left, right) = self.mf_blocks.split_at_mut(src_id.0);
            let src = &right[0].as_ref().unwrap().bh_data[src_start..src_start + len];
            let dst = &mut left[dst_id.0].as_mut().unwrap().bh_data[dst_start..dst_start + len];
            dst.copy_from_slice(src);
        }
    }
}

#[inline]
fn perror_msg(msg: &std::ffi::CStr) {
    let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    let _ = semsg!(c"%s: %s", gettext(msg), strerror(err));
}

/// Open a new or existing memory block file.
pub fn mf_open(fname: Option<std::ffi::CString>, flags: i32) -> Option<Box<Memfile>> {
    let mut mfp = Box::new(Memfile {
        mf_fname: None,
        mf_ffname: None,
        mf_fd: -1,
        mf_flags: 0,
        mf_reopen: false,
        mf_free_list: Vec::new(),
        mf_blocks: Vec::new(),
        mf_free_slots: Vec::new(),
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
        size = vim_lseek(mfp.mf_fd, 0, SEEK_END);
        size <= 0
    } {
        mfp.mf_blocknr_max = 0;
    } else {
        assert!(mfp.mf_page_size > 0 && i64::from(mfp.mf_page_size - 1) <= i64::MAX - size);
        mfp.mf_blocknr_max = (size + i64::from(mfp.mf_page_size) - 1) / i64::from(mfp.mf_page_size);
    }
    mfp.mf_blocknr_min = -1;
    mfp.mf_neg_count = 0;
    mfp.mf_infile_count = mfp.mf_blocknr_max;

    Some(mfp)
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
        if mfp.mf_fd >= 0 && close(mfp.mf_fd) < 0 {
            emsg(gettext(E_SWAPCLOSE));
        }
        if del_file && let Some(ref fname) = mfp.mf_fname {
            os_remove(fname);
        }

        drop(mfp);
    }
}

/// Close the swap file for a memfile. Used when 'swapfile' is reset.
pub fn mf_close_file(buf: &mut FileBuf, getlines: bool) {
    if buf.b_ml.mfp_opt().is_none_or(|m| m.mf_fd < 0) {
        return;
    }

    if getlines {
        for lnum in 1..=buf.b_ml.ml_line_count.get() {
            let _ = ml_get_buf(buf, lnum);
        }
    }

    let Some(mut mfp_ref) = buf.b_ml.mfp_opt() else {
        return;
    };
    if close(mfp_ref.mf_fd) < 0 {
        emsg(gettext(E_SWAPCLOSE));
    }

    mfp_ref.mf_fd = -1;

    if let Some(fname) = mfp_ref.mf_fname.clone() {
        os_remove(fname.as_c_str());

        mf_free_fnames(&mut mfp_ref);
    }
}

/// Set new size for a memfile.
pub fn mf_new_page_size(mfp: &mut Memfile, new_size: u32) {
    mfp.mf_page_size = new_size;
}

/// Get a new block.
pub fn mf_new(mfp: &mut Memfile, negative: bool, page_count: u32) -> BhdrId {
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

    let bhdr = mf_alloc_bhdr(mfp, bnum, page_count, BH_LOCKED | BH_DIRTY);
    mfp.mf_dirty = MF_DIRTY_YES;
    mfp.insert_bhdr(bhdr)
}

/// Get existing block `nr` with `page_count` pages.
pub fn mf_get(mfp: &mut Memfile, nr: Blocknr, page_count: u32) -> Option<BhdrId> {
    if nr >= mfp.mf_blocknr_max || nr <= mfp.mf_blocknr_min {
        return None;
    }

    if let Some(hp_id) = mfp.find_bhdr(nr) {
        mfp.bhdr_mut(hp_id).bh_flags |= BH_LOCKED;
        return Some(hp_id);
    }

    if nr < 0 || nr >= mfp.mf_infile_count || page_count == 0 {
        return None;
    }

    let mut bhdr = mf_alloc_bhdr(mfp, nr, page_count, BH_LOCKED);
    if !mf_read(mfp, &mut bhdr) {
        return None;
    }
    Some(mfp.insert_bhdr(bhdr))
}

/// Release the block `hp_id`.
pub fn mf_put(mfp: &mut Memfile, hp_id: BhdrId, dirty: bool, infile: bool) {
    let mut flags = mfp.bhdr(hp_id).bh_flags;

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
    mfp.bhdr_mut(hp_id).bh_flags = flags;
    if infile {
        let _ = mf_trans_add(mfp, hp_id);
    }
}

/// Signal block as no longer used (may put it in the free list).
pub fn mf_free(mfp: &mut Memfile, hp_id: BhdrId) {
    let Some(bhdr) = mfp.remove_bhdr(hp_id) else {
        return;
    };
    let bnum = bhdr.bh_bnum;
    let page_count = bhdr.bh_page_count;
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
    let got_int_save = g_st().got_int.get();

    if mfp.mf_fd < 0 {
        mfp.mf_dirty = MF_DIRTY_NO;
        return false;
    }

    {
        g_st().got_int.set(false);
    }

    let mut status = true;
    let mut completed_all = true;
    let blocks: Vec<BhdrId> = mfp.mf_hash.values().copied().collect();

    for hp_id in blocks {
        let (bh_bnum, bh_flags) = {
            let hp = mfp.bhdr(hp_id);
            (hp.bh_bnum, hp.bh_flags)
        };
        if ((flags & MFS_ALL) != 0 || bh_bnum >= 0)
            && (bh_flags & BH_DIRTY) != 0
            && (status || (bh_bnum >= 0 && bh_bnum < mfp.mf_infile_count))
        {
            if (flags & MFS_ZERO) != 0 && bh_bnum != 0 {
                continue;
            }
            if !mf_write(mfp, hp_id) {
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
            } else if crate::event::r#loop::main_loop_ptr().recursive == 0 {
                os_breakcheck();
            }
            if g_st().got_int.get() {
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
        g_st().got_int.set(g_st().got_int.get() | (got_int_save));
    }

    status
}

/// Set dirty flag for all blocks in memory file with a positive block number.
pub fn mf_set_dirty(mfp: &mut Memfile) {
    for hp in mfp.mf_blocks.iter_mut().flatten() {
        if hp.bh_bnum > 0 {
            hp.bh_flags |= BH_DIRTY;
        }
    }
    mfp.mf_dirty = MF_DIRTY_YES;
}

/// Release as many blocks as possible.
pub fn mf_release_all() -> bool {
    let mut retval = false;

    for buf in for_all_buffers_mut() {
        if buf.b_ml.mfp_opt().is_some_and(|mfp| mfp.mf_fd < 0) && buf.b_may_swap.get() {
            ml_open_file(buf);
        }
        if let Some(mut mfp_ref) = buf.b_ml.mfp_opt()
            && mfp_ref.mf_fd >= 0
        {
            let blocks: Vec<BhdrId> = mfp_ref.mf_hash.values().copied().collect();
            for hp_id in blocks {
                let bh_flags = mfp_ref.bhdr(hp_id).bh_flags;
                if (bh_flags & BH_LOCKED) == 0
                    && ((bh_flags & BH_DIRTY) == 0 || mf_write(&mut mfp_ref, hp_id))
                {
                    let _ = mfp_ref.remove_bhdr(hp_id);
                    retval = true;
                }
            }
        }
    }

    retval
}

fn mf_alloc_bhdr(mfp: &mut Memfile, bnum: Blocknr, page_count: u32, flags: u32) -> Bhdr {
    let len = (mfp.mf_page_size as usize) * (page_count as usize);
    Bhdr {
        bh_bnum: bnum,
        bh_data: vec![0u8; len],
        bh_page_count: page_count,
        bh_flags: flags,
    }
}

fn mf_read(mfp: &mut Memfile, hp: &mut Bhdr) -> bool {
    if mfp.mf_fd < 0 {
        return false;
    }

    let page_size = mfp.mf_page_size;
    let offset = i64::from(page_size).wrapping_mul(hp.bh_bnum);
    if vim_lseek(mfp.mf_fd, offset, SEEK_SET) != offset {
        perror_msg(c"E294: Seek error in swap file read");
        return false;
    }
    assert!(hp.bh_page_count <= u32::MAX / page_size);
    let size = (page_size * hp.bh_page_count) as usize;
    let nread = read_eintr(mfp.mf_fd, &mut hp.data_slice_mut()[..size]);
    if (nread as usize) != size {
        perror_msg(c"E295: Read error in swap file");
        return false;
    }

    true
}

fn mf_write(mfp: &mut Memfile, hp_id: BhdrId) -> bool {
    if mfp.mf_fd < 0 && !mfp.mf_reopen {
        return false;
    }

    if mfp.bhdr(hp_id).bh_bnum < 0 && !mf_trans_add(mfp, hp_id) {
        return false;
    }

    let page_size = mfp.mf_page_size;

    loop {
        let target_bnum = mfp.bhdr(hp_id).bh_bnum;
        let (nr, hp2_id) = if target_bnum > mfp.mf_infile_count {
            let nr = mfp.mf_infile_count;
            (nr, mfp.find_bhdr(nr))
        } else {
            (target_bnum, Some(hp_id))
        };

        let offset = i64::from(page_size).wrapping_mul(nr);
        let page_count = hp2_id.map_or(1, |id| mfp.bhdr(id).bh_page_count);
        let size = (page_size * page_count) as usize;

        for attempt in 1..=2 {
            if mfp.mf_fd >= 0 {
                if vim_lseek(mfp.mf_fd, offset, SEEK_SET) != offset {
                    perror_msg(c"E296: Seek error in swap file write");
                    return false;
                }
                let data_id = hp2_id.unwrap_or(hp_id);
                let data = &mfp.bhdr(data_id).bh_data[..size];
                let nwritten = write_eintr(mfp.mf_fd, data);
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
                        mfp.mf_fname.as_ref().unwrap(),
                        mfp.mf_flags,
                        S_IREAD | S_IWRITE,
                    );
                }
                mfp.mf_reopen = mfp.mf_fd < 0;
            }
            if attempt == 2 || mfp.mf_fd < 0 {
                {
                    if !g_st().did_swapwrite_msg.get() {
                        emsg(gettext(c"E297: Write error in swap file"));
                    }
                    g_st().did_swapwrite_msg.set(true);
                }
                return false;
            }
        }

        {
            g_st().did_swapwrite_msg.set(false);
        }
        if let Some(hp2_id) = hp2_id {
            mfp.bhdr_mut(hp2_id).bh_flags &= !BH_DIRTY;
        }
        if nr + i64::from(page_count) > mfp.mf_infile_count {
            mfp.mf_infile_count = nr + i64::from(page_count);
        }
        if nr == target_bnum {
            break;
        }
    }
    true
}

fn mf_trans_add(mfp: &mut Memfile, hp_id: BhdrId) -> bool {
    let old_bnum = mfp.bhdr(hp_id).bh_bnum;
    if old_bnum >= 0 {
        return true;
    }

    let page_count = mfp.bhdr(hp_id).bh_page_count;
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

    mfp.mf_hash.remove(&old_bnum);
    mfp.bhdr_mut(hp_id).bh_bnum = new_bnum;
    mfp.mf_hash.insert(new_bnum, hp_id);

    mfp.mf_trans.insert(old_bnum, new_bnum);

    true
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
    mfp.mf_ffname = FullName_save(Some(&fname), false);
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
pub fn mf_need_trans(mfp: &Memfile) -> bool {
    mfp.mf_fname.is_some() && mfp.mf_neg_count > 0
}

fn mf_do_open(mfp: &mut Memfile, fname: std::ffi::CString, mut flags: i32) -> bool {
    mf_set_fnames(mfp, fname);
    let fname_cstr = mfp.mf_fname.clone().unwrap();

    {
        let mut file_info = FileInfo::new();
        if (flags & O_CREAT) != 0 && os_fileinfo_link(&fname_cstr, &mut file_info) {
            mfp.mf_fd = -1;
            emsg(gettext(c"E300: Swap file already exists (symlink attack?)"));
        } else {
            flags |= O_NOFOLLOW;
            mfp.mf_flags = flags;
            mfp.mf_fd = os_open(&fname_cstr, flags, S_IREAD | S_IWRITE);
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
        let mut mfp_box = mf_open(None, 0).expect("mf_open failed");
        let mfp = &mut *mfp_box;
        assert_eq!(mfp.mf_page_size, 4096);

        let hp1 = mf_new(mfp, true, 1);
        assert_eq!(mfp.bhdr(hp1).bh_bnum, -1);
        mf_put(&mut *mfp, hp1, true, true);
        assert_eq!(mf_trans_del(mfp, -1), 0);

        let hp2 = mf_get(mfp, 0, 1).expect("mf_get failed");
        assert_eq!(hp1, hp2);
        mf_put(&mut *mfp, hp2, false, false);
        mf_free(mfp, hp2);

        mf_close(Some(mfp_box), false);
    }

    #[test]
    fn mf_hash_inserts_expands_removes_and_frees_50000_items() {
        const TEST_COUNT: usize = 50_000;
        const fn index_to_key(i: usize) -> Blocknr {
            (i as Blocknr) ^ 15167
        }

        let mut mfp = mf_open(None, 0).expect("mf_open failed");

        for i in 0..TEST_COUNT {
            assert_eq!(mfp.mf_hash.len(), i);
            let key = index_to_key(i);
            assert!(mfp.find_bhdr(key).is_none());

            let bhdr = Bhdr {
                bh_bnum: key,
                bh_data: Vec::new(),
                bh_page_count: 1,
                bh_flags: 0,
            };
            let id = mfp.insert_bhdr(bhdr);
            assert_eq!(Some(id), mfp.find_bhdr(key));
        }

        for i in 0..TEST_COUNT {
            let key = index_to_key(i);
            let id = mfp.find_bhdr(key).unwrap();
            assert_eq!(mfp.bhdr(id).bh_bnum, key);
        }

        for i in 0..TEST_COUNT {
            if i % 100 < 70 {
                let key = index_to_key(i);
                let id = mfp.find_bhdr(key).unwrap();
                assert_eq!(mfp.bhdr(id).bh_bnum, key);

                let removed = mfp.remove_bhdr(id).unwrap();
                assert!(mfp.find_bhdr(key).is_none());

                let new_id = mfp.insert_bhdr(removed);
                assert_eq!(id, new_id);
                assert_eq!(Some(id), mfp.find_bhdr(key));

                let _ = mfp.remove_bhdr(id);
                assert!(mfp.find_bhdr(key).is_none());
            }
        }

        for i in 0..TEST_COUNT {
            let key = index_to_key(i);
            if i % 100 < 70 {
                assert!(mfp.find_bhdr(key).is_none());
            } else {
                let id = mfp.find_bhdr(key).unwrap();
                assert_eq!(mfp.bhdr(id).bh_bnum, key);
            }
        }
    }
}
