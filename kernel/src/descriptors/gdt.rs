//! Global Descriptor Table (GDT) and Task State Segment (TSS) setup.
//!
//! Builds per-CPU GDTs with kernel/user code and data segments, a TSS
//! providing a Double Fault IST and privilege stack table, and loads the
//! selectors into CS/DS/SS.

use crate::cpu::CPUInfo;
use crate::memory::PAGE_SIZE;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};

use x86_64::instructions::segmentation::{CS, DS, SS, Segment};

use x86_64::instructions::tables::load_tss;
use x86_64::structures::tss::TaskStateSegment;

use crate::memory::KMemory;

pub const DF_IST_INDEX: usize = 0;
const STACK_SIZE: usize = 20 * 1024;
const STACK_PAGE_COUNT: usize = STACK_SIZE.div_ceil(PAGE_SIZE);

#[derive(Clone)]
pub struct Selectors {
    pub kernel_code: SegmentSelector,
    pub kernel_data: SegmentSelector,
    pub user_code: SegmentSelector,
    pub user_data: SegmentSelector,
    pub user_code_intel: SegmentSelector,
    pub tss: SegmentSelector,
}

/// # SAFETY: The TSS reference must be guranteed to live for 'static
fn init_gdt(tss_ref: &TaskStateSegment) -> (GlobalDescriptorTable<32>, Selectors) {
    let mut gdt = GlobalDescriptorTable::<32>::empty();

    let kernel_code = gdt.append(Descriptor::kernel_code_segment());
    let kernel_data = gdt.append(Descriptor::kernel_data_segment());
    let tss_sel = unsafe { gdt.append(Descriptor::tss_segment_unchecked(tss_ref)) };
    let user_code = gdt.append(Descriptor::user_code_segment());
    let user_data = gdt.append(Descriptor::user_data_segment());
    let user_code_intel = gdt.append(Descriptor::user_code_segment());

    (
        gdt,
        Selectors {
            kernel_code,
            kernel_data,
            user_code,
            user_data,
            user_code_intel,
            tss: tss_sel,
        },
    )
}

/// Initialize the GDT for a given CPU
pub fn init(cpu: &'static mut CPUInfo) {
    let df_stack = KMemory::alloc_pages(STACK_PAGE_COUNT);
    let priv_stack = KMemory::alloc_pages(STACK_PAGE_COUNT);

    let (df_top, priv_top) = (
        df_stack.get_virt_addr() + STACK_SIZE as u64,
        priv_stack.get_virt_addr() + STACK_SIZE as u64,
    );

    cpu.tss = TaskStateSegment::new();
    cpu.tss.interrupt_stack_table[DF_IST_INDEX] = df_top;
    cpu.tss.privilege_stack_table[0] = priv_top;

    let (gdt, selectors) = init_gdt(&cpu.tss);
    cpu.gdt = gdt;
    cpu.selectors = selectors.clone();
    cpu.kernel_rsp = priv_stack.get_virt_addr().as_u64() + STACK_SIZE as u64;

    cpu.gdt.load();

    unsafe {
        CS::set_reg(selectors.kernel_code);
        DS::set_reg(selectors.kernel_data);
        SS::set_reg(selectors.kernel_data);
        load_tss(selectors.tss);
    }

    df_stack.leak();
    priv_stack.leak();
}
