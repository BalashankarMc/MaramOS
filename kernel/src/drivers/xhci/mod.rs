use crate::drivers::pci::PCIDevice;

pub fn init_xhci(device: &dyn PCIDevice) -> Option<()> {
    device.enable_bus_master();
    device.enable_mmio();

    

    None
}