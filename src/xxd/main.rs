//! `xxd`: hexdump and reverse-hexdump utility (`src/xxd/xxd.c`).

use std::env;
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, BufWriter, IsTerminal, Read, Seek, SeekFrom, Write};
use std::process;

const VERSION: &str = "xxd 2025-11-26 by Juergen Weigert et al.";
#[cfg(windows)]
const OSVER: &str = " (Win32)";
#[cfg(not(windows))]
const OSVER: &str = "";

#[cfg(windows)]
const PATH_SEP: char = '\\';
#[cfg(not(windows))]
const PATH_SEP: char = '/';

const COLS: usize = 256;
const LLEN_NO_COLOR: usize = 39 + 2 + 9 * COLS + 2 + COLS + 2;

const HEX_NORMAL: u8 = 0x00;
const HEX_POSTSCRIPT: u8 = 0x01;
const HEX_CINCLUDE: u8 = 0x02;
const HEX_BITS: u8 = 0x04;
const HEX_LITTLEENDIAN: u8 = 0x08;

const COLOR_RED: u8 = b'1';
const COLOR_GREEN: u8 = b'2';
const COLOR_YELLOW: u8 = b'3';
const COLOR_BLUE: u8 = b'4';
const COLOR_WHITE: u8 = b'7';

const HEX_LOWER: &[u8; 16] = b"0123456789abcdef";
const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

// EBCDIC to ASCII conversion table from a proposed BTL standard April 16, 1979
const ETOA64: [u8; 192] = [
    0o040, 0o240, 0o241, 0o242, 0o243, 0o244, 0o245, 0o246, 0o247, 0o250, 0o325, 0o056, 0o074,
    0o050, 0o053, 0o174, 0o046, 0o251, 0o252, 0o253, 0o254, 0o255, 0o256, 0o257, 0o260, 0o261,
    0o041, 0o044, 0o052, 0o051, 0o073, 0o176, 0o055, 0o057, 0o262, 0o263, 0o264, 0o265, 0o266,
    0o267, 0o270, 0o271, 0o313, 0o054, 0o045, 0o137, 0o076, 0o077, 0o272, 0o273, 0o274, 0o275,
    0o276, 0o277, 0o300, 0o301, 0o302, 0o140, 0o072, 0o043, 0o100, 0o047, 0o075, 0o042, 0o303,
    0o141, 0o142, 0o143, 0o144, 0o145, 0o146, 0o147, 0o150, 0o151, 0o304, 0o305, 0o306, 0o307,
    0o310, 0o311, 0o312, 0o152, 0o153, 0o154, 0o155, 0o156, 0o157, 0o160, 0o161, 0o162, 0o136,
    0o314, 0o315, 0o316, 0o317, 0o320, 0o321, 0o345, 0o163, 0o164, 0o165, 0o166, 0o167, 0o170,
    0o171, 0o172, 0o322, 0o323, 0o324, 0o133, 0o326, 0o327, 0o330, 0o331, 0o332, 0o333, 0o334,
    0o335, 0o336, 0o337, 0o340, 0o341, 0o342, 0o343, 0o344, 0o135, 0o346, 0o347, 0o173, 0o101,
    0o102, 0o103, 0o104, 0o105, 0o106, 0o107, 0o110, 0o111, 0o350, 0o351, 0o352, 0o353, 0o354,
    0o355, 0o175, 0o112, 0o113, 0o114, 0o115, 0o116, 0o117, 0o120, 0o121, 0o122, 0o356, 0o357,
    0o360, 0o361, 0o362, 0o363, 0o134, 0o237, 0o123, 0o124, 0o125, 0o126, 0o127, 0o130, 0o131,
    0o132, 0o364, 0o365, 0o366, 0o367, 0o370, 0o371, 0o060, 0o061, 0o062, 0o063, 0o064, 0o065,
    0o066, 0o067, 0o070, 0o071, 0o372, 0o373, 0o374, 0o375, 0o376, 0o377,
];

fn exit_with_usage(pname: &str) -> ! {
    eprintln!("Usage:\n       {pname} [options] [infile [outfile]]");
    eprintln!("    or\n       {pname} -r [-s [-]offset] [-c cols] [-ps] [infile [outfile]]");
    eprintln!(
        "Options:\n\
         \x20   -a          toggle autoskip: A single '*' replaces nul-lines. Default off.\n\
         \x20   -b          binary digit dump (incompatible with -ps). Default hex.\n\
         \x20   -C          capitalize variable names in C include file style (-i).\n\
         \x20   -c cols     format <cols> octets per line. Default 16 (-i: 12, -ps: 30).\n\
         \x20   -E          show characters in EBCDIC. Default ASCII.\n\
         \x20   -e          little-endian dump (incompatible with -ps,-i,-r).\n\
         \x20   -g bytes    number of octets per group in normal output. Default 2 (-e: 4).\n\
         \x20   -h          print this summary.\n\
         \x20   -i          output in C include file style.\n\
         \x20   -l len      stop after <len> octets.\n\
         \x20   -n name     set the variable name used in C include output (-i).\n\
         \x20   -o off      add <off> to the displayed file position.\n\
         \x20   -ps         output in postscript plain hexdump style.\n\
         \x20   -r          reverse operation: convert (or patch) hexdump into binary.\n\
         \x20   -r -s off   revert with <off> added to file positions found in hexdump.\n\
         \x20   -d          show offset in decimal instead of hex."
    );
    eprintln!("    -s [+][-]seek  start at <seek> bytes abs. (or +: rel.) infile offset.");
    eprintln!(
        "    -u          use upper case hex letters.\n\
         \x20   -R when     colorize the output; <when> can be 'always', 'auto' or 'never'. Default: 'auto'.\n\
         \x20   -v          show version: \"{VERSION}{OSVER}\"."
    );
    process::exit(1);
}

