use x86_64::{VirtAddr, instructions::tables::load_tss, registers::segmentation::{CS, DS, ES, SS, Segment}, structures::{gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector}, tss::TaskStateSegment}};

use crate::{KResult, helpers::LateInit, memory::Stack};

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

pub fn init() -> KResult<()> {
    let mut gdt = GlobalDescriptorTable::<GDT_SIZE>::empty();
    let kernel_code = gdt.append(Descriptor::kernel_code_segment());
    let kernel_data = gdt.append(Descriptor::kernel_data_segment());
    let user_data = gdt.append(Descriptor::user_data_segment());
    let user_code = gdt.append(Descriptor::user_code_segment());

    let ist0_stack = Stack::new()?;
    let ist1_stack = Stack::new()?;
    let ist2_stack = Stack::new()?;

    let mut tss = TaskStateSegment::new();
    tss.interrupt_stack_table[0] = ist0_stack.top();
    tss.interrupt_stack_table[1] = ist1_stack.top();
    tss.interrupt_stack_table[2] = ist2_stack.top();
    
    ist0_stack.leak();
    ist1_stack.leak();
    ist2_stack.leak();

    let tss_ref = TSS.init(tss);
    let tss = gdt.append(Descriptor::tss_segment(tss_ref));

    GDT.init(gdt).load();

    // Safety: We know that the provided TSS is safe.
    unsafe { load_tss(tss) };

    SELECTORS.init(Selectors { kernel_code, kernel_data, user_code, user_data, tss }).load();
    Ok(())
}

// The TSS has no competing references, so, even though this is technically UB, it is safe
#[allow(invalid_reference_casting)] // Intentional to update the RSP0
pub fn update_rsp0(sp: VirtAddr) {
    let addr = core::ptr::from_ref(TSS.get()) as usize as *mut TaskStateSegment;
    // Safety: Deref of a safe pointer
    let reference = unsafe { &mut *addr };
    reference.privilege_stack_table[0] = sp;
}