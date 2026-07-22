TARGET = x86_64-unknown-none
ELF_DIR = target/$(TARGET)/release
ELF = $(ELF_DIR)/kernel

.PHONY: all release test test-unit test-gdb clean

all release:
	cargo build --release
	cp $(ELF) ../kernel.elf

test:
	cargo build --release --features integration-test

test-unit:
	cargo test --lib --target x86_64-unknown-linux-gnu

test-gdb: clean
	cargo build --release --features integration-test \
		--config 'profile.release.debug=2' \
		--config 'profile.release.strip="none"'

clean:
	cargo clean
	rm -f ../kernel.elf