pub mod device;
pub mod nlink;
pub mod interactive;

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
