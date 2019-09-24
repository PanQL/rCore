pub mod consts;
pub mod fast_syscall;
mod handler;
mod trapframe;

pub use self::handler::*;
pub use self::trapframe::*;
use crate::memory::phys_to_virt;
use apic::*;

/// Enable interrupt response
#[inline(always)]
pub unsafe fn enable() {
    x86_64::instructions::interrupts::enable();
}

/// Disable interrupt response
#[inline(always)]
pub unsafe fn disable() {
    x86_64::instructions::interrupts::disable();
}

/// Disable interrupt response and store current interrupt-reponse state
/// return : original interrupt-reponse state
#[inline(always)]
pub unsafe fn disable_and_store() -> usize {
    let r: usize;
    asm!("pushfq; popq $0; cli" : "=r"(r) :: "memory");
    r
}

/// Set interrupt-reponse state as 'flags'
/// flags : new interrupt-response state
#[inline(always)]
pub unsafe fn restore(flags: usize) {
    asm!("pushq $0; popfq" :: "r"(flags) : "memory" "flags");
}

/// Run function f under non-interrupt condition
#[inline(always)]
pub fn no_interrupt(f: impl FnOnce()) {
    let flags = unsafe { disable_and_store() };
    f();
    unsafe { restore(flags) };
}

#[inline(always)]
pub fn enable_irq(irq: u8) {
    let mut ioapic = unsafe { IoApic::new(phys_to_virt(IOAPIC_ADDR as usize)) };
    ioapic.enable(irq, 0);
}

#[inline(always)]
pub fn ack(_irq: u8) {
    let mut lapic = unsafe { XApic::new(phys_to_virt(LAPIC_ADDR)) };
    lapic.eoi();
}
