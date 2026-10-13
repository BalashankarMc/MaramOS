use core::sync::atomic::AtomicBool;

use alloc::vec::Vec;
use limine::mp::MpInfo;
use x86_64::VirtAddr;
use x86_64::instructions::tables::load_tss;
use x86_64::registers::model_specific::GsBase;

use crate::acpi::{init_timer_ap, lapic_init};
use crate::memory::{PhysPage, Stack};
use crate::scheduling::{Scheduler, Task, idle_task};
use crate::{InterruptMutex, KResult, MP_RESPONSE};
use crate::descriptors::{IDT, new_gdt};

use super::{CPUData, CPUS};

pub fn init_bsp() -> KResult<()> {
    let mut cpus = MP_RESPONSE.cpus().to_vec();
    let mut res = Vec::with_capacity(cpus.len());

    // Safety: The returned values never drop. Safe
    let (gdt, mut tss, selectors) = unsafe { new_gdt()? };
    let priv_stack = Stack::new()?;
    tss.privilege_stack_table[0] = priv_stack.top();
    
    let (cs, ds) = (selectors.kernel_code.0, selectors.kernel_data.0);

    unsafe {
        // Safety: GDT load. Safe
        gdt.load_unsafe();
        // Safety: TSS load. Safe
        load_tss(selectors.tss);
    }

    selectors.load();
    crate::descriptors::init();

    let idle = Task::manual(|| idle_task(), PhysPage::new(4)?, cs, ds);
    let bsp_info = CPUData {
        id: 0,
        lapic_id: u64::from(MP_RESPONSE.bsp_lapic_id),
        gdt,
        tss,
        selectors,
        scheduler: InterruptMutex::new(Scheduler::new(idle.clone())),
        idle,
        priv_stack,
        ipi_ack: AtomicBool::new(false),
        tlb_addresses: InterruptMutex::new(Vec::new())
    };

    // Unwrap: BSP must exist
    let pos_bsp = cpus.iter().position(|e| e.lapic_id == MP_RESPONSE.bsp_lapic_id).unwrap();
    cpus.swap_remove(pos_bsp);

    res.push(bsp_info);

    for (id, cpu) in (1..).zip(cpus) {
        // Safety: The returned values never drop. Safe
        let (gdt, mut tss, selectors) = unsafe { new_gdt()? };
        let priv_stack = Stack::new()?;
        tss.privilege_stack_table[0] = priv_stack.top();

        let (cs, ds) = (selectors.kernel_code.0, selectors.kernel_data.0);

        let idle = Task::manual(|| idle_task(), PhysPage::new(4)?, cs, ds);
        let info = CPUData {
            id,
            lapic_id: u64::from(cpu.lapic_id),
            gdt,
            tss,
            selectors,
            scheduler: InterruptMutex::new(Scheduler::new(idle.clone())),
            idle,
            priv_stack,
            ipi_ack: AtomicBool::new(false),
            tlb_addresses: InterruptMutex::new(Vec::new())
        };

        res.push(info);
    }

    CPUS.init(res);

    super::ipi::init()?;
    GsBase::write(VirtAddr::from_ptr(&raw const CPUS[0]));
    Ok(())
}

pub unsafe extern "C" fn ap_entry(mp: &MpInfo) -> ! {
    let ptr = mp.extra_argument() as *mut CPUData;
    GsBase::write(VirtAddr::from_ptr(ptr));

    // Safety: Dereference of a safe pointer. Safe
    // We need to cast &CPUData to &mut CPUData. This is safe
    // as a cpu will only write to it's own CPUData once at init.
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

    super::ONLINE_APS.fetch_add(1, core::sync::atomic::Ordering::SeqCst);

    x86_64::instructions::interrupts::enable();

    idle_task();
}