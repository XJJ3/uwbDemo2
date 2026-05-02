use uwb_delay_test::device::{self, Role};

fn main() {
    println!("Scanning all devices...\n");
    
    for role in [Role::Master, Role::Slave, Role::Node, Role::Anchor, Role::Tag, Role::Console] {
        let devices = device::scan_devices(921600, role);
        if !devices.is_empty() {
            println!("{:?} devices:", role);
            for d in &devices {
                println!("  {} - ID: {}", d.port, d.id);
            }
            println!();
        }
    }
}
