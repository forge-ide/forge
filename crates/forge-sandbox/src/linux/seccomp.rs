//! seccomp filters applied by the helper just before it execs the command.
//!
//! Both tiers deny syscalls that reach past the process's own files and network: tracing other
//! processes, the kernel keyring, BPF, io_uring (which bypasses seccomp for the operations it
//! carries), mounts and namespaces, modules, and terminal input injection. Light, which has no
//! network namespace, also denies every socket except TCP over IPv4 and IPv6 (Landlock then
//! limits which ports those may reach), so no UDP, raw, netlink, or Unix-domain sockets;
//! `socketpair` still works.

use std::collections::BTreeMap;

use seccompiler::{
    BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
    SeccompRule,
};

use crate::policy::Mode;

/// Denied outright in every tier.
const DENIED: &[libc::c_long] = &[
    libc::SYS_ptrace,
    libc::SYS_process_vm_readv,
    libc::SYS_process_vm_writev,
    libc::SYS_keyctl,
    libc::SYS_add_key,
    libc::SYS_request_key,
    libc::SYS_bpf,
    libc::SYS_perf_event_open,
    libc::SYS_io_uring_setup,
    libc::SYS_io_uring_enter,
    libc::SYS_io_uring_register,
    libc::SYS_mount,
    libc::SYS_umount2,
    libc::SYS_pivot_root,
    libc::SYS_unshare,
    libc::SYS_setns,
    libc::SYS_kexec_load,
    libc::SYS_kexec_file_load,
    libc::SYS_init_module,
    libc::SYS_finit_module,
    libc::SYS_delete_module,
    libc::SYS_swapon,
    libc::SYS_swapoff,
    libc::SYS_reboot,
    libc::SYS_acct,
    libc::SYS_userfaultfd,
    libc::SYS_open_by_handle_at,
];

/// `ioctl` requests that push input into a terminal.
const DENIED_IOCTLS: &[u64] = &[libc::TIOCSTI, libc::TIOCLINUX];

/// Apply the filter for `mode` to this thread; the exec'd command inherits it.
pub fn apply(mode: Mode) -> Result<(), String> {
    let program = build(mode)?;
    seccompiler::apply_filter(&program).map_err(|e| format!("seccomp: {e}"))
}

fn build(mode: Mode) -> Result<BpfProgram, String> {
    let fail = |e: seccompiler::BackendError| format!("seccomp: {e}");
    let cond = |arg: u8, op: SeccompCmpOp, value: u64| {
        SeccompCondition::new(arg, SeccompCmpArgLen::Dword, op, value).map_err(fail)
    };
    let rule = |conds: Vec<SeccompCondition>| SeccompRule::new(conds).map_err(fail);

    let mut rules: BTreeMap<i64, Vec<SeccompRule>> = DENIED.iter().map(|s| (*s, vec![])).collect();
    let mut ioctls = Vec::new();
    for request in DENIED_IOCTLS {
        ioctls.push(rule(vec![cond(1, SeccompCmpOp::Eq, *request)?])?);
    }
    rules.insert(libc::SYS_ioctl, ioctls);

    if mode == Mode::Light {
        let (inet, inet6) = (libc::AF_INET as u64, libc::AF_INET6 as u64);
        // Any domain but IPv4 and IPv6.
        let mut socket = vec![rule(vec![
            cond(0, SeccompCmpOp::Ne, inet)?,
            cond(0, SeccompCmpOp::Ne, inet6)?,
        ])?];
        // IPv4 and IPv6 with any type but stream; the low bits of the type hold it, the high
        // bits hold SOCK_CLOEXEC and SOCK_NONBLOCK.
        for domain in [inet, inet6] {
            for kind in 0..16u64 {
                if kind == libc::SOCK_STREAM as u64 {
                    continue;
                }
                socket.push(rule(vec![
                    cond(0, SeccompCmpOp::Eq, domain)?,
                    cond(1, SeccompCmpOp::MaskedEq(0xf), kind)?,
                ])?);
            }
        }
        rules.insert(libc::SYS_socket, socket);
    }

    let arch = std::env::consts::ARCH
        .try_into()
        .map_err(|e: seccompiler::BackendError| fail(e))?;
    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EPERM as u32),
        arch,
    )
    .map_err(fail)?;
    filter.try_into().map_err(fail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_filters_compile() {
        assert!(!build(Mode::Light).unwrap().is_empty());
        assert!(!build(Mode::Standard).unwrap().is_empty());
    }
}
