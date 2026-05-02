pub mod nlink;

pub const SYNC0: u8 = 0xA5;
pub const SYNC1: u8 = 0x5A;
pub const PAYLOAD_HEADER_SIZE: usize = 12;

pub struct Stats {
    pub count: u64,
    pub lost: u64,
    pub min_ms: f64,
    pub max_ms: f64,
    pub sum_ms: f64,
    pub sum_sq_ms: f64,
}

impl Stats {
    #[inline]
    pub fn new() -> Self {
        Self {
            count: 0,
            lost: 0,
            min_ms: f64::MAX,
            max_ms: 0.0,
            sum_ms: 0.0,
            sum_sq_ms: 0.0,
        }
    }

    #[inline]
    pub fn add(&mut self, delay_ms: f64) {
        self.count += 1;
        self.sum_ms += delay_ms;
        self.sum_sq_ms += delay_ms * delay_ms;
        if delay_ms < self.min_ms {
            self.min_ms = delay_ms;
        }
        if delay_ms > self.max_ms {
            self.max_ms = delay_ms;
        }
    }

    #[inline]
    pub fn avg_ms(&self) -> f64 {
        if self.count > 0 {
            self.sum_ms / self.count as f64
        } else {
            0.0
        }
    }

    #[inline]
    pub fn stddev_ms(&self) -> f64 {
        if self.count < 2 {
            return 0.0;
        }
        let mean = self.avg_ms();
        let var = self.sum_sq_ms / self.count as f64 - mean * mean;
        if var > 0.0 { var.sqrt() } else { 0.0 }
    }
}

#[inline]
pub fn pack_payload(dst: &mut [u8], seq: u16, timestamp: f64) {
    dst[0] = SYNC0;
    dst[1] = SYNC1;
    dst[2] = (seq & 0xFF) as u8;
    dst[3] = ((seq >> 8) & 0xFF) as u8;
    dst[4..12].copy_from_slice(&timestamp.to_le_bytes());
    if dst.len() > 12 {
        dst[12..].fill(0);
    }
}

#[inline]
pub fn unpack_payload(data: &[u8]) -> Option<(u16, f64)> {
    if data.len() < PAYLOAD_HEADER_SIZE {
        return None;
    }
    if data[0] != SYNC0 || data[1] != SYNC1 {
        return None;
    }
    let seq = data[2] as u16 | ((data[3] as u16) << 8);
    let ts = f64::from_le_bytes(data[4..12].try_into().unwrap());
    Some((seq, ts))
}

pub struct RecvBuffer {
    buf: Box<[u8; 65536]>,
    len: usize,
    payload_size: usize,
}

impl RecvBuffer {
    pub fn new(payload_size: usize) -> Self {
        Self {
            buf: Box::new([0u8; 65536]),
            len: 0,
            payload_size,
        }
    }

    #[inline]
    pub fn feed(&mut self, data: &[u8], recv_ns: u64) -> SmallVec<(u16, u64), 16> {
        let mut results = SmallVec::new();
        
        if self.len + data.len() > self.buf.len() {
            self.len = 0;
        }
        self.buf[self.len..self.len + data.len()].copy_from_slice(data);
        self.len += data.len();

        while self.len >= self.payload_size {
            if self.buf[0] == SYNC0 && self.buf[1] == SYNC1 {
                if let Some((seq, _ts)) = unpack_payload(&self.buf[..self.payload_size]) {
                    results.push((seq, recv_ns));
                }
                self.buf.copy_within(self.payload_size..self.len, 0);
                self.len -= self.payload_size;
            } else {
                self.buf.copy_within(1..self.len, 0);
                self.len -= 1;
            }
        }
        results
    }
}

pub struct SmallVec<T, const N: usize> {
    data: [MaybeUninit<T>; N],
    len: usize,
}

use std::mem::MaybeUninit;

impl<T, const N: usize> SmallVec<T, N> {
    pub fn new() -> Self {
        Self {
            data: unsafe { MaybeUninit::uninit().assume_init() },
            len: 0,
        }
    }

    #[inline]
    pub fn push(&mut self, val: T) {
        if self.len < N {
            self.data[self.len].write(val);
            self.len += 1;
        }
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.data[..self.len].iter().map(|x| unsafe { x.assume_init_ref() })
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<T, const N: usize> Drop for SmallVec<T, N> {
    fn drop(&mut self) {
        for i in 0..self.len {
            unsafe { self.data[i].assume_init_drop() };
        }
    }
}

impl<T, const N: usize> IntoIterator for SmallVec<T, N> {
    type Item = T;
    type IntoIter = SmallVecIntoIter<T, N>;

    fn into_iter(self) -> Self::IntoIter {
        SmallVecIntoIter { vec: self, pos: 0 }
    }
}

pub struct SmallVecIntoIter<T, const N: usize> {
    vec: SmallVec<T, N>,
    pos: usize,
}

impl<T, const N: usize> Iterator for SmallVecIntoIter<T, N> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos < self.vec.len {
            let item = unsafe { self.vec.data[self.pos].assume_init_read() };
            self.pos += 1;
            Some(item)
        } else {
            None
        }
    }
}

impl<T, const N: usize> Drop for SmallVecIntoIter<T, N> {
    fn drop(&mut self) {
        for i in self.pos..self.vec.len {
            unsafe { self.vec.data[i].assume_init_drop() };
        }
    }
}
