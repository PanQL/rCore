pub mod serial;

use super::BootInfo;

pub fn init(boot_info: &BootInfo) {
    serial::init();
}