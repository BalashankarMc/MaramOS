use x86_64::{registers::segmentation::{CS, DS, ES, SS, Segment}, structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector}};

use crate::{helpers::LateInit, memory::PAGE_SIZE};

const STACK_SIZE: usize = 20 * 0x400;
const STACK_PAGES: usize = STACK_SIZE.div_ceil(PAGE_SIZE);

static GDT: LateInit<GlobalDescriptorTable<32>> = LateInit::new();
static SELECTORS: LateInit<Selectors> = LateInit::new();

pub struct Selectors {
    kernel_code: SegmentSelector,
    kernel_data: SegmentSelector
}

impl Selectors {
    fn init(&self) {
        unsafe {
            CS::set_reg(self.kernel_code);
            DS::set_reg(self.kernel_data);
            ES::set_reg(self.kernel_data);
            SS::set_reg(self.kernel_data);
        }
    }
}

pub fn init() {
    let mut gdt = GlobalDescriptorTable::<32>::empty();
    let kcode = gdt.append(Descriptor::kernel_code_segment());
    let kdata = gdt.append(Descriptor::kernel_data_segment());
    GDT.init(gdt).load();

    SELECTORS.init(Selectors { kernel_code: kcode, kernel_data: kdata }).init();
}