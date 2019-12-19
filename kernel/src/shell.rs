//! Kernel shell

// use crate::fs::ROOT_INODE;
use crate::process::*;
use alloc::string::String;
use alloc::vec::Vec;

pub fn add_simple_kernel_shell() {
    processor().manager().add(Thread::new_kernel(shell, 0));
}
pub extern "C" fn shell(_arg: usize) -> ! {

    println!("a sad shell can not get_line");
    loop{}
}