fn error_exit(pname: &str, ret: i32, msg: &str) -> ! {
    eprintln!("{pname}: {msg}");
    process::exit(ret);
}

fn perror_exit(pname: &str, ret: i32, err: &io::Error) -> ! {
    eprintln!("{pname}: {err}");
    process::exit(ret);
}

fn c_strtoul(s: &str) -> u64 {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let mut neg = false;
    if i < bytes.len() {
        if bytes[i] == b'-' {
            neg = true;
            i += 1;
        } else if bytes[i] == b'+' {
            i += 1;
        }
    }
    let base: u64 = if i + 1 < bytes.len()
        && bytes[i] == b'0'
        && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X')
        && i + 2 < bytes.len()
        && bytes[i + 2].is_ascii_hexdigit()
    {
        i += 2;
        16
    } else if i < bytes.len() && bytes[i] == b'0' {
        i += 1;
        8
    } else {
        10
    };

    let mut acc: u64 = 0;
    while i < bytes.len() {
        let d = match bytes[i] {
            b'0'..=b'9' => u64::from(bytes[i] - b'0'),
            b'a'..=b'f' => u64::from(bytes[i] - b'a' + 10),
            b'A'..=b'F' => u64::from(bytes[i] - b'A' + 10),
            _ => break,
        };
        if d >= base {
            break;
        }
        acc = acc.wrapping_mul(base).wrapping_add(d);
        i += 1;
    }
    if neg { acc.wrapping_neg() } else { acc }
}

fn c_strtol(s: &str) -> i64 {
    c_strtoul(s) as i64
}

#[inline]
const fn parse_hex_digit(c: u8) -> i32 {
    match c {
        b'0'..=b'9' => (c - b'0') as i32,
        b'a'..=b'f' => (c - b'a' + 10) as i32,
        b'A'..=b'F' => (c - b'A' + 10) as i32,
        _ => -1,
    }
}

#[inline]
const fn parse_bin_digit(c: u8) -> i32 {
    match c {
        b'0'..=b'1' => (c - b'0') as i32,
        _ => -1,
    }
}

#[inline]
const fn get_color_char(e: u8, ebcdic: bool) -> u8 {
    if ebcdic {
        if matches!(
            e,
            75..=80
                | 90..=97
                | 107..=111
                | 121..=127
                | 129..=137
                | 145..=154
                | 162..=169
                | 192..=201
                | 208..=217
                | 226..=233
                | 240..=249
                | 64
                | 173
                | 189
                | 224
        ) {
            COLOR_GREEN
        } else if matches!(e, 5 | 13 | 37) {
            COLOR_YELLOW
        } else if e == 0 {
            COLOR_WHITE
        } else if e == 255 {
            COLOR_BLUE
        } else {
            COLOR_RED
        }
    } else if matches!(e, 32..=126) {
        COLOR_GREEN
    } else if matches!(e, 9 | 10 | 13) {
        COLOR_YELLOW
    } else if e == 0 {
        COLOR_WHITE
    } else if e == 255 {
        COLOR_BLUE
    } else {
        COLOR_RED
    }
}

fn enable_color() -> bool {
    if !io::stdout().is_terminal() {
        return false;
    }
    #[cfg(windows)]
    {
        const STD_OUTPUT_HANDLE: u32 = (-11i32) as u32;
        const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
        unsafe extern "system" {
            fn GetStdHandle(nStdHandle: u32) -> *mut std::ffi::c_void;
            fn GetConsoleMode(hConsoleHandle: *mut std::ffi::c_void, lpMode: *mut u32) -> i32;
            fn SetConsoleMode(hConsoleHandle: *mut std::ffi::c_void, dwMode: u32) -> i32;
        }
        unsafe {
            let out = GetStdHandle(STD_OUTPUT_HANDLE);
            let mut mode: u32 = 0;
            GetConsoleMode(out, &raw mut mode);
            mode |= ENABLE_VIRTUAL_TERMINAL_PROCESSING;
            SetConsoleMode(out, mode) != 0
        }
    }
    #[cfg(not(windows))]
    {
        true
    }
}

trait OutputSink: Write {
    fn try_seek_rel(&mut self, delta: i64) -> bool;
    fn finish_end(&mut self);
}

struct StdoutSink<W: Write>(W);
impl<W: Write> Write for StdoutSink<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}
impl<W: Write> OutputSink for StdoutSink<W> {
    fn try_seek_rel(&mut self, _delta: i64) -> bool {
        false
    }
    fn finish_end(&mut self) {}
}

struct FileSink(BufWriter<File>);
impl Write for FileSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}
impl OutputSink for FileSink {
    fn try_seek_rel(&mut self, delta: i64) -> bool {
        self.0.seek(SeekFrom::Current(delta)).is_ok()
    }
    fn finish_end(&mut self) {
        let _ = self.0.seek(SeekFrom::End(0));
    }
}

