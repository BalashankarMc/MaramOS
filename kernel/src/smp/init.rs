use alloc::vec::Vec;
use limine::mp::MpInfo;
use x86_64::instructions::tables::load_tss;

use crate::acpi::{init_timer_ap, lapic_init};
use crate::memory::PhysPage;
use crate::scheduling::{Scheduler, Task, idle_task};
use crate::{InterruptMutex, KResult, MP_RESPONSE, halt_loop};
use crate::descriptors::{IDT, new_gdt};

use super::{CPUData, CPUS, current_cpu};

pub fn init_bsp() -> KResult<()> {
    let mut cpus = MP_RESPONSE.cpus().to_vec();
    let mut res = Vec::with_capacity(cpus.len());

    // Safety: The returned values never drop. Safe
    let (gdt, tss, selectors) = unsafe { new_gdt()? };
    let (cs, ds) = (selectors.kernel_code.0, selectors.kernel_data.0);

    unsafe {
        // Safety: GDT load. Safe
        gdt.load_unsafe();
        // Safety: TSS load. Safe
        load_tss(selectors.tss);
    }

    selectors.load();
    crate::descriptors::init();

    let idle = Task::manual(idle_task, PhysPage::new(4)?, cs, ds);
    let bsp_info = CPUData {
        id: 0,
        lapic_id: u64::from(MP_RESPONSE.bsp_lapic_id),
        gdt,
        tss,
        selectors,
        scheduler: InterruptMutex::new(Scheduler::new(idle.clone())),
        idle
    };

    // Unwrap: BSP must exist
    let pos_bsp = cpus.iter().position(|e| e.lapic_id == MP_RESPONSE.bsp_lapic_id).unwrap();
    cpus.swap_remove(pos_bsp);

    res.push(bsp_info);

    for (id, cpu) in (1..).zip(cpus) {
        // Safety: The returned values never drop. Safe
        let (gdt, tss, selectors) = unsafe { new_gdt()? };

        let (cs, ds) = (selectors.kernel_code.0, selectors.kernel_data.0);

        let idle = Task::manual(idle_task, PhysPage::new(4)?, cs, ds);
        let info = CPUData {
            id,
            lapic_id: u64::from(cpu.lapic_id),
            gdt,
            tss,
            selectors,
            scheduler: InterruptMutex::new(Scheduler::new(idle.clone())),
            idle
        };

        res.push(info);
    }

    CPUS.init(res);
    Ok(())
}

pub unsafe extern "C" fn ap_entry(_mp: &MpInfo) -> ! {
    let ptr = core::ptr::from_ref(current_cpu()) as u64 as *mut CPUData;

    // Safety: Dereference of a safe pointer. Safe
    // We need to cast &CPUData to &mut CPUData. This is safe
    // as a cpu will only write to it's own CPUData once at init.
    #[allow(invalid_reference_casting)] 
    let data = unsafe { &mut *ptr };

    unsafe {
        // Safety: GDT load. Safe.
        data.gdt.load_unsafe();
        // Safety: TSS load. Safe
        load_tss(data.selectors.tss);
    }

    data.selectors.load();
    IDT.load();

    lapic_init();
    init_timer_ap();
    x86_64::instructions::interrupts::enable();

    halt_loop()
}