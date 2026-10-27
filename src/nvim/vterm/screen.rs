//! VTerm screen buffer and damage management (`src/nvim/vterm/screen.c`).

use std::ffi::{c_char, c_int};
use std::ptr;

use crate::terminal::Terminal;
use crate::vterm::keyboard::{BUFIDX_ALTSCREEN, BUFIDX_PRIMARY, VTermColor, VTermStateFull};
use crate::vterm::mouse::{VTerm, VTermPos};
use crate::vterm::pen::{
    VTERM_ATTR_BACKGROUND, VTERM_ATTR_BASELINE, VTERM_ATTR_BLINK, VTERM_ATTR_BOLD,
    VTERM_ATTR_CONCEAL, VTERM_ATTR_DIM, VTERM_ATTR_FONT, VTERM_ATTR_FOREGROUND, VTERM_ATTR_ITALIC,
    VTERM_ATTR_OVERLINE, VTERM_ATTR_REVERSE, VTERM_ATTR_SMALL, VTERM_ATTR_STRIKE,
    VTERM_ATTR_UNDERLINE, VTERM_ATTR_URI, VTermAttr, VTermProp, VTermRect, VTermStateCallbacks,
    VTermValue,
};
use crate::vterm::state::{
    VTermStateFallbacks, vterm_obtain_state, vterm_state_get_lineinfo, vterm_state_reset,
    vterm_state_set_callbacks, vterm_state_set_unrecognised_fallbacks,
};
use crate::vterm::vterm::{vterm_get_size, vterm_scroll_rect};

use crate::grid::schar_T;
pub type VTermDamageSize = c_int;

const VTERM_DAMAGE_CELL: VTermDamageSize = 0;
const VTERM_DAMAGE_ROW: VTermDamageSize = 1;
const VTERM_DAMAGE_SCREEN: VTermDamageSize = 2;
pub(crate) const VTERM_DAMAGE_SCROLL: VTermDamageSize = 3;

pub(crate) const VTERM_PROP_ALTSCREEN: VTermProp = 3;
pub(crate) const VTERM_PROP_REVERSE: VTermProp = 6;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VTermScreenCellAttrs {
    pub flags: u32,
}