fn read_byte<R: Read>(r: &mut R, pname: &str) -> Option<u8> {
    let mut b = [0u8; 1];
    match r.read(&mut b) {
        Ok(0) => None,
        Ok(_) => Some(b[0]),
        Err(ref e) => perror_exit(pname, 2, e),
    }
}

fn write_all_or_die<W: Write>(w: &mut W, buf: &[u8], pname: &str) {
    if let Err(ref e) = w.write_all(buf) {
        perror_exit(pname, 3, e);
    }
}

fn skip_to_eol<R: Read>(r: &mut R, mut cur: Option<u8>, pname: &str) -> Option<u8> {
    while let Some(b) = cur {
        if b == b'\n' {
            return Some(b'\n');
        }
        cur = read_byte(r, pname);
    }
    None
}

fn huntype<R: Read, W: OutputSink>(
    fpi: &mut R,
    fpo: &mut W,
    cols: usize,
    hextype: u8,
    base_off: i64,
    pname: &str,
) -> i32 {
    let mut ign_garb = true;
    let mut n1: i32 = -1;
    let mut n2: i32 = 0;
    let mut n3: i32;
    let mut p = cols;
    let mut b: u8 = 0;
    let mut bcnt: usize = 0;
    let mut have_off: i64 = 0;
    let mut want_off: i64 = 0;

    while let Some(c_in) = read_byte(fpi, pname) {
        if c_in == b'\r' {
            continue;
        }
        if hextype == HEX_POSTSCRIPT && matches!(c_in, b' ' | b'\n' | b'\t') {
            continue;
        }

        if hextype == HEX_NORMAL || hextype == HEX_POSTSCRIPT {
            n3 = n2;
            n2 = n1;
            n1 = parse_hex_digit(c_in);
            if n1 == -1 && ign_garb {
                continue;
            }
        } else {
            n3 = n2;
            n1 = parse_hex_digit(c_in);
            if n1 == -1 && ign_garb {
                continue;
            }
            let bt = parse_bin_digit(c_in);
            if bt != -1 {
                b = (b << 1) | (bt as u8);
                bcnt += 1;
            }
        }

        ign_garb = false;

        if hextype != HEX_POSTSCRIPT && p >= cols {
            if hextype == HEX_NORMAL {
                if n1 < 0 {
                    p = 0;
                    continue;
                }
                want_off = (((want_off as u64) << 4) | (n1 as u64)) as i64;
            } else {
                if n1 < 0 {
                    p = 0;
                    bcnt = 0;
                    continue;
                }
                want_off = (((want_off as u64) << 4) | (n1 as u64)) as i64;
            }
            continue;
        }

        let target_off = base_off.wrapping_add(want_off);
        if target_off != have_off {
            if let Err(ref e) = fpo.flush() {
                perror_exit(pname, 3, e);
            }
            if fpo.try_seek_rel(target_off - have_off) {
                have_off = target_off;
            }
            if target_off < have_off {
                error_exit(pname, 5, "Cannot seek backwards.");
            }
            while have_off < target_off {
                write_all_or_die(fpo, &[0], pname);
                have_off += 1;
            }
        }

        let mut c_cur = Some(c_in);
        if hextype == HEX_NORMAL || hextype == HEX_POSTSCRIPT {
            if n2 >= 0 && n1 >= 0 {
                let byte = (((n2 << 4) | n1) & 0xff) as u8;
                write_all_or_die(fpo, &[byte], pname);
                have_off += 1;
                want_off += 1;
                n1 = -1;
                if hextype == HEX_NORMAL {
                    p += 1;
                    if p >= cols {
                        c_cur = skip_to_eol(fpi, c_cur, pname);
                    }
                }
            } else if n1 < 0 && n2 < 0 && n3 < 0 {
                c_cur = skip_to_eol(fpi, c_cur, pname);
            }
        } else if bcnt == 8 {
            write_all_or_die(fpo, &[b], pname);
            have_off += 1;
            want_off += 1;
            b = 0;
            bcnt = 0;
            p += 1;
            if p >= cols {
                c_cur = skip_to_eol(fpi, c_cur, pname);
            }
        }

        if c_cur == Some(b'\n') {
            if hextype == HEX_NORMAL || hextype == HEX_BITS {
                want_off = 0;
            }
            p = cols;
            ign_garb = true;
        } else if c_cur.is_none() {
            break;
        }
    }

    if let Err(ref e) = fpo.flush() {
        perror_exit(pname, 3, e);
    }
    fpo.finish_end();
    0
}

fn format_colored_line(l: &[u8], colors: Option<&[u8]>, out: &mut Vec<u8>) {
    out.clear();
    if let Some(cols) = colors {
        if !l.is_empty() {
            if cols[0] != 0 {
                out.extend_from_slice(&[b'\x1b', b'[', b'1', b';', b'3', cols[0], b'm']);
            }
            out.push(l[0]);
            for i in 1..l.len() {
                if cols[i] != cols[i - 1] {
                    if cols[i - 1] != 0 {
                        out.extend_from_slice(b"\x1b[0m");
                    }
                    if cols[i] != 0 {
                        out.extend_from_slice(&[b'\x1b', b'[', b'1', b';', b'3', cols[i], b'm']);
                    }
                }
                out.push(l[i]);
            }
            if cols.get(l.len()).copied().unwrap_or(0) != 0 {
                out.extend_from_slice(b"\x1b[0m");
            }
        }
    } else {
        out.extend_from_slice(l);
    }
}

