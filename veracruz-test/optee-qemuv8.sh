#!/bin/bash

cd ./optee-qemuv8-3.4.0 && /work/rust-optee-trustzone-sdk/optee-qemuv8-3.4.0/qemu/aarch64-softmmu/qemu-system-aarch64 \
    -nodefaults \
    -nographic \
    -serial stdio -serial file:/tmp/serial.log \
    -smp 2 \
    -machine virt,secure=on -cpu cortex-a57 \
    -d unimp -semihosting-config enable=on,target=native \
    -m 1057 \
    -initrd ./rootfs.cpio.gz \
    -append 'console=ttyAMA0,38400 keep_bootcon root=/dev/vda2' \
    -kernel ./Image -no-acpi \
    -fsdev local,id=fsdev0,path=$(pwd)/../shared,security_model=none \
    -device virtio-9p-device,fsdev=fsdev0,mount_tag=host \
    -netdev user,id=vmnic \
    -device virtio-net-device,netdev=vmnic

    #-bios /work/rust-optee-trustzone-sdk/optee-qemuv8-3.4.0/out/bin/bl1.bin \
