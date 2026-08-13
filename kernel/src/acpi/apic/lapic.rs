//! Local APIC Driver.
//! 
//! Only supports X2APIC mode

use x86_64::registers::model_specific::{ApicBase, ApicBaseFlags, Msr};

use crate::{LateInit, errors::ACPIError, helpers::wait_while};

const X2APIC_SVR: u32 = 0x80F;
const X2APIC_ERROR_STATUS: u32 = 0x808;
const X2APIC_EOI: u32 = 0x80B;
const X2APIC_APICID: u32 = 0x802;
const X2APIC_TIMER_LVT: u32 = 0x832;
const X2APIC_TIMER_ICR: u32 = 0x838;
const X2APIC_TIMER_CCR: u32 = 0x839;
const X2APIC_TIMER_DCR: u32 = 0x83E;
const X2APIC_SELF_IPI: u32 = 0x83F;

pub static LAPIC_TIMER_VECTOR: LateInit<u8> = LateInit::new();
pub static LAPIC_TIMER_TICKS: LateInit<u32> = LateInit::new();

pub fn init() -> Result<(), ACPIError> {
    let (frame, flags) = ApicBase::read();

    let is_x2apic_supported = ((core::arch::x86_64::__cpuid(1).ecx >> 21) & 1) == 1;

    if !is_x2apic_supported { return Err(ACPIError::NoX2APIC) }

    // Safety: Just an MSR write
    unsafe { ApicBase::write(frame, flags | ApicBaseFlags::LAPIC_ENABLE | ApicBaseFlags::X2APIC_ENABLE) };

    // Set spurious vector to 0xFF with APIC software enable (bit 8)
    // Safety: Just writing to a MSR (Model-Specific Register). Safe
    unsafe { Msr::new(X2APIC_SVR).write(0xFF | (1 << 8)) };
    
    // Clear error status
    // Safety: Same as before
    unsafe { Msr::new(X2APIC_ERROR_STATUS).write(0); }

    Ok(())
}

pub fn eoi() {
    // Safety: Just an MSR write
    unsafe { Msr::new(X2APIC_EOI).write(0); }
}

pub fn id() -> u32 {
    // Safety: Just another MSR write
    unsafe { Msr::new(X2APIC_APICID).read() as u32 }
}

pub fn send_self_ipi(vector: u8) {
    // Safety: Just another MSR write
    unsafe { Msr::new(X2APIC_SELF_IPI).write(u64::from(vector)); }
}

pub fn init_timer(vector: u8) {
    // Initialize MSRs
    let mut timer_icr = Msr::new(X2APIC_TIMER_ICR);

    // Safety: Just more MSR writes
    unsafe {
        Msr::new(X2APIC_TIMER_DCR).write(3);
        timer_icr.write(u64::from(u32::MAX));
    }

    let start = crate::acpi::passed_nanos();
    wait_while(|| { crate::acpi::passed_nanos() - start < 10_000_000 });

    // Safety: Just more MSR writes
    let elapsed = u32::MAX - unsafe { Msr::new(X2APIC_TIMER_CCR).read() } as u32;

    let lvt_entry = u32::from(vector) | (1 << 17);

    // Safety: More MSR writes
    unsafe {
        Msr::new(X2APIC_TIMER_LVT).write(u64::from(lvt_entry));
        timer_icr.write(u64::from(elapsed));
    }

    LAPIC_TIMER_VECTOR.init(vector);
    LAPIC_TIMER_TICKS.init(elapsed);
}

pub fn init_ap() {
    let (frame, flags) = ApicBase::read();

    if !flags.contains(ApicBaseFlags::LAPIC_ENABLE) {
        // Safety: More MSR Writes
        unsafe { ApicBase::write(frame, flags | ApicBaseFlags::LAPIC_ENABLE); }
    }

    // Safety: Same as last
    unsafe {
        Msr::new(X2APIC_SVR).write(0xFF | (1 << 8));
        Msr::new(X2APIC_ERROR_STATUS).write(0);
    }
}

pub fn init_timer_ap() {
    let vector = *LAPIC_TIMER_VECTOR;
    let ticks = *LAPIC_TIMER_TICKS;

    // Safety: Just more MSR writes
    unsafe {
        Msr::new(X2APIC_TIMER_DCR).write(3);
        Msr::new(X2APIC_TIMER_LVT).write(u64::from(vector) | (1 << 17));
        Msr::new(X2APIC_TIMER_ICR).write(u64::from(ticks));
    }
}