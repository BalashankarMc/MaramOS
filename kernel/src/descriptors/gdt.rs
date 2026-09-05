use x86_64::{instructions::tables::load_tss, registers::segmentation::{CS, DS, ES, SS, Segment}, structures::{gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector}, tss::TaskStateSegment}};

use crate::{KernelResult, helpers::LateInit, memory::{PAGE_SIZE, PhysPage}};

const STACK_SIZE: usize = 20 * 0x400; // 20 MiB
const STACK_PAGES: usize = STACK_SIZE.div_ceil(PAGE_SIZE); // 5 Pages
const GDT_SIZE: usize = 64;

static GDT: LateInit<GlobalDescriptorTable<GDT_SIZE>> = LateInit::new();
pub static SELECTORS: LateInit<Selectors> = LateInit::new();
static TSS: LateInit<TaskStateSegment> = LateInit::new();

pub struct Selectors {
    pub kernel_code: SegmentSelector,
    pub kernel_data: SegmentSelector,
    pub user_code: SegmentSelector,
    pub user_data: SegmentSelector,
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

pub fn init() -> KernelResult<()> {
    let mut gdt = GlobalDescriptorTable::<GDT_SIZE>::empty();
    let kernel_code = gdt.append(Descriptor::kernel_code_segment());
    let kernel_data = gdt.append(Descriptor::kernel_data_segment());
    let user_code = gdt.append(Descriptor::user_code_segment());
    let user_data = gdt.append(Descriptor::user_data_segment());

    let stack_pages = PhysPage::new(STACK_PAGES * 5)?;
    let stack_addr = stack_pages.address();
    let stack_size = STACK_SIZE as u64;

    let stack0_base = stack_addr + stack_size;
    let stack1_base = stack0_base + stack_size;
    let stack2_base = stack1_base + stack_size;
    let stack3_base = stack2_base + stack_size;
    let priv_stack_base = stack3_base + stack_size;

    stack_pages.leak();

    let mut tss = TaskStateSegment::new();
    tss.interrupt_stack_table[0] = stack0_base;
    tss.interrupt_stack_table[1] = stack1_base;
    tss.interrupt_stack_table[2] = stack2_base;
    tss.interrupt_stack_table[3] = stack3_base;

    tss.privilege_stack_table[0] = priv_stack_base;
    
    let tss_ref = TSS.init(tss);
    let tss = gdt.append(Descriptor::tss_segment(tss_ref));

    GDT.init(gdt).load();

    // Safety: We know that the provided TSS is safe.
    unsafe { load_tss(tss) };

    SELECTORS.init(Selectors { kernel_code, kernel_data, user_code, user_data, tss }).load();
    Ok(())
}