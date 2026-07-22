//! Application Processor (AP) startup.
//!
//! Brings up all APs listed in the MP tables, assigning each a [`CPUInfo`]
//! block and a scheduler, then sending them to their idle loop.

use x86_64::{VirtAddr, registers::model_specific::GsBase};

use crate::{acpi, memory::KMemory, syscalls};

use super::{CPU_PAGE, CPUInfo};

/// Bootstrap all APs enumerated in the MP data.
///
/// For each AP (skipping the BSP), a [`CPUInfo`] is written into the
/// pre-allocated page area, the AP is started via [`MpInfo::bootstrap`],
/// and it runs [`ap_start`] which sets up descriptors, the local APIC,
/// and a kernel task before entering the idle loop.
pub fn init_aps() {
    let resp = &crate::boot::requests::MP_DATA;
    let cpus = resp.cpus();

    let mut i = 1;
    for cpu in cpus {
        if cpu.lapic_id == crate::acpi::lapic_id() { continue } // Skip BSP

        crate::scheduling::init_scheduler();
        let addr = CPU_PAGE.get_virt_addr().as_mut_ptr::<CPUInfo>();
        let info = CPUInfo {
            id: i,
            lapic_id: cpu.lapic_id,
            cr3_save: KMemory::kernel_l4().start_address().as_u64(),
            ..unsafe { core::mem::zeroed() }
        };

        unsafe { addr.add(i as usize).write(info) };
        cpu.bootstrap(ap_start, unsafe { addr.add(i as usize) } as u64);

        i += 1;
    }
}

/// Entry point for APs — sets up per-CPU state and enters the scheduler.
///
/// # Safety
/// Called by the bootloader's MP wake code. `info.extra_argument()` must point
/// to a valid, zeroed [`CPUInfo`] written by [`init_aps`].
unsafe extern "C" fn ap_start(info: &limine::mp::MpInfo) -> ! {
    x86_64::instructions::interrupts::disable();
    let info_ptr = info.extra_argument() as *mut CPUInfo;

    crate::descriptors::gdt::init(unsafe { &mut *info_ptr });
    crate::descriptors::interrupts::load_idt_ap();
    GsBase::write(VirtAddr::new(info_ptr as u64));
    crate::fpu::init();
    syscalls::init();
    acpi::lapic_init();
    acpi::lapic_init_timer_ap();

    x86_64::instructions::interrupts::enable();

    crate::halt_loop()
}
