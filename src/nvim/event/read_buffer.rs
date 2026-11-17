//! Read batches own their bytes while callbacks may reenter the stream.

#[derive(Default)]
pub struct ReadBuffer {
    bytes: Vec<u8>,
    end: usize,
}

pub struct ReadBatch {
    bytes: Vec<u8>,
    end: usize,
}

impl ReadBuffer {
    pub fn available(&self) -> usize {
        self.end
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.end]
    }

    pub fn writable(&mut self, capacity: usize) -> &mut [u8] {
        if self.bytes.is_empty() {
            self.bytes.resize(capacity, 0);
        }
        &mut self.bytes[self.end..]
    }

    pub fn commit(&mut self, count: usize) {
        assert!(count <= self.bytes.len() - self.end);
        self.end += count;
    }

    pub fn consume(&mut self, count: usize) {
        assert!(count <= self.end);
        self.bytes.copy_within(count..self.end, 0);
        self.end -= count;
    }

    pub fn take(&mut self) -> ReadBatch {
        ReadBatch {
            bytes: std::mem::take(&mut self.bytes),
            end: std::mem::take(&mut self.end),
        }
    }

    pub fn restore(&mut self, mut batch: ReadBatch, consumed: usize) {
        assert!(consumed <= batch.end);
        let remaining = batch.end - consumed;
        batch.bytes.copy_within(consumed..batch.end, 0);
        let needed = remaining + self.end;
        if batch.bytes.len() < needed {
            batch.bytes.resize(needed, 0);
        }
        batch.bytes[remaining..needed].copy_from_slice(self.as_slice());
        self.bytes = batch.bytes;
        self.end = needed;
    }
}

impl ReadBatch {
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn append(buffer: &mut ReadBuffer, bytes: &[u8]) {
        buffer.writable(32)[..bytes.len()].copy_from_slice(bytes);
        buffer.commit(bytes.len());
    }

    #[test]
    fn reentrant_reads_do_not_overwrite_the_active_batch() {
        let mut buffer = ReadBuffer::default();
        append(&mut buffer, b"first");
        let first = buffer.take();
        append(&mut buffer, b"second");
        let second = buffer.take();
        append(&mut buffer, b"third");
        assert_eq!(first.as_slice(), b"first");
        assert_eq!(second.as_slice(), b"second");
        buffer.restore(second, 2);
        buffer.restore(first, 3);
        assert_eq!(buffer.as_slice(), b"stcondthird");
    }

    #[test]
    fn ordinary_dispatch_reuses_the_read_allocation() {
        let mut buffer = ReadBuffer::default();
        append(&mut buffer, b"payload");
        let address = buffer.as_slice().as_ptr();
        let batch = buffer.take();
        buffer.restore(batch, 7);
        append(&mut buffer, b"next");
        assert_eq!(buffer.as_slice(), b"next");
        assert_eq!(buffer.as_slice().as_ptr(), address);
    }
}
