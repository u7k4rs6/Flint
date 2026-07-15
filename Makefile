BOOTIMAGE := target/x86_64-flint/debug/bootimage-flint.bin
INT_LOG := target/qemu-int.log

# Single-command debug launch (Doc 4 section 6): builds the bootimage, then
# starts QEMU halted at the reset vector (-s -S) with its gdb stub on the
# usual port 1234 and interrupt/reset logging (-d int,cpu_reset) enabled --
# exactly the invocation SUMMARY.md/DECISIONS.md already document as the
# project's own debug workflow for the finicky milestones (the context
# switch, the ring-3/syscall boundary), just no longer hand-typed. Attach
# with `gdb -ex "target remote :1234"` in another shell once this prints
# QEMU's startup banner and waits.
.PHONY: debug bootimage
debug: bootimage
	qemu-system-x86_64 \
		-drive format=raw,file=$(BOOTIMAGE) \
		-serial stdio \
		-display none \
		-s -S \
		-d int,cpu_reset \
		-D $(INT_LOG)

# Always invoked, never skipped by Make's own file-timestamp tracking --
# `cargo bootimage` is already incremental and fast when nothing changed,
# the same property `cargo build`/`run`/`test` already have, so `make debug`
# stays correct even after an edit without needing Make to understand
# Cargo's own dependency graph.
bootimage:
	cargo bootimage
