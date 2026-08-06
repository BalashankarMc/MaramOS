use x86_64::{instructions::tables::load_tss, registers::segmentation::{CS, DS, ES, SS, Segment}, structures::{gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector}, tss::TaskStateSegment}};

use crate::{helpers::LateInit, memory::{PAGE_SIZE, PhysPage}};

const STACK_SIZE: usize = 20 * 0x400;
const STACK_PAGES: usize = STACK_SIZE.div_ceil(PAGE_SIZE);

static GDT: LateInit<GlobalDescriptorTable<32>> = LateInit::new();
static SELECTORS: LateInit<Selectors> = LateInit::new();
static TSS: LateInit<TaskStateSegment> = LateInit::new();

pub struct Selectors {
    kernel_code: SegmentSelector,
    kernel_data: SegmentSelector,
    tss: SegmentSelector
}

impl Selectors {
    fn load(&self) {
        unsafe {
            CS::set_reg(self.kernel_code);
            DS::set_reg(self.kernel_data);
            ES::set_reg(self.kernel_data);
            SS::set_reg(self.kernel_data);
        }
    }
}

pub fn init() -> Result<(), ()> {
    let mut gdt = GlobalDescriptorTable::<32>::empty();
    let kcode = gdt.append(Descriptor::kernel_code_segment());
    let kdata = gdt.append(Descriptor::kernel_data_segment());

    let mut tss = TaskStateSegment::new();
    tss.interrupt_stack_table[0] = PhysPage::new(STACK_PAGES).ok_or(())?.leak().0 + STACK_SIZE as u64;
    tss.interrupt_stack_table[1] = PhysPage::new(STACK_PAGES).ok_or(())?.leak().0 + STACK_SIZE as u64;
    tss.interrupt_stack_table[2] = PhysPage::new(STACK_PAGES).ok_or(())?.leak().0 + STACK_SIZE as u64;
    
    let tss_ref = TSS.init(tss);
    let tss_sel = gdt.append(Descriptor::tss_segment(tss_ref));

    GDT.init(gdt).load();

    // Safety: We know that the provided TSS is safe.
    unsafe { load_tss(tss_sel) };

    SELECTORS.init(Selectors { kernel_code: kcode, kernel_data: kdata, tss: tss_sel }).load();
    Ok(())
}