struct XxdLineState {
    z: Vec<u8>,
    z_colors: Vec<u8>,
    zero_seen: i8,
    scratch: Vec<u8>,
}

impl XxdLineState {
    fn new() -> Self {
        Self {
            z: Vec::new(),
            z_colors: vec![0u8; LLEN_NO_COLOR + 1],
            zero_seen: 0,
            scratch: Vec::new(),
        }
    }

    fn xxdline<W: Write>(
        &mut self,
        fp: &mut W,
        l: &[u8],
        colors: Option<&[u8]>,
        nz: i32,
        pname: &str,
    ) {
        if nz == 0 && self.zero_seen == 1 {
            self.z.clear();
            self.z.extend_from_slice(l);
            if let Some(cols) = colors {
                let copy_len = self.z.len().min(cols.len());
                self.z_colors[..copy_len].copy_from_slice(&cols[..copy_len]);
                if copy_len < self.z_colors.len() {
                    self.z_colors[copy_len] = 0;
                }
            }
        }

        let first_zero = if nz == 0 {
            let prev = self.zero_seen;
            self.zero_seen = self.zero_seen.saturating_add(1);
            prev == 0
        } else {
            false
        };

        if nz != 0 || first_zero {
            if nz != 0 {
                if nz < 0 {
                    self.zero_seen -= 1;
                }
                if self.zero_seen == 2 {
                    format_colored_line(&self.z, Some(&self.z_colors), &mut self.scratch);
                    write_all_or_die(fp, &self.scratch, pname);
                }
                if self.zero_seen > 2 {
                    write_all_or_die(fp, b"*\n", pname);
                }
            }
            if nz >= 0 || self.zero_seen > 0 {
                format_colored_line(l, colors, &mut self.scratch);
                write_all_or_die(fp, &self.scratch, pname);
            }
            if nz != 0 {
                self.zero_seen = 0;
            }
        }

        if self.zero_seen == 0x7f {
            self.zero_seen = 4;
        }
    }
}

#[derive(Clone, Debug)]
struct XxdConfig {
    pname: String,
    autoskip: bool,
    hextype: u8,
    uppercase: bool,
    capitalize: bool,
    decimal_offset: bool,
    revert: bool,
    ebcdic: bool,
    cols: usize,
    octspergrp: usize,
    length: i64,
    seekoff: i64,
    relseek: bool,
    negseek: bool,
    displayoff: u64,
    varname: Option<String>,
    color: bool,
    infile: Option<String>,
    outfile: Option<String>,
}

