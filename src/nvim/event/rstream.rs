//! Readable libuv streams with callback-owned read batches.

use crate::event::r#loop::{
    Loop, uv_fs_get_result, uv_fs_read, uv_fs_req_cleanup, uv_fs_t, uv_handle_get_loop,
    uv_handle_get_type, uv_handle_t, uv_idle_start, uv_idle_stop, uv_read_start, uv_read_stop,
};
use crate::event::multiqueue::{Event, multiqueue_put_event};
use crate::event::stream::{
    Stream, StreamCbData, StreamState, UV_TTY, stream_init, stream_may_close, uv_idle_t,
    uv_stream_t,
};
use crate::event::wstream::uv_buf_t;
use crate::memory::ARENA_BLOCK_SIZE;
use crate::os::fs::{UV_ENOBUFS, UV_EOF};

#[derive(Clone)]
pub struct RStream {
    pub s: Stream,
}

impl std::ops::Deref for RStream {
    type Target = StreamState;

    fn deref(&self) -> &StreamState {
        &self.s
    }
}

impl RStream {
    pub fn new() -> Self {
        Self { s: Stream::new() }
    }

    pub fn read_snapshot(&self) -> Vec<u8> {
        self.s.with_read(|buffer| buffer.as_slice().to_vec())
    }

    pub fn did_eof_volatile(&self) -> bool {
        self.did_eof.get()
    }
}

pub struct ReadEvent {
    stream: RStream,
    dispatched: bool,
}

impl Drop for ReadEvent {
    fn drop(&mut self) {
        if !self.dispatched {
            self.stream.pending_read.set(false);
        }
        self.stream.s.finish_request();
    }
}

fn rstream_space(stream: &RStream) -> usize {
    stream
        .s
        .with_read_mut(|buffer| buffer.writable(ARENA_BLOCK_SIZE).len())
}

pub fn rstream_init_fd(loop_: &Loop, stream: &RStream, fd: i32) {
    unsafe { stream_init(Some(loop_), &stream.s, fd, false) };
    rstream_init(stream);
}

pub fn rstream_init(stream: &RStream) {
    stream.num_bytes.set(0);
    let old = stream.s.with_read_mut(std::mem::take);
    drop(old);
    stream.is_rstream.set(true);
}

pub fn rstream_start_inner(stream: &RStream) {
    assert!(stream.uv.is_active());
    unsafe {
        if stream.has_uvstream.get() {
            uv_read_start(stream.uv.as_ptr(), Some(alloc_cb), Some(read_cb));
        } else {
            uv_idle_start(stream.uv.as_ptr(), Some(fread_idle_cb));
        }
    }
}

pub fn rstream_start(stream: &RStream, data: StreamCbData) {
    stream.cb_data.set(data);
    stream.want_read.set(true);
    if !stream.paused_full.get() {
        rstream_start_inner(stream);
    }
}

pub fn rstream_stop_inner(stream: &RStream) {
    if !stream.uv.is_active() {
        return;
    }
    unsafe {
        if stream.has_uvstream.get() {
            uv_read_stop(stream.uv.as_ptr());
        } else {
            uv_idle_stop(stream.uv.as_ptr());
        }
    }
}

pub fn rstream_stop(stream: &RStream) {
    rstream_stop_inner(stream);
    stream.want_read.set(false);
}

unsafe extern "C" fn alloc_cb(handle: *mut uv_handle_t, _suggested: usize, buf: *mut uv_buf_t) {
    let stream = unsafe { Stream::from_callback(handle) };
    let writable =
        stream.with_read_mut(|buffer| uv_buf_t::from_mut_slice(buffer.writable(ARENA_BLOCK_SIZE)));
    unsafe { buf.write(writable) };
}

unsafe extern "C" fn read_cb(handle: *mut uv_stream_t, count: isize, _buf: *const uv_buf_t) {
    let stream = RStream {
        s: unsafe { Stream::from_callback(handle) },
    };
    if count <= 0 {
        if count == UV_ENOBUFS as isize || count == 0 {
            return;
        }
        if count != UV_EOF as isize || unsafe { uv_handle_get_type(handle) } != UV_TTY {
            unsafe { uv_read_stop(handle) };
        }
        invoke_read_cb(&stream, true);
        return;
    }
    let count = count as usize;
    stream.num_bytes.set(stream.num_bytes.get() + count);
    stream.s.with_read_mut(|buffer| buffer.commit(count));
    invoke_read_cb(&stream, false);
}

unsafe extern "C" fn fread_idle_cb(handle: *mut uv_idle_t) {
    let stream = RStream {
        s: unsafe { Stream::from_callback(handle) },
    };
    let mut request = [0u64; 128];
    let request = request.as_mut_ptr().cast::<uv_fs_t>();
    let writable = stream
        .s
        .with_read_mut(|buffer| uv_buf_t::from_mut_slice(buffer.writable(ARENA_BLOCK_SIZE)));
    let count = unsafe {
        uv_fs_read(
            uv_handle_get_loop(handle),
            request,
            stream.fd.get(),
            &writable,
            1,
            stream.fpos.get(),
            None,
        );
        let result = uv_fs_get_result(request);
        uv_fs_req_cleanup(request);
        result
    };
    if count <= 0 {
        unsafe { uv_idle_stop(handle) };
        invoke_read_cb(&stream, true);
        return;
    }
    stream
        .s
        .with_read_mut(|buffer| buffer.commit(count as usize));
    stream.fpos.set(stream.fpos.get() + count as i64);
    invoke_read_cb(&stream, false);
}

pub fn read_event(mut event: ReadEvent) {
    event.dispatched = true;
    let stream = &event.stream;
    stream.pending_read.set(false);
    let batch = stream.s.with_read_mut(|buffer| buffer.take());
    let consumed = stream
        .cb_data
        .get()
        .call_read_cb(stream.fd.get(), batch.as_slice(), stream.did_eof.get())
        .unwrap_or(0);
    stream
        .s
        .with_read_mut(|buffer| buffer.restore(batch, consumed));
    resume_if_writable(stream);
}

pub fn rstream_available(stream: &RStream) -> usize {
    stream.s.with_read(|buffer| buffer.available())
}

fn resume_if_writable(stream: &RStream) {
    // Console shutdown drains output before beginning native handle closure.
    if stream.uv.is_active()
        && stream.want_read.get()
        && stream.paused_full.get()
        && rstream_space(stream) != 0
    {
        assert!(stream.cb_data.get().has_read_cb());
        stream.paused_full.set(false);
        rstream_start_inner(stream);
    }
}

pub fn rstream_consume(stream: &RStream, consumed: usize) {
    stream.s.with_read_mut(|buffer| buffer.consume(consumed));
    resume_if_writable(stream);
}

fn invoke_read_cb(stream: &RStream, eof: bool) {
    stream.did_eof.set(stream.did_eof.get() || eof);
    if rstream_space(stream) == 0 {
        rstream_stop_inner(stream);
        stream.paused_full.set(true);
    }
    if stream.pending_read.replace(true) {
        return;
    }
    stream.pending_reqs.set(stream.pending_reqs.get() + 1);
    let event = ReadEvent {
        stream: stream.clone(),
        dispatched: false,
    };
    let events = stream.events.get();
    if !events.is_null() {
        multiqueue_put_event(events, Event::RStreamRead(event));
    } else {
        read_event(event);
    }
}

pub fn rstream_may_close(stream: &RStream) {
    stream_may_close(&stream.s);
}
