pub const HEADER_WRITE: u8 = 0x54;
pub const HEADER_READ: u8 = 0x55;
pub const MARK_USER_FRAME1: u8 = 0xF1;

#[repr(u8)]
pub enum Role {
    Node = 0,
    Anchor = 1,
    Tag = 2,
    Console = 3,
    Master = 4,
    Slave = 5,
}

pub fn checksum(data: &[u8]) -> u8 {
    let mut sum: u32 = 0;
    for &b in data {
        sum += b as u32;
    }
    (sum & 0xFF) as u8
}

pub fn build_user_frame1(remote_role: u8, remote_id: u8, payload: &[u8]) -> Vec<u8> {
    let total = 11 + payload.len();
    let mut buf = vec![0u8; total];
    buf[0] = HEADER_WRITE;
    buf[1] = MARK_USER_FRAME1;
    buf[2..6].fill(0xFF);
    buf[6] = remote_role;
    buf[7] = remote_id;
    buf[8] = (payload.len() & 0xFF) as u8;
    buf[9] = ((payload.len() >> 8) & 0xFF) as u8;
    buf[10..10 + payload.len()].copy_from_slice(payload);
    buf[total - 1] = checksum(&buf[..total - 1]);
    buf
}

pub fn build_broadcast_frame(payload: &[u8]) -> Vec<u8> {
    build_user_frame1(Role::Node as u8, 0xFF, payload)
}

pub fn build_unicast_frame(slave_id: u8, payload: &[u8]) -> Vec<u8> {
    build_user_frame1(Role::Slave as u8, slave_id, payload)
}