fn parse_cli(args: &[String], default_color: bool) -> XxdConfig {
    let raw_pname = args.first().map_or("xxd", String::as_str);
    let pname = raw_pname
        .rsplit(PATH_SEP)
        .next()
        .unwrap_or(raw_pname)
        .to_string();

    let mut autoskip = false;
    let mut hextype: u8 = HEX_NORMAL;
    let mut uppercase = false;
    let mut capitalize = false;
    let mut decimal_offset = false;
    let mut revert = false;
    let mut ebcdic = false;
    let mut cols: i32 = 0;
    let mut colsgiven = false;
    let mut octspergrp: i32 = -1;
    let mut length: i64 = -1;
    let mut seekoff: i64 = 0;
    let mut relseek = true;
    let mut negseek = false;
    let mut displayoff: u64 = 0;
    let mut varname: Option<String> = None;
    let mut color = default_color;

    let mut idx = 1usize;
    while idx < args.len() {
        let arg = &args[idx];
        let pp = if arg.starts_with("--") && arg.len() > 2 {
            &arg[1..]
        } else {
            arg.as_str()
        };

        if pp.starts_with("-a") {
            autoskip = !autoskip;
        } else if pp.starts_with("-b") {
            hextype |= HEX_BITS;
        } else if pp.starts_with("-e") {
            hextype |= HEX_LITTLEENDIAN;
        } else if pp.starts_with("-u") {
            uppercase = true;
        } else if pp.starts_with("-p") {
            hextype |= HEX_POSTSCRIPT;
        } else if pp.starts_with("-i") {
            hextype |= HEX_CINCLUDE;
        } else if pp.starts_with("-C") {
            capitalize = true;
        } else if pp.starts_with("-d") {
            decimal_offset = true;
        } else if pp.starts_with("-r") {
            revert = true;
        } else if pp.starts_with("-E") {
            ebcdic = true;
        } else if pp.starts_with("-v") {
            eprintln!("{VERSION}{OSVER}");
            process::exit(0);
        } else if let Some(rest) = pp.strip_prefix("-c") {
            if rest.starts_with("apitalize") {
                capitalize = true;
            } else if !rest.is_empty() && !rest.starts_with("ols") {
                colsgiven = true;
                cols = c_strtol(rest) as i32;
            } else {
                let Some(next) = args.get(idx + 1) else {
                    exit_with_usage(&pname);
                };
                colsgiven = true;
                cols = c_strtol(next) as i32;
                idx += 1;
            }
        } else if let Some(rest) = pp.strip_prefix("-g") {
            if !rest.is_empty() && !rest.starts_with("roup") {
                octspergrp = c_strtol(rest) as i32;
            } else {
                let Some(next) = args.get(idx + 1) else {
                    exit_with_usage(&pname);
                };
                octspergrp = c_strtol(next) as i32;
                idx += 1;
            }
        } else if let Some(rest) = pp.strip_prefix("-o") {
            if !rest.is_empty() && !rest.starts_with("ffset") {
                displayoff = c_strtoul(rest);
            } else {
                let Some(next) = args.get(idx + 1) else {
                    exit_with_usage(&pname);
                };
                let s = next.strip_prefix('+').unwrap_or(next.as_str());
                if let Some(neg_s) = s.strip_prefix('-') {
                    displayoff = 0u64.wrapping_sub(c_strtoul(neg_s));
                } else {
                    displayoff = c_strtoul(s);
                }
                idx += 1;
            }
        } else if let Some(rest) = pp.strip_prefix("-s") {
            relseek = false;
            negseek = false;
            let target = if !rest.is_empty() && !rest.starts_with("kip") && !rest.starts_with("eek")
            {
                rest
            } else {
                let Some(next) = args.get(idx + 1) else {
                    exit_with_usage(&pname);
                };
                idx += 1;
                next.as_str()
            };
            let t1 = if let Some(stripped) = target.strip_prefix('+') {
                relseek = true;
                stripped
            } else {
                target
            };
            let t2 = if let Some(stripped) = t1.strip_prefix('-') {
                negseek = true;
                stripped
            } else {
                t1
            };
            seekoff = c_strtol(t2);
        } else if let Some(rest) = pp.strip_prefix("-l") {
            if !rest.is_empty() && !rest.starts_with("en") {
                length = c_strtol(rest);
            } else {
                let Some(next) = args.get(idx + 1) else {
                    exit_with_usage(&pname);
                };
                length = c_strtol(next);
                idx += 1;
            }
        } else if let Some(rest) = pp.strip_prefix("-n") {
            if !rest.is_empty() && !rest.starts_with("ame") {
                varname = Some(rest.to_string());
            } else {
                let Some(next) = args.get(idx + 1) else {
                    exit_with_usage(&pname);
                };
                varname = Some(next.clone());
                idx += 1;
            }
        } else if let Some(mut pw) = pp.strip_prefix("-R") {
            if pw.is_empty() {
                let Some(next) = args.get(idx + 1) else {
                    exit_with_usage(&pname);
                };
                pw = next.as_str();
                idx += 1;
            }
            if pw.starts_with("always") {
                let _ = enable_color();
                color = true;
            } else if pw.starts_with("never") {
                color = false;
            } else if pw.starts_with("auto") {
                color = enable_color();
            } else {
                exit_with_usage(&pname);
            }
        } else if arg == "--" {
            idx += 1;
            break;
        } else if pp.starts_with('-') && pp.len() > 1 {
            exit_with_usage(&pname);
        } else {
            break;
        }
        idx += 1;
    }

    if hextype != (HEX_CINCLUDE | HEX_BITS) && (hextype & (hextype.wrapping_sub(1))) != 0 {
        error_exit(&pname, 1, "only one of -b, -e, -u, -p, -i can be used");
    }

    if !colsgiven || (cols == 0 && hextype != HEX_POSTSCRIPT) {
        cols = match hextype {
            HEX_POSTSCRIPT => 30,
            HEX_CINCLUDE => 12,
            x if x == (HEX_CINCLUDE | HEX_BITS) || x == HEX_BITS => 6,
            _ => 16,
        };
    }

    if octspergrp < 0 {
        octspergrp = match hextype {
            x if x == (HEX_CINCLUDE | HEX_BITS) || x == HEX_BITS => 1,
            HEX_NORMAL => 2,
            HEX_LITTLEENDIAN => 4,
            _ => 0,
        };
    }

    if (hextype == HEX_POSTSCRIPT && cols < 0)
        || (hextype != HEX_POSTSCRIPT && cols < 1)
        || (matches!(hextype, HEX_NORMAL | HEX_BITS | HEX_LITTLEENDIAN) && cols as usize > COLS)
    {
        eprintln!("{pname}: invalid number of columns (max. {COLS}).");
        process::exit(1);
    }

    if octspergrp < 1 || octspergrp > cols {
        octspergrp = cols;
    } else if hextype == HEX_LITTLEENDIAN && (octspergrp & (octspergrp - 1)) != 0 {
        error_exit(
            &pname,
            1,
            "number of octets per group must be a power of 2 with -e.",
        );
    }

    let rem = &args[idx..];
    if rem.len() > 2 {
        exit_with_usage(&pname);
    }
    let infile = rem.first().filter(|s| s.as_str() != "-").cloned();
    let outfile = rem.get(1).filter(|s| s.as_str() != "-").cloned();

    XxdConfig {
        pname,
        autoskip,
        hextype,
        uppercase,
        capitalize,
        decimal_offset,
        revert,
        ebcdic,
        cols: cols as usize,
        octspergrp: octspergrp as usize,
        length,
        seekoff,
        relseek,
        negseek,
        displayoff,
        varname,
        color,
        infile,
        outfile,
    }
}

