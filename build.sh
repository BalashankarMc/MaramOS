#!/bin/bash

set -e 

BUILD_DIR="$(pwd)"
KERNEL_DIR="${BUILD_DIR}/kernel"
BOOTLOADER_FILES="${BUILD_DIR}/limine-files"

ISO_DIR="${BUILD_DIR}/iso-root"

rm -f maram.iso
rm -rf $ISO_DIR
mkdir -p "${ISO_DIR}/boot"
mkdir -p "${ISO_DIR}/EFI/BOOT"

cp "${BOOTLOADER_FILES}/limine.conf" "${ISO_DIR}/boot/"
cp "${BOOTLOADER_FILES}/limine-uefi-cd.bin" "${ISO_DIR}/boot/"
cp "${BOOTLOADER_FILES}/BOOTX64.EFI" "${ISO_DIR}/EFI/BOOT/"

cd "$KERNEL_DIR"
cargo build --release
cp ./target/x86_64-unknown-none/release/kernel "${ISO_DIR}/boot/kernel.elf"

cd "$BUILD_DIR"
xorriso -as mkisofs \
    -e boot/limine-uefi-cd.bin \
    -no-emul-boot \
    -isohybrid-gpt-basdat \
    "${ISO_DIR}" -o "maram.iso"