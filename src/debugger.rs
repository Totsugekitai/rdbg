use std::fmt::Debug;

#[allow(unused)]
use crate::{
    breakpoint::BreakpointManager,
    command::Command,
    debug_info::{self, TdbDebugInfo},
    dump, mem, register,
    syscall::{get_regs, SyscallInfo, SyscallStack},
};
use nix::{sys::ptrace, unistd::Pid};

#[derive(Debug)]
pub struct DebuggerInfo {
    pub syscall_stack: SyscallStack,
    pub breakpoint_manager: BreakpointManager,
    pub debug_info: TdbDebugInfo,
    pub prev_command: Option<crate::command::Command>,
    pub watch_list: Vec<(Watchable, u64)>,
    pub step_flag: bool,
}

impl DebuggerInfo {
    pub fn set_watchpoint(&mut self, watchpoint: Watchable, init_value: u64) {
        self.watch_list.push((watchpoint, init_value));
    }
}

#[derive(Debug)]
pub enum Watchable {
    Address(mem::Address),
    Register(register::Register),
}

pub fn debugger_main(child: Pid, filename: &str) {
    if let Err(e) = ptrace::attach(child) {
        panic!("ptrace::attach failed, errno: {e}");
    }

    // init
    crate::signal::init(child);
    let mut syscall_stack = SyscallStack::new();
    let breakpoint_manager = BreakpointManager::new(child);
    let (debug_info, status) = TdbDebugInfo::init(filename, child, &mut syscall_stack);
    let mut debugger_info = DebuggerInfo {
        syscall_stack,
        breakpoint_manager,
        debug_info,
        watch_list: Vec::new(),
        prev_command: None,
        step_flag: false,
    };

    loop {
        let command = match Command::read(&debugger_info) {
            Ok(command) => command,
            Err(e) => {
                println!("{e}");
                continue;
            }
        };

        Command::exec(command, &mut debugger_info, status).unwrap();
    }
}

pub fn catch_syscall(pid: Pid, syscall_stack: &mut SyscallStack) {
    let syscall_info = SyscallInfo::from_regs(&get_regs(pid));

    if let Some(top) = syscall_stack.top() {
        // syscallの入口だった場合
        if top.number() != syscall_info.number() {
            syscall_stack.push(syscall_info);
        }
        // syscallの出口だった場合
        else if syscall_stack.pop().is_none() {
            panic!("syscall count failed");
        }
    } else {
        syscall_stack.push(syscall_info);
    }

    if let Err(e) = ptrace::syscall(pid, None) {
        panic!("ptrace::syscall failed: errno = {:?}", e);
    }
}