fn dump_stream<R: Read, W: Write>(
    cfg: &XxdConfig,
    fpi: &mut R,
    fpo: &mut W,
    effective_seekoff: i64,
) {
    let hexx = if cfg.uppercase { HEX_UPPER } else { HEX_LOWER };

    if (cfg.hextype & HEX_CINCLUDE) != 0 {
        let vname = cfg.varname.as_deref().or(cfg.infile.as_deref());
        if let Some(vn) = vname {
            let prefix = if vn.as_bytes().first().is_some_and(u8::is_ascii_digit) {
                "__"
            } else {
                ""
            };
            let ident: String = vn
                .bytes()
                .map(|b| {
                    if b.is_ascii_alphanumeric() {
                        let ch = b as char;
                        if cfg.capitalize {
                            ch.to_ascii_uppercase()
                        } else {
                            ch
                        }
                    } else {
                        '_'
                    }
                })
                .collect();
            write_all_or_die(
                fpo,
                format!("unsigned char {prefix}{ident}[] = {{\n").as_bytes(),
                &cfg.pname,
            );
        }

        let mut p: i64 = 0;
        while cfg.length < 0 || p < cfg.length {
            let Some(c) = read_byte(fpi, &cfg.pname) else {
                break;
            };
            let sep = if p == 0 {
                "  "
            } else if (p as usize).is_multiple_of(cfg.cols) {
                ",\n  "
            } else {
                ", "
            };
            if (cfg.hextype & HEX_BITS) != 0 {
                write_all_or_die(fpo, format!("{sep}0b{c:08b}").as_bytes(), &cfg.pname);
            } else if cfg.uppercase {
                write_all_or_die(fpo, format!("{sep}0X{c:02X}").as_bytes(), &cfg.pname);
            } else {
                write_all_or_die(fpo, format!("{sep}0x{c:02x}").as_bytes(), &cfg.pname);
            }
            p += 1;
        }

        if p > 0 {
            write_all_or_die(fpo, b"\n", &cfg.pname);
        }

        if let Some(vn) = vname {
            let prefix = if vn.as_bytes().first().is_some_and(u8::is_ascii_digit) {
                "__"
            } else {
                ""
            };
            let ident: String = vn
                .bytes()
                .map(|b| {
                    if b.is_ascii_alphanumeric() {
                        let ch = b as char;
                        if cfg.capitalize {
                            ch.to_ascii_uppercase()
                        } else {
                            ch
                        }
                    } else {
                        '_'
                    }
                })
                .collect();
            let len_suffix = if cfg.capitalize { "LEN" } else { "len" };
            write_all_or_die(
                fpo,
                format!("}};\nunsigned int {prefix}{ident}_{len_suffix} = {p};\n").as_bytes(),
                &cfg.pname,
            );
        }
        return;
    }

    if cfg.hextype == HEX_POSTSCRIPT {
        let mut p = cfg.cols;
        let mut n: i64 = 0;
        while cfg.length < 0 || n < cfg.length {
            let Some(e) = read_byte(fpi, &cfg.pname) else {
                break;
            };
            let pair = [hexx[(e >> 4) as usize], hexx[(e & 0x0f) as usize]];
            write_all_or_die(fpo, &pair, &cfg.pname);
            n += 1;
            if cfg.cols > 0 {
                p -= 1;
                if p == 0 {
                    write_all_or_die(fpo, b"\n", &cfg.pname);
                    p = cfg.cols;
                }
            }
        }
        if cfg.cols == 0 || p < cfg.cols {
            write_all_or_die(fpo, b"\n", &cfg.pname);
        }
        return;
    }

    let grplen = if cfg.hextype != HEX_BITS {
        cfg.octspergrp * 2 + 1
    } else {
        cfg.octspergrp * 8 + 1
    };

    let mut l = vec![b' '; LLEN_NO_COLOR + 1];
    let mut colors = vec![0u8; LLEN_NO_COLOR + 1];
    let mut line_state = XxdLineState::new();
    let mut p: usize = 0;
    let mut n: i64 = 0;
    let mut nonzero: i32 = 0;
    let mut addrlen: usize = 9;
    let mut c: usize = 0;

    while cfg.length < 0 || n < cfg.length {
        let Some(mut e) = read_byte(fpi, &cfg.pname) else {
            break;
        };
        if p == 0 {
            let off_val = (n as u64)
                .wrapping_add(effective_seekoff as u64)
                .wrapping_add(cfg.displayoff);
            let addr_str = if cfg.decimal_offset {
                format!("{off_val:08}:")
            } else {
                format!("{off_val:08x}:")
            };
            addrlen = addr_str.len();
            l[..addrlen].copy_from_slice(addr_str.as_bytes());
            l[addrlen..LLEN_NO_COLOR].fill(b' ');
        }

        let x = if cfg.hextype == HEX_LITTLEENDIAN {
            p ^ (cfg.octspergrp - 1)
        } else {
            p
        };
        c = addrlen + 1 + (grplen * x) / cfg.octspergrp;
        let mut cur_color = 0u8;
        if cfg.hextype == HEX_NORMAL || cfg.hextype == HEX_LITTLEENDIAN {
            if cfg.color {
                cur_color = get_color_char(e, cfg.ebcdic);
                colors[c] = cur_color;
                colors[c + 1] = cur_color;
            }
            l[c] = hexx[(e >> 4) as usize];
            c += 1;
            l[c] = hexx[(e & 0x0f) as usize];
        } else {
            for i in (0..8).rev() {
                l[c] = if (e & (1 << i)) != 0 { b'1' } else { b'0' };
                c += 1;
            }
        }
        if e != 0 {
            nonzero += 1;
        }

        c = if cfg.hextype == HEX_LITTLEENDIAN {
            grplen * (cfg.cols + cfg.octspergrp - 1) / cfg.octspergrp - 1
        } else {
            (grplen * cfg.cols - 1) / cfg.octspergrp
        };

        if cfg.ebcdic {
            e = if e < 64 {
                b'.'
            } else {
                ETOA64[(e - 64) as usize]
            };
        }

        c += addrlen + 3 + p;
        if cfg.color {
            colors[c] = cur_color;
        }
        l[c] = if (32..127).contains(&e) { e } else { b'.' };
        c += 1;
        n += 1;
        p += 1;
        if p == cfg.cols {
            l[c] = b'\n';
            c += 1;
            line_state.xxdline(
                fpo,
                &l[..c],
                cfg.color.then_some(&colors[..=c]),
                if cfg.autoskip { nonzero } else { 1 },
                &cfg.pname,
            );
            colors[..c].fill(0);
            nonzero = 0;
            p = 0;
        }
    }

    if p > 0 {
        l[c] = b'\n';
        let line_len = c + 1;
        if cfg.color {
            let mut x = p;
            let mut p_fill = p;
            if cfg.hextype == HEX_LITTLEENDIAN {
                let mut fill = cfg.octspergrp - (p_fill % cfg.octspergrp);
                if fill == cfg.octspergrp {
                    fill = 0;
                }
                let c_start =
                    addrlen + 1 + (grplen * (x - (cfg.octspergrp - fill))) / cfg.octspergrp;
                for c_fill in c_start..(c_start + fill) {
                    colors[c_fill] = COLOR_RED;
                    l[c_fill] = b' ';
                    x += 1;
                    p_fill += 1;
                }
            }
            if cfg.hextype != HEX_BITS {
                let c_start = addrlen
                    + 1
                    + (grplen * x) / cfg.octspergrp
                    + (cfg.cols - p_fill)
                    + (cfg.cols - p_fill) / cfg.octspergrp;
                for c_fill in c_start..(c_start + (cfg.cols - p_fill)) {
                    colors[c_fill] = COLOR_RED;
                    l[c_fill] = b' ';
                }
            }
            colors[line_len] = 0;
            line_state.xxdline(
                fpo,
                &l[..line_len],
                Some(&colors[..=line_len]),
                1,
                &cfg.pname,
            );
        } else {
            line_state.xxdline(fpo, &l[..line_len], None, 1, &cfg.pname);
        }
    } else if cfg.autoskip {
        line_state.xxdline(
            fpo,
            &l[..c],
            cfg.color.then_some(&colors[..=c]),
            -1,
            &cfg.pname,
        );
    }
}

