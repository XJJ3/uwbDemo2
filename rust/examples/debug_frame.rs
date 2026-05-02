use uwb_delay_test::{nlink, SYNC0, SYNC1};

fn main() {
    let handshake = nlink::build_user_frame1(nlink::Role::Slave as u8, 0xFF, &[]);
    println!("Handshake frame ({} bytes): {:02X?}", handshake.len(), handshake);
    
    let mut payload = [0u8; 16];
    payload[0] = SYNC0;
    payload[1] = SYNC1;
    payload[2] = 0;  // seq low
    payload[3] = 0;  // seq high
    payload[4..12].copy_from_slice(&123456789u64.to_le_bytes());
    
    let data_frame = nlink::build_user_frame1(nlink::Role::Slave as u8, 0xFF, &payload);
    println!("Data frame ({} bytes): {:02X?}", data_frame.len(), data_frame);
}
