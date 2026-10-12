use alloc::boxed::Box;
use x86_64::registers::segmentation::{CS, DS, ES, SS, Segment};
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;

use crate::{KResult, memory::Stack};

const GDT_SIZE: usize = 64;

#[derive(Debug, Clone, Copy)]
pub struct Selectors {
    pub kernel_code: SegmentSelector,
    pub kernel_data: SegmentSelector,
    pub user_code: SegmentSelector,
    pub user_data: SegmentSelector,
    pub tss: SegmentSelector
}

impl Selectors {
    pub fn load(&self) {
        unsafe {
            CS::set_reg(self.kernel_code);
            DS::set_reg(self.kernel_data);
            ES::set_reg(self.kernel_data);
            SS::set_reg(self.kernel_data);
        }
    }
}

/// Initialize a GDT.
/// 
/// Safety: The return values must not be dropped
pub unsafe fn new_gdt() -> KResult<(Box<GlobalDescriptorTable   <GDT_SIZE>>, Box<TaskStateSegment>, Selectors)> {
    let mut gdt = Box::new(GlobalDescriptorTable::empty());
    let kernel_code = gdt.append(Descriptor::kernel_code_segment());
    let kernel_data = gdt.append(Descriptor::kernel_data_segment());
    let user_data = gdt.append(Descriptor::user_data_segment());
    let user_code = gdt.append(Descriptor::user_code_segment());

    let ist0_stack = Stack::new()?;
    let ist1_stack = Stack::new()?;
    let ist2_stack = Stack::new()?;

    let mut tss = Box::new(TaskStateSegment::new());
    tss.interrupt_stack_table[0] = ist0_stack.top();
    tss.interrupt_stack_table[1] = ist1_stack.top();
    tss.interrupt_stack_table[2] = ist2_stack.top();
    
    ist0_stack.leak();
    ist1_stack.leak();
    ist2_stack.leak();

    let tss_sel = unsafe { gdt.append(Descriptor::tss_segment_unchecked(&raw const *tss)) };
    let selectors = Selectors { kernel_code, kernel_data, user_code, user_data, tss: tss_sel };    

    Ok((gdt, tss, selectors))
}