fn main() {
    let no_color = env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    let default_color = if no_color { false } else { enable_color() };
    let args: Vec<String> = env::args().collect();
    let cfg = parse_cli(&args, default_color);

    let mut infile_opt = match &cfg.infile {
        Some(path) => match File::open(path) {
            Ok(f) => Some(f),
            Err(e) => perror_exit(&cfg.pname, 2, &e),
        },
        None => None,
    };

    let mut outfile_opt = match &cfg.outfile {
        Some(path) => match OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(!cfg.revert)
            .open(path)
        {
            Ok(mut f) => {
                let _ = f.seek(SeekFrom::Start(0));
                Some(BufWriter::new(f))
            }
            Err(e) => perror_exit(&cfg.pname, 3, &e),
        },
        None => None,
    };

    if cfg.revert {
        if !matches!(cfg.hextype, HEX_NORMAL | HEX_POSTSCRIPT | HEX_BITS) {
            error_exit(&cfg.pname, -1, "Cannot revert this type of hexdump");
        }
        let base_off = if cfg.negseek {
            -cfg.seekoff
        } else {
            cfg.seekoff
        };
        if let Some(ref mut f) = infile_opt {
            let _ = f.seek(SeekFrom::Start(0));
            let mut reader = BufReader::new(f);
            let code = match outfile_opt.take() {
                Some(fw) => {
                    let mut sink = FileSink(fw);
                    huntype(
                        &mut reader,
                        &mut sink,
                        cfg.cols,
                        cfg.hextype,
                        base_off,
                        &cfg.pname,
                    )
                }
                None => {
                    let stdout = io::stdout();
                    let mut sink = StdoutSink(BufWriter::new(stdout.lock()));
                    huntype(
                        &mut reader,
                        &mut sink,
                        cfg.cols,
                        cfg.hextype,
                        base_off,
                        &cfg.pname,
                    )
                }
            };
            process::exit(code);
        } else {
            let stdin = io::stdin();
            let mut reader = BufReader::new(stdin.lock());
            let code = match outfile_opt.take() {
                Some(fw) => {
                    let mut sink = FileSink(fw);
                    huntype(
                        &mut reader,
                        &mut sink,
                        cfg.cols,
                        cfg.hextype,
                        base_off,
                        &cfg.pname,
                    )
                }
                None => {
                    let stdout = io::stdout();
                    let mut sink = StdoutSink(BufWriter::new(stdout.lock()));
                    huntype(
                        &mut reader,
                        &mut sink,
                        cfg.cols,
                        cfg.hextype,
                        base_off,
                        &cfg.pname,
                    )
                }
            };
            process::exit(code);
        }
    }

    let mut effective_seekoff = cfg.seekoff;
    if cfg.seekoff != 0 || cfg.negseek || !cfg.relseek {
        if let Some(ref mut f) = infile_opt {
            let pos_res = if cfg.relseek {
                f.seek(SeekFrom::Current(if cfg.negseek {
                    -cfg.seekoff
                } else {
                    cfg.seekoff
                }))
            } else if cfg.negseek {
                f.seek(SeekFrom::End(-cfg.seekoff))
            } else {
                f.seek(SeekFrom::Start(cfg.seekoff as u64))
            };
            match pos_res {
                Ok(pos) => {
                    effective_seekoff = pos as i64;
                }
                Err(_) => {
                    if cfg.negseek {
                        error_exit(&cfg.pname, 4, "Cannot seek.");
                    }
                    let mut reader = BufReader::new(f);
                    for _ in 0..cfg.seekoff {
                        if read_byte(&mut reader, &cfg.pname).is_none() {
                            error_exit(&cfg.pname, 4, "Cannot seek.");
                        }
                    }
                }
            }
        } else {
            if cfg.negseek {
                error_exit(&cfg.pname, 4, "Cannot seek.");
            }
            let stdin = io::stdin();
            let mut lock = stdin.lock();
            for _ in 0..cfg.seekoff {
                if read_byte(&mut lock, &cfg.pname).is_none() {
                    error_exit(&cfg.pname, 4, "Cannot seek.");
                }
            }
        }
    }

    match (infile_opt.as_mut(), outfile_opt.as_mut()) {
        (Some(fi), Some(fo)) => {
            let mut reader = BufReader::new(fi);
            dump_stream(&cfg, &mut reader, fo, effective_seekoff);
            if let Err(ref e) = fo.flush() {
                perror_exit(&cfg.pname, 3, e);
            }
        }
        (Some(fi), None) => {
            let mut reader = BufReader::new(fi);
            let stdout = io::stdout();
            let mut writer = BufWriter::new(stdout.lock());
            dump_stream(&cfg, &mut reader, &mut writer, effective_seekoff);
            if let Err(ref e) = writer.flush() {
                perror_exit(&cfg.pname, 3, e);
            }
        }
        (None, Some(fo)) => {
            let stdin = io::stdin();
            let mut reader = BufReader::new(stdin.lock());
            dump_stream(&cfg, &mut reader, fo, effective_seekoff);
            if let Err(ref e) = fo.flush() {
                perror_exit(&cfg.pname, 3, e);
            }
        }
        (None, None) => {
            let stdin = io::stdin();
            let mut reader = BufReader::new(stdin.lock());
            let stdout = io::stdout();
            let mut writer = BufWriter::new(stdout.lock());
            dump_stream(&cfg, &mut reader, &mut writer, effective_seekoff);
            if let Err(ref e) = writer.flush() {
                perror_exit(&cfg.pname, 3, e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_dump(args: &[&str], input: &[u8]) -> Vec<u8> {
        let owned: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let cfg = parse_cli(&owned, false);
        let mut out = Vec::new();
        let mut reader = input;
        dump_stream(&cfg, &mut reader, &mut out, cfg.seekoff);
        out
    }

    fn run_revert(args: &[&str], input: &[u8]) -> Vec<u8> {
        let owned: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let cfg = parse_cli(&owned, false);
        let mut out = Vec::new();
        {
            let mut sink = StdoutSink(&mut out);
            let mut reader = input;
            let base_off = if cfg.negseek {
                -cfg.seekoff
            } else {
                cfg.seekoff
            };
            huntype(
                &mut reader,
                &mut sink,
                cfg.cols,
                cfg.hextype,
                base_off,
                &cfg.pname,
            );
        }
        out
    }

    #[test]
    fn normal_hexdump_and_revert_roundtrip() {
        let input = b"Hello, Neovim xxd in Rust!\n";
        let hex = run_dump(&["xxd"], input);
        let decoded = run_revert(&["xxd", "-r"], &hex);
        assert_eq!(decoded, input);
    }

    #[test]
    fn postscript_and_binary_and_cinclude_modes_work() {
        let ps = run_dump(&["xxd", "-p"], b"ABC");
        assert_eq!(ps, b"414243\n");
        let ps_rev = run_revert(&["xxd", "-r", "-p"], &ps);
        assert_eq!(ps_rev, b"ABC");

        let bits = run_dump(&["xxd", "-b"], b"A");
        assert_eq!(
            String::from_utf8(bits.clone()).unwrap(),
            "00000000: 01000001                                               A\n"
        );
        let bits_rev = run_revert(&["xxd", "-r", "-b"], &bits);
        assert_eq!(bits_rev, b"A");

        let cinc = run_dump(&["xxd", "-i", "-n", "123foo"], b"\x01\xfe");
        assert_eq!(
            String::from_utf8(cinc).unwrap(),
            "unsigned char __123foo[] = {\n  0x01, 0xfe\n};\nunsigned int __123foo_len = 2;\n"
        );
    }

    #[test]
    fn autoskip_compresses_consecutive_zero_lines() {
        let input = [0u8; 64];
        let out = run_dump(&["xxd", "-a"], &input);
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("\n*\n"));
        assert!(s.starts_with("00000000:"));
        assert!(s.contains("\n00000030:"));
    }
}
