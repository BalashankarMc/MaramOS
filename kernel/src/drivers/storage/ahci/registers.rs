use crate::memory::{MMIORegion, MMIORegister};

pub struct GlobalRegisters<'a> {
    pub cap: MMIORegister<'a, u32>,
    pub cap2: MMIORegister<'a, u32>,
    pub ghc: MMIORegister<'a, u32>,
    pub pi: MMIORegister<'a, u32>,
    pub bohc: MMIORegister<'a, u32>
}

impl<'a> GlobalRegisters<'a> {
    pub fn new(region: &'a MMIORegion) -> Option<Self> {
        Some(Self {
            cap: region.register(0)?,
            cap2: region.register(0x24)?,
            ghc: region.register(4)?,
            pi: region.register(0xC)?,
            bohc: region.register(0x28)?
        })
    }
}

pub struct PortRegisters<'a> {
    pub clb: MMIORegister<'a, u32>,
    pub clbu: MMIORegister<'a, u32>,
    pub fb: MMIORegister<'a, u32>,
    pub fbu: MMIORegister<'a, u32>,
    pub is: MMIORegister<'a, u32>,
    pub cmd: MMIORegister<'a, u32>,
    pub tfd: MMIORegister<'a, u32>,
    pub sig: MMIORegister<'a, u32>,
    pub ssts: MMIORegister<'a, u32>,
    pub sctl: MMIORegister<'a, u32>,
    pub serr: MMIORegister<'a, u32>,
    pub sact: MMIORegister<'a, u32>,
    pub ci: MMIORegister<'a, u32>
}

impl<'a> PortRegisters<'a> {
    pub fn new(region: &'a MMIORegion, port: u32) -> Option<Self> {
        let reg = |offset: usize| region.register(0x100 + (port as usize * 0x80) + offset);
        Some(Self {
            clb: reg(0)?,
            clbu: reg(4)?,
            fb: reg(8)?,
            fbu: reg(0xC)?,
            is: reg(0x10)?,
            cmd: reg(0x18)?,
            tfd: reg(0x20)?,
            sig: reg(0x24)?,
            ssts: reg(0x28)?,
            sctl: reg(0x2C)?,
            serr: reg(0x30)?,
            sact: reg(0x34)?,
            ci: reg(0x38)?
        })
    }
}