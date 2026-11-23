#![cfg_attr(feature = "axstd", no_std)]
#![cfg_attr(feature = "axstd", no_main)]
#![feature(asm_const)]
#![feature(riscv_ext_intrinsics)]

#[cfg(feature = "axstd")]
extern crate axstd as std;
extern crate alloc;
#[macro_use]
extern crate axlog;

mod task;
mod vcpu;
mod regs;
mod csrs;
mod sbi;
mod loader;

use vcpu::VmCpuRegisters;
use riscv::register::{scause, sstatus, stval};
use csrs::defs::hstatus;
use tock_registers::LocalRegisterCopy;
use csrs::{RiscvCsrTrait, CSR};
use vcpu::_run_guest;
use sbi::SbiMessage;
use loader::load_vm_image;
use alloc::boxed::Box;
use axhal::mem::{phys_to_virt, virt_to_phys, PhysAddr, PAGE_SIZE_4K};
use axmm::AddrSpace;
use crate::regs::GprIndex::{A0, A1};

const VM_ENTRY: usize = 0x8020_0000;
const GUEST_HART_ID: usize = 0x1234;
const GUEST_LOAD_VALUE: usize = 0x6688;

const CSR_READ_MHARTID: u32 = 0xf140_25f3; // csrr a1, mhartid
const LOAD_GUEST_VALUE: u32 = 0x0400_3503; // ld a0, 64(zero)

#[repr(C, align(16384))]
struct GuestPageTable([u8; 4 * PAGE_SIZE_4K]);

#[cfg_attr(feature = "axstd", no_mangle)]
fn main() {
    ax_println!("Hypervisor ...");

    // A new address space for vm.
    let mut uspace = AddrSpace::new_empty(0.into(), VM_ENTRY + PAGE_SIZE_4K).unwrap();

    // Load vm binary file into address space.
    if let Err(e) = load_vm_image("/sbin/skernel2", &mut uspace) {
        panic!("Cannot load app! {:?}", e);
    }

    // Setup context to prepare to enter guest mode.
    let mut ctx = VmCpuRegisters::default();
    prepare_guest_context(&mut ctx);

    // Setup pagetable for 2nd address mapping.
    // Sv39x4 的根页表占四页，原有 Sv39 的根表只占第一页。
    let mut root = Box::new(GuestPageTable([0; 4 * PAGE_SIZE_4K]));
    let root_src = phys_to_virt(uspace.page_table_root()).as_ptr();
    unsafe {
        core::ptr::copy_nonoverlapping(root_src, root.0.as_mut_ptr(), PAGE_SIZE_4K);
    }
    prepare_vm_pgtable(virt_to_phys((root.0.as_ptr() as usize).into()));

    // 客体的退出入口只处理同步异常，运行期间暂时屏蔽宿主中断。
    let host_sie: usize;
    unsafe {
        core::arch::asm!("csrrw {saved}, sie, zero", saved = out(reg) host_sie);
    }
    while !run_guest(&mut ctx, &uspace) {}

    unsafe {
        core::arch::asm!("csrw hgatp, zero");
        core::arch::riscv64::hfence_gvma_all();
        core::arch::asm!("csrw sie, {saved}", saved = in(reg) host_sie);
    }
    ax_println!("Hypervisor ok!");
}

fn prepare_vm_pgtable(ept_root: PhysAddr) {
    let hgatp = 8usize << 60 | usize::from(ept_root) >> 12;
    unsafe {
        core::arch::asm!(
            "csrw hgatp, {hgatp}",
            hgatp = in(reg) hgatp,
        );
        core::arch::riscv64::hfence_gvma_all();
    }
}

fn run_guest(ctx: &mut VmCpuRegisters, uspace: &AddrSpace) -> bool {
    unsafe {
        _run_guest(ctx);
    }

    vmexit_handler(ctx, uspace)
}

fn guest_instruction(uspace: &AddrSpace, pc: usize) -> u32 {
    let mut bytes = [0; 4];
    uspace.read(pc.into(), &mut bytes).unwrap();
    u32::from_le_bytes(bytes)
}

fn vmexit_handler(ctx: &mut VmCpuRegisters, uspace: &AddrSpace) -> bool {
    use scause::{Exception, Trap};

    let scause = scause::read();
    match scause.cause() {
        Trap::Exception(Exception::VirtualSupervisorEnvCall) => {
            let sbi_msg = SbiMessage::from_regs(ctx.guest_regs.gprs.a_regs()).ok();
            ax_println!("VmExit Reason: VSuperEcall: {:?}", sbi_msg);
            if let Some(msg) = sbi_msg {
                match msg {
                    SbiMessage::Reset(_) => {
                        let a0 = ctx.guest_regs.gprs.reg(A0);
                        let a1 = ctx.guest_regs.gprs.reg(A1);
                        ax_println!("a0 = {:#x}, a1 = {:#x}", a0, a1);
                        assert_eq!(a0, GUEST_LOAD_VALUE);
                        assert_eq!(a1, GUEST_HART_ID);
                        ax_println!("Shutdown vm normally!");
                        return true;
                    },
                    _ => panic!("Unsupported SBI call: {:?}", msg),
                }
            } else {
                panic!("bad sbi message! ");
            }
        },
        Trap::Exception(Exception::IllegalInstruction)
            if guest_instruction(uspace, ctx.guest_regs.sepc) == CSR_READ_MHARTID =>
        {
            ctx.guest_regs.gprs.set_reg(A1, GUEST_HART_ID);
        }
        Trap::Exception(Exception::LoadGuestPageFault)
            if guest_instruction(uspace, ctx.guest_regs.sepc) == LOAD_GUEST_VALUE =>
        {
            ctx.guest_regs.gprs.set_reg(A0, GUEST_LOAD_VALUE);
        }
        _ => {
            panic!(
                "Unhandled trap: {:?}, sepc: {:#x}, stval: {:#x}",
                scause.cause(),
                ctx.guest_regs.sepc,
                stval::read()
            );
        }
    }
    // 退出处理已模拟这条 4 字节指令，从下一条指令继续运行。
    ctx.guest_regs.sepc += 4;
    false
}

fn prepare_guest_context(ctx: &mut VmCpuRegisters) {
    // Set hstatus
    let mut hstatus = LocalRegisterCopy::<usize, hstatus::Register>::new(
        riscv::register::hstatus::read().bits(),
    );
    // Set Guest bit in order to return to guest mode.
    hstatus.modify(hstatus::spv::Guest);
    // Set SPVP bit in order to accessing VS-mode memory from HS-mode.
    hstatus.modify(hstatus::spvp::Supervisor);
    CSR.hstatus.write_value(hstatus.get());
    ctx.guest_regs.hstatus = hstatus.get();

    // Set sstatus in guest mode.
    let mut sstatus = sstatus::read();
    sstatus.set_spp(sstatus::SPP::Supervisor);
    ctx.guest_regs.sstatus = sstatus.bits();
    // Return to entry to start vm.
    ctx.guest_regs.sepc = VM_ENTRY;
}
