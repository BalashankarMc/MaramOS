//! Local APIC driver.
//!
//! Provides xAPIC/MMIO and x2APIC/MSR register access for EOI, self-IPI,
//! timer calibration, and periodic timer setup. Calibrates the APIC timer
//! by spin-waiting on the HPET for a 10 ms interval.

use crate::{library::LateInit, memory::KMemory};

use core::arch::asm;
use x86_64::{
    VirtAddr,
    registers::model_specific::{ApicBase, ApicBaseFlags},
};

const X2APIC_TIMER_LVT: u32 = 0x832;
const X2APIC_TIMER_ICR: u32 = 0x838;
const X2APIC_TIMER_CCR: u32 = 0x839;
const X2APIC_TIMER_DCR: u32 = 0x83E;

const TIMER_LVT_OFFSET: u64 = 0x320;
const TIMER_DCR_OFFSET: u64 = 0x3E0;
const TIMER_ICR_OFFSET: u64 = 0x380;
const TIMER_CCR_OFFSET: u64 = 0x390;
pub(crate) const SPURIOUS_VECTOR_REG: u64 = 0xF0;
pub(crate) const ERROR_STATUS_REG: u64 = 0x80;
pub(crate) const ICR_LOW_REG: u64 = 0x300;
pub(crate) const ICR_HIGH_REG: u64 = 0x310;
pub(crate) const X2APIC_ICR_MSR: u32 = 0x83F;
const X2APIC_APICID: u32 = 0x802;

pub(crate) fn is_x2apic() -> bool {
    ApicBase::read().1.contains(ApicBaseFlags::X2APIC_ENABLE)
}

fn write_timer_dcr(val: u32) {
    if is_x2apic() {
        unsafe { asm!("wrmsr", in("ecx") X2APIC_TIMER_DCR, in("eax") val, in("edx") 0u32) };
    } else {
        let regs = *LAPIC_REGS.get();
        unsafe {
            (regs + TIMER_DCR_OFFSET)
                .as_mut_ptr::<u32>()
                .write_volatile(val);
        }
    }
}

fn write_timer_icr(val: u32) {
    if is_x2apic() {
        unsafe { asm!("wrmsr", in("ecx") X2APIC_TIMER_ICR, in("eax") val, in("edx") 0u32) };
    } else {
        let regs = *LAPIC_REGS.get();
        unsafe {
            (regs + TIMER_ICR_OFFSET)
                .as_mut_ptr::<u32>()
                .write_volatile(val);
        }
    }
}

fn read_timer_ccr() -> u32 {
    if is_x2apic() {
        let (val, _): (u32, u32);
        unsafe { asm!("rdmsr", in("ecx") X2APIC_TIMER_CCR, out("eax") val, out("edx") _) };
        val
    } else {
        let regs = *LAPIC_REGS.get();
        unsafe { (regs + TIMER_CCR_OFFSET).as_ptr::<u32>().read_volatile() }
    }
}

fn write_timer_lvt(val: u32) {
    if is_x2apic() {
        unsafe { asm!("wrmsr", in("ecx") X2APIC_TIMER_LVT, in("eax") val, in("edx") 0u32) };
    } else {
        let regs = *LAPIC_REGS.get();
        unsafe {
            (regs + TIMER_LVT_OFFSET)
                .as_mut_ptr::<u32>()
                .write_volatile(val);
        }
    }
}

pub static LAPIC_REGS: LateInit<VirtAddr> = LateInit::new();
pub static LAPIC_TIMER_VECTOR: LateInit<u8> = LateInit::new();
pub static LAPIC_TIMER_TICKS: LateInit<u32> = LateInit::new();

pub fn init() {
    let (frame, flags) = ApicBase::read();
    if !flags.contains(ApicBaseFlags::LAPIC_ENABLE) {
        unsafe { ApicBase::write(frame, flags | ApicBaseFlags::LAPIC_ENABLE) };
    }

    let regs = KMemory::map_mmio(frame.start_address(), 1);

    let svr = unsafe { (regs + SPURIOUS_VECTOR_REG).as_ptr::<u32>().read_volatile() };
    unsafe {
        (regs + SPURIOUS_VECTOR_REG)
            .as_mut_ptr::<u32>()
            .write_volatile(svr | (1 << 8) | 0xFF);
    }
    unsafe {
        (regs + ERROR_STATUS_REG)
            .as_mut_ptr::<u32>()
            .write_volatile(0);
    }

    LAPIC_REGS.init(regs);
}

pub fn eoi() {
    let regs = *LAPIC_REGS.get();
    unsafe {
        (regs + 0xB0).as_mut_ptr::<u32>().write_volatile(0);
    } // 0xB0 is EOI register
}

pub fn id() -> u32 {
    if is_x2apic() {
        let (low, _): (u32, u32);
        unsafe { asm!("rdmsr", in("ecx") X2APIC_APICID, out("eax") low, out("edx") _) };
        low
    } else {
        let regs = *LAPIC_REGS.get();
        unsafe { (regs + 0x20).as_ptr::<u32>().read_volatile() >> 24 } // 0x20 is ID register
    }
}

pub fn send_self_ipi(vector: u8) {
    if is_x2apic() {
        unsafe { asm!("wrmsr", in("ecx") X2APIC_ICR_MSR, in("eax") vector as u32, in("edx") 0) };
    } else {
        let regs = *LAPIC_REGS.get();
        unsafe {
            let icr = vector as u32 | (1 << 18); // bits 18:19 = 01 (self)
            (regs + ICR_HIGH_REG).as_mut_ptr::<u32>().write_volatile(0);
            (regs + ICR_LOW_REG).as_mut_ptr::<u32>().write_volatile(icr);
        }
    }
}

pub fn init_timer(vector: u8) {
    write_timer_dcr(0x3);
    write_timer_icr(u32::MAX);

    let start = crate::acpi::passed_nanos();
    while crate::acpi::passed_nanos() - start < 10_000_000 {
        core::hint::spin_loop();
    }

    let elapsed = u32::MAX - read_timer_ccr();

    let lvt_entry = vector as u32 | (1 << 17);
    write_timer_lvt(lvt_entry);
    write_timer_icr(elapsed);

    LAPIC_TIMER_VECTOR.init(vector);
    LAPIC_TIMER_TICKS.init(elapsed);
}

pub fn init_timer_ap() {
    let vector = *LAPIC_TIMER_VECTOR.get();
    let ticks = *LAPIC_TIMER_TICKS.get();

    write_timer_dcr(0x3);
    let lvt_entry = vector as u32 | (1 << 17);
    write_timer_lvt(lvt_entry);
    write_timer_icr(ticks);
}

pub fn init_ap() {
    let (frame, flags) = ApicBase::read();
    if !flags.contains(ApicBaseFlags::LAPIC_ENABLE) {
        unsafe { ApicBase::write(frame, flags | ApicBaseFlags::LAPIC_ENABLE) };
    }
    let regs = *LAPIC_REGS.get(); // use existing BSP mapping
    let svr = unsafe { (regs + SPURIOUS_VECTOR_REG).as_ptr::<u32>().read_volatile() };
    unsafe {
        (regs + SPURIOUS_VECTOR_REG)
            .as_mut_ptr::<u32>()
            .write_volatile(svr | (1 << 8) | 0xFF);
    }
    unsafe {
        (regs + ERROR_STATUS_REG)
            .as_mut_ptr::<u32>()
            .write_volatile(0);
    }
}