impl VTermScreenCellAttrs {
    #[inline]
    pub const fn bold(&self) -> u32 {
        self.flags & 0x01
    }
    #[inline]
    pub const fn set_bold(&mut self, val: u32) {
        self.flags = (self.flags & !0x01) | (val & 0x01);
    }
    #[inline]
    pub const fn underline(&self) -> u32 {
        (self.flags >> 1) & 0x03
    }
    #[inline]
    pub const fn set_underline(&mut self, val: u32) {
        self.flags = (self.flags & !(0x03 << 1)) | ((val & 0x03) << 1);
    }
    #[inline]
    pub const fn italic(&self) -> u32 {
        (self.flags >> 3) & 0x01
    }
    #[inline]
    pub const fn set_italic(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 3)) | ((val & 0x01) << 3);
    }
    #[inline]
    pub const fn blink(&self) -> u32 {
        (self.flags >> 4) & 0x01
    }
    #[inline]
    pub const fn set_blink(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 4)) | ((val & 0x01) << 4);
    }
    #[inline]
    pub const fn reverse(&self) -> u32 {
        (self.flags >> 5) & 0x01
    }
    #[inline]
    pub const fn set_reverse(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 5)) | ((val & 0x01) << 5);
    }
    #[inline]
    pub const fn conceal(&self) -> u32 {
        (self.flags >> 6) & 0x01
    }
    #[inline]
    pub const fn set_conceal(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 6)) | ((val & 0x01) << 6);
    }
    #[inline]
    pub const fn strike(&self) -> u32 {
        (self.flags >> 7) & 0x01
    }
    #[inline]
    pub const fn set_strike(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 7)) | ((val & 0x01) << 7);
    }
    #[inline]
    pub const fn font(&self) -> u32 {
        (self.flags >> 8) & 0x0f
    }
    #[inline]
    pub const fn set_font(&mut self, val: u32) {
        self.flags = (self.flags & !(0x0f << 8)) | ((val & 0x0f) << 8);
    }
    #[inline]
    pub const fn set_dwl(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 12)) | ((val & 0x01) << 12);
    }
    #[inline]
    pub const fn set_dhl(&mut self, val: u32) {
        self.flags = (self.flags & !(0x03 << 13)) | ((val & 0x03) << 13);
    }
    #[inline]
    pub const fn small(&self) -> u32 {
        (self.flags >> 15) & 0x01
    }
    #[inline]
    pub const fn set_small(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 15)) | ((val & 0x01) << 15);
    }
    #[inline]
    pub const fn baseline(&self) -> u32 {
        (self.flags >> 16) & 0x03
    }
    #[inline]
    pub const fn set_baseline(&mut self, val: u32) {
        self.flags = (self.flags & !(0x03 << 16)) | ((val & 0x03) << 16);
    }
    #[inline]
    pub const fn dim(&self) -> u32 {
        (self.flags >> 18) & 0x01
    }
    #[inline]
    pub const fn set_dim(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 18)) | ((val & 0x01) << 18);
    }
    #[inline]
    pub const fn overline(&self) -> u32 {
        (self.flags >> 19) & 0x01
    }
    #[inline]
    pub const fn set_overline(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 19)) | ((val & 0x01) << 19);
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VTermScreenCell {
    pub schar: schar_T,
    pub width: c_char,
    pub attrs: VTermScreenCellAttrs,
    pub fg: VTermColor,
    pub bg: VTermColor,
    pub uri: c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ScreenPen {
    pub fg: VTermColor,
    pub bg: VTermColor,
    pub uri: c_int,
    pub flags: u32,
}

impl ScreenPen {
    #[inline]
    pub const fn bold(&self) -> u32 {
        self.flags & 0x01
    }
    #[inline]
    pub const fn set_bold(&mut self, val: u32) {
        self.flags = (self.flags & !0x01) | (val & 0x01);
    }
    #[inline]
    pub const fn underline(&self) -> u32 {
        (self.flags >> 1) & 0x03
    }
    #[inline]
    pub const fn set_underline(&mut self, val: u32) {
        self.flags = (self.flags & !(0x03 << 1)) | ((val & 0x03) << 1);
    }
    #[inline]
    pub const fn italic(&self) -> u32 {
        (self.flags >> 3) & 0x01
    }
    #[inline]
    pub const fn set_italic(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 3)) | ((val & 0x01) << 3);
    }
    #[inline]
    pub const fn blink(&self) -> u32 {
        (self.flags >> 4) & 0x01
    }
    #[inline]
    pub const fn set_blink(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 4)) | ((val & 0x01) << 4);
    }
    #[inline]
    pub const fn reverse(&self) -> u32 {
        (self.flags >> 5) & 0x01
    }
    #[inline]
    pub const fn set_reverse(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 5)) | ((val & 0x01) << 5);
    }
    #[inline]
    pub const fn conceal(&self) -> u32 {
        (self.flags >> 6) & 0x01
    }
    #[inline]
    pub const fn set_conceal(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 6)) | ((val & 0x01) << 6);
    }
    #[inline]
    pub const fn strike(&self) -> u32 {
        (self.flags >> 7) & 0x01
    }
    #[inline]
    pub const fn set_strike(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 7)) | ((val & 0x01) << 7);
    }
    #[inline]
    pub const fn font(&self) -> u32 {
        (self.flags >> 8) & 0x0f
    }
    #[inline]
    pub const fn set_font(&mut self, val: u32) {
        self.flags = (self.flags & !(0x0f << 8)) | ((val & 0x0f) << 8);
    }
    #[inline]
    pub const fn small(&self) -> u32 {
        (self.flags >> 12) & 0x01
    }
    #[inline]
    pub const fn set_small(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 12)) | ((val & 0x01) << 12);
    }
    #[inline]
    pub const fn baseline(&self) -> u32 {
        (self.flags >> 13) & 0x03
    }
    #[inline]
    pub const fn set_baseline(&mut self, val: u32) {
        self.flags = (self.flags & !(0x03 << 13)) | ((val & 0x03) << 13);
    }
    #[inline]
    pub const fn dim(&self) -> u32 {
        (self.flags >> 15) & 0x01
    }
    #[inline]
    pub const fn set_dim(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 15)) | ((val & 0x01) << 15);
    }
    #[inline]
    pub const fn overline(&self) -> u32 {
        (self.flags >> 16) & 0x01
    }
    #[inline]
    pub const fn set_overline(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 16)) | ((val & 0x01) << 16);
    }
    #[inline]
    pub const fn protected_cell(&self) -> u32 {
        (self.flags >> 17) & 0x01
    }
    #[inline]
    pub const fn set_protected_cell(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 17)) | ((val & 0x01) << 17);
    }
    #[inline]
    pub const fn dwl(&self) -> u32 {
        (self.flags >> 18) & 0x01
    }
    #[inline]
    pub const fn set_dwl(&mut self, val: u32) {
        self.flags = (self.flags & !(0x01 << 18)) | ((val & 0x01) << 18);
    }
    #[inline]
    pub const fn dhl(&self) -> u32 {
        (self.flags >> 19) & 0x03
    }
    #[inline]
    pub const fn set_dhl(&mut self, val: u32) {
        self.flags = (self.flags & !(0x03 << 19)) | ((val & 0x03) << 19);
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ScreenCell {
    pub schar: schar_T,
    pub pen: ScreenPen,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VTermGlyphInfo {
    pub schar: schar_T,
    pub width: c_int,
    pub flags: u32,
}

impl VTermGlyphInfo {
    #[inline]
    pub const fn protected_cell(&self) -> u32 {
        self.flags & 0x01
    }
    #[inline]
    pub const fn dwl(&self) -> u32 {
        (self.flags >> 1) & 0x01
    }
    #[inline]
    pub const fn dhl(&self) -> u32 {
        (self.flags >> 2) & 0x03
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct VTermLineInfo {
    pub flags: u32,
}

impl VTermLineInfo {
    #[inline]
    pub const fn doublewidth(&self) -> u32 {
        self.flags & 0x01
    }
    #[inline]
    pub const fn set_doublewidth(&mut self, val: u32) {
        self.flags = (self.flags & !0x01) | (val & 0x01);
    }
    #[inline]
    pub const fn doubleheight(&self) -> u32 {
        (self.flags >> 1) & 0x03
    }
    #[inline]
    pub const fn set_doubleheight(&mut self, val: u32) {
        self.flags = (self.flags & !(0x03 << 1)) | ((val & 0x03) << 1);
    }
    #[inline]
    pub const fn continuation(&self) -> bool {
        ((self.flags >> 3) & 0x01) != 0
    }
    #[inline]
    pub fn set_continuation(&mut self, val: bool) {
        self.flags = (self.flags & !(0x01 << 3)) | (u32::from(val) << 3);
    }
}

#[repr(C)]
pub struct VTermStateFields {
    pub pos: VTermPos,
    pub lineinfos: [Vec<VTermLineInfo>; 2],
}

#[repr(C)]
pub struct VTermScreenCallbacks {
    pub damage: Option<unsafe fn(rect: VTermRect, user: *mut Terminal) -> c_int>,
    pub moverect: Option<unsafe fn(dest: VTermRect, src: VTermRect, user: *mut Terminal) -> c_int>,
    pub movecursor: Option<crate::vterm::pen::VTermMoveCursorFn<Terminal>>,
    pub settermprop:
        Option<unsafe fn(prop: VTermProp, val: &mut VTermValue, user: *mut Terminal) -> c_int>,
    pub bell: Option<unsafe fn(user: *mut Terminal) -> c_int>,
    pub resize: Option<unsafe fn(rows: c_int, cols: c_int, user: *mut Terminal) -> c_int>,
    pub theme: Option<unsafe fn(dark: &mut bool, user: *mut Terminal) -> c_int>,
    pub sb_pushline:
        Option<unsafe fn(cols: c_int, cells: *const VTermScreenCell, user: *mut Terminal) -> c_int>,
    pub sb_popline:
        Option<unsafe fn(cols: c_int, cells: *mut VTermScreenCell, user: *mut Terminal) -> c_int>,
    pub sb_clear: Option<unsafe fn(user: *mut Terminal) -> c_int>,
}

#[repr(C)]
pub struct VTermScreen {
    pub vt: *mut VTerm,
    pub callbacks: Option<&'static VTermScreenCallbacks>,
    pub damage_merge: VTermDamageSize,
    pub damaged: VTermRect,
    pub pending_scrollrect: VTermRect,
    pub pending_scroll_downward: c_int,
    pub pending_scroll_rightward: c_int,
    pub rows: c_int,
    pub cols: c_int,
    pub flags: u32,
    pub buffers: [Vec<ScreenCell>; 2],
    pub bufidx: usize,
    pub sb_buffer: Vec<VTermScreenCell>,
    pub pen: ScreenPen,
}

impl VTermScreen {
    #[inline]
    pub fn state_ptr(&self) -> *mut VTermStateFull {
        unsafe { (*self.vt).state_mut_ptr() }
    }

    #[inline]
    pub fn term_ptr(&self) -> *mut Terminal {
        unsafe { (*self.vt).outdata }
    }

    #[inline]
    pub const fn global_reverse(&self) -> u32 {
        self.flags & 0x01
    }
    #[inline]
    pub const fn set_global_reverse(&mut self, val: u32) {
        self.flags = (self.flags & !0x01) | (val & 0x01);
    }
    #[inline]
    pub const fn reflow(&self) -> bool {
        ((self.flags >> 1) & 0x01) != 0
    }
    #[inline]
    pub fn set_reflow(&mut self, val: bool) {
        self.flags = (self.flags & !(0x01 << 1)) | (u32::from(val) << 1);
    }
}

#[inline]
fn clearcell(screen: &VTermScreen, cell: &mut ScreenCell) {
    cell.schar = 0;
    cell.pen = screen.pen;
}

#[inline]
fn vterm_rect_move(rect: &mut VTermRect, row_delta: c_int, col_delta: c_int) {
    rect.start_row += row_delta;
    rect.end_row += row_delta;
    rect.start_col += col_delta;
    rect.end_col += col_delta;
}

pub fn getcell(screen: &VTermScreen, row: c_int, col: c_int) -> *mut ScreenCell {
    if row < 0 || row >= screen.rows || col < 0 || col >= screen.cols {
            return ptr::null_mut();
        }
        let idx = (screen.cols * row + col) as usize;
        (&raw const screen.buffers[screen.bufidx][idx]).cast_mut()
}

fn alloc_buffer(screen: &mut VTermScreen, rows: c_int, cols: c_int) -> Vec<ScreenCell> {
    let count = (rows as usize) * (cols as usize);
        let mut new_buffer = vec![ScreenCell::default(); count];

        for cell in &mut new_buffer {
            clearcell(&*screen, cell);
        }

        new_buffer
}

fn damagerect(screen: &mut VTermScreen, rect: VTermRect) {
    unsafe {
        let emit = match screen.damage_merge {
            VTERM_DAMAGE_CELL => rect,
            VTERM_DAMAGE_ROW => {
                if rect.end_row > rect.start_row + 1 {
                    vterm_screen_flush_damage(screen);
                    rect
                } else if screen.damaged.start_row == -1 {
                    screen.damaged = rect;
                    return;
                } else if rect.start_row == screen.damaged.start_row {
                    if screen.damaged.start_col > rect.start_col {
                        screen.damaged.start_col = rect.start_col;
                    }
                    if screen.damaged.end_col < rect.end_col {
                        screen.damaged.end_col = rect.end_col;
                    }
                    return;
                } else {
                    let prev = screen.damaged;
                    screen.damaged = rect;
                    prev
                }
            }
            VTERM_DAMAGE_SCREEN | VTERM_DAMAGE_SCROLL => {
                if screen.damaged.start_row == -1 {
                    screen.damaged = rect;
                } else {
                    rect_expand(&mut screen.damaged, &rect);
                }
                return;
            }
            _ => return,
        };

        if let Some(callbacks) = screen.callbacks
            && let Some(cb) = callbacks.damage
        {
            cb(emit, screen.term_ptr());
        }
    }
}

fn damagescreen(screen: &mut VTermScreen) {
    let rect = VTermRect {
            start_row: 0,
            end_row: screen.rows,
            start_col: 0,
            end_col: screen.cols,
        };
        damagerect(screen, rect);
}

unsafe fn putglyph(info: *mut VTermGlyphInfo, pos: VTermPos, screen: *mut VTermScreen) -> c_int {
    unsafe {
        let screen = &mut *screen;
        let cell = getcell(screen, pos.row, pos.col);

        if cell.is_null() {
            return 0;
        }

        (*cell).schar = (*info).schar;
        if (*info).schar != 0 {
            (*cell).pen = screen.pen;
        }

        for col in 1..(*info).width {
            let onecell = getcell(screen, pos.row, pos.col + col);
            if onecell.is_null() {
                break;
            }
            (*onecell).schar = u32::MAX;
        }

        let rect = VTermRect {
            start_row: pos.row,
            end_row: pos.row + 1,
            start_col: pos.col,
            end_col: pos.col + (*info).width,
        };

        (*cell).pen.set_protected_cell((*info).protected_cell());
        (*cell).pen.set_dwl((*info).dwl());
        (*cell).pen.set_dhl((*info).dhl());

        damagerect(screen, rect);

        1
    }
}

fn sb_pushline_from_row(screen: &mut VTermScreen, row: c_int) {
    unsafe {
        let mut pos = VTermPos { row, col: 0 };
        let sb_buf_ptr = screen.sb_buffer.as_mut_ptr();
        while pos.col < screen.cols {
            vterm_screen_get_cell(screen, pos, sb_buf_ptr.add(pos.col as usize));
            pos.col += 1;
        }

        if let Some(callbacks) = screen.callbacks
            && let Some(cb) = callbacks.sb_pushline
        {
            cb(screen.cols, sb_buf_ptr, screen.term_ptr());
        }
    }
}

unsafe fn moverect_internal(dest: VTermRect, src: VTermRect, screen: *mut VTermScreen) -> c_int {
    unsafe {
        let screen = &mut *screen;
        if screen
            .callbacks
            .is_some_and(|cbs| cbs.sb_pushline.is_some())
            && dest.start_row == 0
            && dest.start_col == 0
            && dest.end_col == screen.cols
            && screen.bufidx == BUFIDX_PRIMARY
        {
            for row in 0..src.start_row {
                sb_pushline_from_row(screen, row);
            }
        }

        let cols = (src.end_col - src.start_col) as usize;
        let downward = src.start_row - dest.start_row;

        let (init_row, test_row, inc_row) = if downward < 0 {
            (dest.end_row - 1, dest.start_row - 1, -1)
        } else {
            (dest.start_row, dest.end_row, 1)
        };

        let mut row = init_row;
        while row != test_row {
            ptr::copy(
                getcell(screen, row + downward, src.start_col),
                getcell(screen, row, dest.start_col),
                cols,
            );
            row += inc_row;
        }

        1
    }
}

unsafe fn moverect_user(dest: VTermRect, src: VTermRect, screen: *mut VTermScreen) -> c_int {
    unsafe {
        let screen = &mut *screen;
        if let Some(callbacks) = screen.callbacks
            && let Some(cb) = callbacks.moverect
        {
            if screen.damage_merge != VTERM_DAMAGE_SCROLL {
                vterm_screen_flush_damage(screen);
            }

            if cb(dest, src, screen.term_ptr()) != 0 {
                return 1;
            }
        }

        damagerect(screen, dest);

        1
    }
}

unsafe fn erase_internal(rect: VTermRect, selective: c_int, screen: *mut VTermScreen) -> c_int {
    unsafe {
        let screen = &mut *screen;
        let mut row = rect.start_row;
        let state = screen.state_ptr();
        while row < (*state).rows && row < rect.end_row {
            let info = vterm_state_get_lineinfo(&*state, row);

            for col in rect.start_col..rect.end_col {
                let cell = getcell(screen, row, col);

                if selective != 0 && (*cell).pen.protected_cell() != 0 {
                    continue;
                }

                (*cell).schar = 0;
                (*cell).pen = ScreenPen {
                    fg: screen.pen.fg,
                    bg: screen.pen.bg,
                    uri: 0,
                    flags: 0,
                };
                (*cell).pen.set_dwl(info.doublewidth());
                (*cell).pen.set_dhl(info.doubleheight());
            }
            row += 1;
        }

        1
    }
}

unsafe fn erase_user(rect: VTermRect, _selective: c_int, screen: *mut VTermScreen) -> c_int {
    unsafe {
        let screen = &mut *screen;
        damagerect(screen, rect);
        1
    }
}

unsafe fn erase(rect: VTermRect, selective: c_int, screen: *mut VTermScreen) -> c_int {
    unsafe {
        let screen = &mut *screen;
        erase_internal(rect, selective, screen);
        erase_user(rect, 0, screen)
    }
}

unsafe fn scrollrect(
    mut rect: VTermRect,
    downward: c_int,
    rightward: c_int,
    screen: *mut VTermScreen,
) -> c_int {
    unsafe {
        let screen = &mut *screen;
        if screen.damage_merge != VTERM_DAMAGE_SCROLL {
            vterm_scroll_rect(
                rect,
                downward,
                rightward,
                Some(moverect_internal),
                Some(erase_internal),
                &raw mut *screen,
            );

            vterm_screen_flush_damage(screen);

            vterm_scroll_rect(
                rect,
                downward,
                rightward,
                Some(moverect_user),
                Some(erase_user),
                &raw mut *screen,
            );

            return 1;
        }

        if screen.damaged.start_row != -1
            && rect_intersects(&mut rect, &mut screen.damaged) == 0
        {
            vterm_screen_flush_damage(screen);
        }

        if screen.pending_scrollrect.start_row == -1 {
            screen.pending_scrollrect = rect;
            screen.pending_scroll_downward = downward;
            screen.pending_scroll_rightward = rightward;
        } else if rect_equal(&mut screen.pending_scrollrect, &mut rect) != 0
            && ((screen.pending_scroll_downward == 0 && downward == 0)
                || (screen.pending_scroll_rightward == 0 && rightward == 0))
        {
            screen.pending_scroll_downward += downward;
            screen.pending_scroll_rightward += rightward;
        } else {
            vterm_screen_flush_damage(screen);

            screen.pending_scrollrect = rect;
            screen.pending_scroll_downward = downward;
            screen.pending_scroll_rightward = rightward;
        }

        vterm_scroll_rect(
            rect,
            downward,
            rightward,
            Some(moverect_internal),
            Some(erase_internal),
            &raw mut *screen,
        );

        if screen.damaged.start_row == -1 {
            return 1;
        }

        if rect_contains(&mut rect, &mut screen.damaged) != 0 {
            vterm_rect_move(&mut screen.damaged, -downward, -rightward);
            rect_clip(&mut screen.damaged, &mut rect);
        } else if rect.start_col <= screen.damaged.start_col
            && rect.end_col >= screen.damaged.end_col
            && rightward == 0
        {
            if screen.damaged.start_row >= rect.start_row
                && screen.damaged.start_row < rect.end_row
            {
                screen.damaged.start_row -= downward;
                if screen.damaged.start_row < rect.start_row {
                    screen.damaged.start_row = rect.start_row;
                }
                if screen.damaged.start_row > rect.end_row {
                    screen.damaged.start_row = rect.end_row;
                }
            }
            if screen.damaged.end_row >= rect.start_row
                && screen.damaged.end_row < rect.end_row
            {
                screen.damaged.end_row -= downward;
                if screen.damaged.end_row < rect.start_row {
                    screen.damaged.end_row = rect.start_row;
                }
                if screen.damaged.end_row > rect.end_row {
                    screen.damaged.end_row = rect.end_row;
                }
            }
        }

        1
    }
}

unsafe fn movecursor(
    pos: VTermPos,
    oldpos: VTermPos,
    visible: c_int,
    screen: *mut VTermScreen,
) -> c_int {
    unsafe {
        let screen = &mut *screen;
        if let Some(callbacks) = screen.callbacks
            && let Some(cb) = callbacks.movecursor
        {
            return cb(pos, oldpos, visible, screen.term_ptr());
        }
        0
    }
}

unsafe fn setpenattr(attr: VTermAttr, val: &mut VTermValue, screen: *mut VTermScreen) -> c_int {
    unsafe {
        let screen = &mut *screen;
        match attr {
            VTERM_ATTR_BOLD => {
                screen.pen.set_bold(val.boolean() as u32);
                1
            }
            VTERM_ATTR_UNDERLINE => {
                screen.pen.set_underline(val.number() as u32);
                1
            }
            VTERM_ATTR_ITALIC => {
                screen.pen.set_italic(val.boolean() as u32);
                1
            }
            VTERM_ATTR_BLINK => {
                screen.pen.set_blink(val.boolean() as u32);
                1
            }
            VTERM_ATTR_REVERSE => {
                screen.pen.set_reverse(val.boolean() as u32);
                1
            }
            VTERM_ATTR_CONCEAL => {
                screen.pen.set_conceal(val.boolean() as u32);
                1
            }
            VTERM_ATTR_STRIKE => {
                screen.pen.set_strike(val.boolean() as u32);
                1
            }
            VTERM_ATTR_FONT => {
                screen.pen.set_font(val.number() as u32);
                1
            }
            VTERM_ATTR_FOREGROUND => {
                screen.pen.fg = val.color();
                1
            }
            VTERM_ATTR_BACKGROUND => {
                screen.pen.bg = val.color();
                1
            }
            VTERM_ATTR_SMALL => {
                screen.pen.set_small(val.boolean() as u32);
                1
            }
            VTERM_ATTR_BASELINE => {
                screen.pen.set_baseline(val.number() as u32);
                1
            }
            VTERM_ATTR_URI => {
                screen.pen.uri = val.number();
                1
            }
            VTERM_ATTR_DIM => {
                screen.pen.set_dim(val.boolean() as u32);
                1
            }
            VTERM_ATTR_OVERLINE => {
                screen.pen.set_overline(val.boolean() as u32);
                1
            }
            _ => 0,
        }
    }
}

unsafe fn settermprop(prop: VTermProp, val: &mut VTermValue, screen: *mut VTermScreen) -> c_int {
    unsafe {
        let screen = &mut *screen;
        match prop {
            VTERM_PROP_ALTSCREEN => {
                if val.boolean() != 0 && screen.buffers[BUFIDX_ALTSCREEN].is_empty() {
                    return 0;
                }
                screen.bufidx = if val.boolean() != 0 {
                    BUFIDX_ALTSCREEN
                } else {
                    BUFIDX_PRIMARY
                };
                if val.boolean() == 0 {
                    damagescreen(screen);
                }
            }
            VTERM_PROP_REVERSE => {
                screen.set_global_reverse(val.boolean() as u32);
                damagescreen(screen);
            }
            _ => {}
        }

        if let Some(callbacks) = screen.callbacks
            && let Some(cb) = callbacks.settermprop
        {
            return cb(prop, val, screen.term_ptr());
        }

        1
    }
}

unsafe fn bell(screen: *mut VTermScreen) -> c_int {
    unsafe {
        let screen = &mut *screen;
        if let Some(callbacks) = screen.callbacks
            && let Some(cb) = callbacks.bell
        {
            return cb(screen.term_ptr());
        }
        0
    }
}

fn line_popcount(buffer: &[ScreenCell], row: c_int, _rows: c_int, cols: c_int) -> c_int {
    let mut col = cols - 1;
    while col >= 0 && buffer[(row * cols + col) as usize].schar == 0 {
        col -= 1;
    }
    col + 1
}

unsafe fn resize_buffer(
    screen: &mut VTermScreen,
    bufidx: usize,
    new_rows: c_int,
    new_cols: c_int,
    active: bool,
    statefields: *mut VTermStateFields,
) {
    unsafe {
        let old_rows = screen.rows;
        let old_cols = screen.cols;

        let mut new_buffer_vec =
            vec![ScreenCell::default(); (new_rows as usize) * (new_cols as usize)];
        let mut new_lineinfo_vec = vec![VTermLineInfo::default(); new_rows as usize];

        let mut old_row = old_rows - 1;
        let mut new_row = new_rows - 1;

        let old_cursor = (*statefields).pos;
        let mut new_cursor = VTermPos { row: -1, col: -1 };

        let mut final_blank_row = new_rows;
        let do_reflow = screen.reflow() && (bufidx == BUFIDX_PRIMARY);

        {
            let old_buffer = &screen.buffers[bufidx];
            let old_lineinfo = &(&*statefields).lineinfos[bufidx];
            let has_old_lineinfo = !old_lineinfo.is_empty();

            while old_row >= 0 {
                let old_row_end = old_row;
                while do_reflow
                    && has_old_lineinfo
                    && old_row > 0
                    && old_lineinfo[old_row as usize].continuation()
                {
                    old_row -= 1;
                }
                let old_row_start = old_row;

                let mut width: c_int = 0;
                for row in old_row_start..=old_row_end {
                    if do_reflow
                        && row < (old_rows - 1)
                        && old_lineinfo[(row + 1) as usize].continuation()
                    {
                        width += old_cols;
                    } else {
                        width += line_popcount(old_buffer, row, old_rows, old_cols);
                    }
                }

                if final_blank_row == (new_row + 1) && width == 0 {
                    final_blank_row = new_row;
                }

                let new_height = if do_reflow {
                    if width != 0 {
                        (width + new_cols - 1) / new_cols
                    } else {
                        1
                    }
                } else {
                    1
                };

                let mut new_row_end = new_row;
                let mut new_row_start = new_row - new_height + 1;

                old_row = old_row_start;
                let mut old_col: c_int = 0;

                let spare_rows = new_rows - final_blank_row;

                if new_row_start < 0
                    && spare_rows >= 0
                    && (!active
                        || new_cursor.row == -1
                        || (new_cursor.row - new_row_start) < new_rows)
                {
                    let mut downwards = -new_row_start;
                    if downwards > spare_rows {
                        downwards = spare_rows;
                    }
                    let rowcount = new_rows - downwards;

                    new_buffer_vec.copy_within(
                        0..(rowcount as usize) * (new_cols as usize),
                        (downwards * new_cols) as usize,
                    );
                    new_lineinfo_vec.copy_within(0..rowcount as usize, downwards as usize);

                    new_row += downwards;
                    new_row_start += downwards;
                    new_row_end += downwards;

                    if new_cursor.row >= 0 {
                        new_cursor.row += downwards;
                    }

                    final_blank_row += downwards;
                }

                if new_row_start < 0 {
                    if old_row_start <= old_cursor.row && old_cursor.row <= old_row_end {
                        new_cursor.row = 0;
                        new_cursor.col = old_cursor.col;
                        if new_cursor.col >= new_cols {
                            new_cursor.col = new_cols - 1;
                        }
                    }
                    break;
                }

                new_row = new_row_start;
                old_row = old_row_start;
                while new_row <= new_row_end {
                    let mut count = if width >= new_cols { new_cols } else { width };
                    width -= count;

                    let mut new_col: c_int = 0;

                    while count != 0 {
                        new_buffer_vec[(new_row * new_cols + new_col) as usize] =
                            old_buffer[(old_row * old_cols + old_col) as usize];

                        if old_cursor.row == old_row && old_cursor.col == old_col {
                            new_cursor.row = new_row;
                            new_cursor.col = new_col;
                        }

                        old_col += 1;
                        if old_col == old_cols {
                            old_row += 1;

                            if !do_reflow {
                                new_col += 1;
                                break;
                            }
                            old_col = 0;
                        }

                        new_col += 1;
                        count -= 1;
                    }

                    if old_row <= old_row_end
                        && old_cursor.row == old_row
                        && old_cursor.col >= old_col
                    {
                        new_cursor.row = new_row;
                        new_cursor.col = old_cursor.col - old_col + new_col;
                        if new_cursor.col >= new_cols {
                            new_cursor.col = new_cols - 1;
                        }
                    }

                    while new_col < new_cols {
                        clearcell(
                            &*screen,
                            &mut new_buffer_vec[(new_row * new_cols + new_col) as usize],
                        );
                        new_col += 1;
                    }

                    new_lineinfo_vec[new_row as usize].set_continuation(new_row > new_row_start);
                    new_row += 1;
                }

                old_row = old_row_start - 1;
                new_row = new_row_start - 1;
            }
        }

        if old_cursor.row <= old_row {
            new_cursor.row = 0;
            new_cursor.col = old_cursor.col;
            if new_cursor.col >= new_cols {
                new_cursor.col = new_cols - 1;
            }
        }

        if active && (new_cursor.row == -1 || new_cursor.col == -1) {
            std::process::abort();
        }

        if old_row >= 0 && bufidx == BUFIDX_PRIMARY {
            if screen
                .callbacks
                .is_some_and(|cbs| cbs.sb_pushline.is_some())
            {
                for row in 0..=old_row {
                    sb_pushline_from_row(screen, row);
                }
            }
            if active {
                (*statefields).pos.row -= old_row + 1;
            }
        }
        if new_row >= 0
            && bufidx == BUFIDX_PRIMARY
            && let Some(callbacks) = screen.callbacks
            && let Some(sb_popline) = callbacks.sb_popline
        {
            while new_row >= 0 {
                let sb_buf_ptr = screen.sb_buffer.as_mut_ptr();
                if sb_popline(old_cols, sb_buf_ptr, screen.term_ptr()) == 0 {
                    break;
                }

                let mut pos = VTermPos {
                    row: new_row,
                    col: 0,
                };
                while pos.col < old_cols && pos.col < new_cols {
                    let src = &screen.sb_buffer[pos.col as usize];
                    let dst_idx = (pos.row * new_cols + pos.col) as usize;
                    let dst = &mut new_buffer_vec[dst_idx];

                    dst.schar = src.schar;

                    dst.pen.set_bold(src.attrs.bold());
                    dst.pen.set_underline(src.attrs.underline());
                    dst.pen.set_italic(src.attrs.italic());
                    dst.pen.set_blink(src.attrs.blink());
                    dst.pen
                        .set_reverse(src.attrs.reverse() ^ screen.global_reverse());
                    dst.pen.set_conceal(src.attrs.conceal());
                    dst.pen.set_strike(src.attrs.strike());
                    dst.pen.set_font(src.attrs.font());
                    dst.pen.set_small(src.attrs.small());
                    dst.pen.set_baseline(src.attrs.baseline());
                    dst.pen.set_dim(src.attrs.dim());
                    dst.pen.set_overline(src.attrs.overline());

                    dst.pen.fg = src.fg;
                    dst.pen.bg = src.bg;
                    dst.pen.uri = src.uri;

                    let width = c_int::from(src.width);
                    if width == 2 && pos.col < (new_cols - 1) {
                        new_buffer_vec[dst_idx + 1].schar = u32::MAX;
                    }
                    pos.col += width;
                }
                while pos.col < new_cols {
                    clearcell(
                        &*screen,
                        &mut new_buffer_vec[(pos.row * new_cols + pos.col) as usize],
                    );
                    pos.col += 1;
                }
                new_row -= 1;

                if active {
                    (*statefields).pos.row += 1;
                }
            }
        }
        if new_row >= 0 {
            let moverows = new_rows - new_row - 1;
            let src_start = ((new_row + 1) * new_cols) as usize;
            let count = (moverows as usize) * (new_cols as usize);
            new_buffer_vec.copy_within(src_start..src_start + count, 0);
            let li_start = (new_row + 1) as usize;
            new_lineinfo_vec.copy_within(li_start..li_start + (moverows as usize), 0);

            new_cursor.row -= new_row + 1;

            for r in moverows..new_rows {
                for col in 0..new_cols {
                    clearcell(&*screen, &mut new_buffer_vec[(r * new_cols + col) as usize]);
                }
                new_lineinfo_vec[r as usize] = VTermLineInfo { flags: 0 };
            }
        }

        screen.buffers[bufidx] = new_buffer_vec;
        (&mut *statefields).lineinfos[bufidx] = new_lineinfo_vec;

        if active {
            (*statefields).pos = new_cursor;
        }
    }
}

unsafe fn resize(
    new_rows: c_int,
    new_cols: c_int,
    fields: *mut VTermStateFields,
    screen: *mut VTermScreen,
) -> c_int {
    unsafe {
        let screen = &mut *screen;
        let altscreen_active = !screen.buffers[BUFIDX_ALTSCREEN].is_empty()
            && screen.bufidx == BUFIDX_ALTSCREEN;

        let old_rows = screen.rows;
        let old_cols = screen.cols;

        if new_cols > old_cols {
            screen.sb_buffer = vec![VTermScreenCell::default(); new_cols as usize];
        }

        resize_buffer(screen, 0, new_rows, new_cols, !altscreen_active, fields);
        if !screen.buffers[BUFIDX_ALTSCREEN].is_empty() {
            resize_buffer(screen, 1, new_rows, new_cols, altscreen_active, fields);
        } else if new_rows != old_rows {
            (&mut *fields).lineinfos[BUFIDX_ALTSCREEN] =
                vec![VTermLineInfo { flags: 0 }; new_rows as usize];
        }

        screen.bufidx = if altscreen_active {
            BUFIDX_ALTSCREEN
        } else {
            BUFIDX_PRIMARY
        };

        screen.rows = new_rows;
        screen.cols = new_cols;

        if new_cols <= old_cols {
            screen.sb_buffer = vec![VTermScreenCell::default(); new_cols as usize];
        }

        damagescreen(screen);

        if let Some(callbacks) = screen.callbacks
            && let Some(cb) = callbacks.resize
        {
            return cb(new_rows, new_cols, screen.term_ptr());
        }

        1
    }
}

unsafe fn theme(dark: &mut bool, screen: *mut VTermScreen) -> c_int {
    unsafe {
        let screen = &mut *screen;
        if let Some(callbacks) = screen.callbacks
            && let Some(cb) = callbacks.theme
        {
            return cb(dark, screen.term_ptr());
        }
        1
    }
}

unsafe fn setlineinfo(
    row: c_int,
    newinfo_ptr: *const VTermLineInfo,
    oldinfo_ptr: *const VTermLineInfo,
    screen: *mut VTermScreen,
) -> c_int {
    unsafe {
        let screen = &mut *screen;
        let newinfo = &*newinfo_ptr;
        let oldinfo = &*oldinfo_ptr;

        if newinfo.doublewidth() != oldinfo.doublewidth()
            || newinfo.doubleheight() != oldinfo.doubleheight()
        {
            for col in 0..screen.cols {
                let cell = getcell(screen, row, col);
                (*cell).pen.set_dwl(newinfo.doublewidth());
                (*cell).pen.set_dhl(newinfo.doubleheight());
            }

            let mut rect = VTermRect {
                start_row: row,
                end_row: row + 1,
                start_col: 0,
                end_col: if newinfo.doublewidth() != 0 {
                    screen.cols / 2
                } else {
                    screen.cols
                },
            };
            damagerect(screen, rect);

            if newinfo.doublewidth() != 0 {
                rect.start_col = screen.cols / 2;
                rect.end_col = screen.cols;
                erase_internal(rect, 0, screen);
            }
        }

        1
    }
}

unsafe fn sb_clear(screen: *mut VTermScreen) -> c_int {
    unsafe {
        let screen = &mut *screen;
        if let Some(callbacks) = screen.callbacks
            && let Some(cb) = callbacks.sb_clear
            && cb(screen.term_ptr()) != 0
        {
            return 1;
        }
        0
    }
}

static STATE_CBS: VTermStateCallbacks = VTermStateCallbacks {
    putglyph: Some(putglyph),
    movecursor: Some(movecursor),
    scrollrect: Some(scrollrect),
    moverect: None,
    erase: Some(erase),
    initpen: None,
    setpenattr: Some(setpenattr),
    settermprop: Some(settermprop),
    bell: Some(bell),
    resize: Some(resize),
    theme: Some(theme),
    setlineinfo: Some(setlineinfo),
    sb_clear: Some(sb_clear),
};

fn screen_new(vt: &mut VTerm) -> Option<Box<VTermScreen>> {
    unsafe {
        let _state = vterm_obtain_state(vt);
        if false {
            return None;
        }

        let mut rows: c_int = 0;
        let mut cols: c_int = 0;
        vterm_get_size(vt, &raw mut rows, &raw mut cols);

        let mut screen = Box::new(VTermScreen {
            vt,
            callbacks: None,
            damage_merge: VTERM_DAMAGE_CELL,
            damaged: VTermRect {
                start_row: -1,
                end_row: 0,
                start_col: 0,
                end_col: 0,
            },
            pending_scrollrect: VTermRect {
                start_row: -1,
                end_row: 0,
                start_col: 0,
                end_col: 0,
            },
            pending_scroll_downward: 0,
            pending_scroll_rightward: 0,
            rows,
            cols,
            flags: 0,
            buffers: [Vec::new(), Vec::new()],
            bufidx: BUFIDX_PRIMARY,
            sb_buffer: vec![VTermScreenCell::default(); cols as usize],
            pen: ScreenPen::default(),
        });

        screen.set_global_reverse(0);
        screen.set_reflow(false);

        let screen_ptr: *mut VTermScreen = &raw mut *screen;
        screen.buffers[BUFIDX_PRIMARY] = alloc_buffer(&mut *screen_ptr, rows, cols);

        Some(screen)
    }
}

pub fn vterm_screen_reset(screen: &mut VTermScreen, hard: c_int) {
    unsafe {
        screen.damaged.start_row = -1;
        screen.pending_scrollrect.start_row = -1;
        vterm_state_reset(&mut *screen.state_ptr(), hard);
        vterm_screen_flush_damage(screen);
    }
}

pub unsafe fn vterm_screen_get_cell(
    screen: &VTermScreen,
    pos: VTermPos,
    cell: *mut VTermScreenCell,
) -> c_int {
    unsafe {
        let intcell = getcell(screen, pos.row, pos.col);
        if intcell.is_null() {
            return 0;
        }

        (*cell).schar = if (*intcell).schar == u32::MAX {
            0
        } else {
            (*intcell).schar
        };

        (*cell).attrs.set_bold((*intcell).pen.bold());
        (*cell).attrs.set_underline((*intcell).pen.underline());
        (*cell).attrs.set_italic((*intcell).pen.italic());
        (*cell).attrs.set_blink((*intcell).pen.blink());
        (*cell)
            .attrs
            .set_reverse((*intcell).pen.reverse() ^ screen.global_reverse());
        (*cell).attrs.set_conceal((*intcell).pen.conceal());
        (*cell).attrs.set_strike((*intcell).pen.strike());
        (*cell).attrs.set_font((*intcell).pen.font());
        (*cell).attrs.set_small((*intcell).pen.small());
        (*cell).attrs.set_baseline((*intcell).pen.baseline());
        (*cell).attrs.set_dim((*intcell).pen.dim());
        (*cell).attrs.set_overline((*intcell).pen.overline());

        (*cell).attrs.set_dwl((*intcell).pen.dwl());
        (*cell).attrs.set_dhl((*intcell).pen.dhl());

        (*cell).fg = (*intcell).pen.fg;
        (*cell).bg = (*intcell).pen.bg;
        (*cell).uri = (*intcell).pen.uri;

        if pos.col < (screen.cols - 1)
            && (*getcell(screen, pos.row, pos.col + 1)).schar == u32::MAX
        {
            (*cell).width = 2;
        } else {
            (*cell).width = 1;
        }

        1
    }
}

pub fn vterm_obtain_screen(vt: &mut VTerm) -> &'static mut VTermScreen {
    unsafe {
        if vt.screen.is_none() {
            vt.screen = screen_new(vt);
            let screen_ptr = vt.screen_mut_ptr();
            if !screen_ptr.is_null() {
                vterm_state_set_callbacks(&mut *vt.state_mut_ptr(), Some(&STATE_CBS), screen_ptr);
            }
        }
        &mut *vt.screen_mut_ptr()
    }
}

pub fn vterm_screen_enable_reflow(screen: &mut VTermScreen, reflow: bool) {
    screen.set_reflow(reflow);
}

pub fn vterm_screen_enable_altscreen(screen: &mut VTermScreen, altscreen: c_int) {
    unsafe {
        if screen.buffers[BUFIDX_ALTSCREEN].is_empty() && altscreen != 0 {
            let mut rows: c_int = 0;
            let mut cols: c_int = 0;
            vterm_get_size(&*screen.vt, &raw mut rows, &raw mut cols);

            screen.buffers[BUFIDX_ALTSCREEN] = alloc_buffer(screen, rows, cols);
        }
    }
}

pub unsafe fn vterm_screen_set_callbacks(
    screen: &mut VTermScreen,
    callbacks: Option<&'static VTermScreenCallbacks>,
    user: *mut Terminal,
) {
    unsafe {
        screen.callbacks = callbacks;
        if !user.is_null() {
            (*screen.vt).outdata = user;
        }
    }
}

pub unsafe fn vterm_screen_set_unrecognised_fallbacks(
    screen: &mut VTermScreen,
    fallbacks: Option<&'static VTermStateFallbacks>,
    user: *mut Terminal,
) {
    unsafe {
        vterm_state_set_unrecognised_fallbacks(&mut *screen.state_ptr(), fallbacks, user);
    }
}

pub fn vterm_screen_flush_damage(screen: &mut VTermScreen) {
    unsafe {
        if screen.pending_scrollrect.start_row != -1 {
            vterm_scroll_rect(
                screen.pending_scrollrect,
                screen.pending_scroll_downward,
                screen.pending_scroll_rightward,
                Some(moverect_user),
                Some(erase_user),
                &raw mut *screen,
            );

            screen.pending_scrollrect.start_row = -1;
        }

        if screen.damaged.start_row != -1 {
            if let Some(callbacks) = screen.callbacks
                && let Some(cb) = callbacks.damage
            {
                cb(screen.damaged, screen.term_ptr());
            }

            screen.damaged.start_row = -1;
        }
    }
}

pub fn vterm_screen_set_damage_merge(screen: &mut VTermScreen, size: VTermDamageSize) {
    vterm_screen_flush_damage(screen);
        screen.damage_merge = size;
}

#[cfg(test)]
pub fn vterm_screen_convert_color_to_rgb(screen: &VTermScreen, col: &mut VTermColor) {
    unsafe {
        crate::vterm::pen::vterm_state_convert_color_to_rgb(&*screen.state_ptr(), col);
    }
}

pub fn rect_expand(dst: &mut VTermRect, src: &VTermRect) {
    if dst.start_row > src.start_row {
            dst.start_row = src.start_row;
        }
        if dst.start_col > src.start_col {
            dst.start_col = src.start_col;
        }
        if dst.end_row < src.end_row {
            dst.end_row = src.end_row;
        }
        if dst.end_col < src.end_col {
            dst.end_col = src.end_col;
        }
}

pub fn rect_clip(dst: &mut VTermRect, bounds: &mut VTermRect) {
    if dst.start_row < bounds.start_row {
            dst.start_row = bounds.start_row;
        }
        if dst.start_col < bounds.start_col {
            dst.start_col = bounds.start_col;
        }
        if dst.end_row > bounds.end_row {
            dst.end_row = bounds.end_row;
        }
        if dst.end_col > bounds.end_col {
            dst.end_col = bounds.end_col;
        }
        if dst.end_row < dst.start_row {
            dst.end_row = dst.start_row;
        }
        if dst.end_col < dst.start_col {
            dst.end_col = dst.start_col;
        }
}

pub fn rect_equal(a: &mut VTermRect, b: &mut VTermRect) -> c_int {
    c_int::from(
            a.start_row == b.start_row
                && a.start_col == b.start_col
                && a.end_row == b.end_row
                && a.end_col == b.end_col,
        )
}

pub fn rect_contains(big: &mut VTermRect, small: &mut VTermRect) -> c_int {
    if small.start_row < big.start_row
            || small.start_col < big.start_col
            || small.end_row > big.end_row
            || small.end_col > big.end_col
        {
            0
        } else {
            1
        }
}

pub fn rect_intersects(a: &mut VTermRect, b: &mut VTermRect) -> c_int {
    if a.start_row > b.end_row
            || b.start_row > a.end_row
            || a.start_col > b.end_col
            || b.start_col > a.end_col
        {
            0
        } else {
            1
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_operations_expand_clip_and_check_overlap() {
        let mut r1 = VTermRect {
            start_row: 2,
            end_row: 5,
            start_col: 3,
            end_col: 8,
        };
        let mut r2 = VTermRect {
            start_row: 1,
            end_row: 4,
            start_col: 6,
            end_col: 10,
        };
        assert_eq!(rect_intersects(&mut r1, &mut r2), 1);
            assert_eq!(rect_equal(&mut r1, &mut r2), 0);
            rect_expand(&mut r1, &r2);
            assert_eq!(
                (r1.start_row, r1.end_row, r1.start_col, r1.end_col),
                (1, 5, 3, 10)
            );
            assert_eq!(rect_contains(&mut r1, &mut r2), 1);
    }
}
