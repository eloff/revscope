use std::io;

#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::net::SocketAddr;

#[cfg(target_os = "linux")]
use fxhash::FxHashSet;

#[cfg(target_os = "linux")]
const ESRCH: i32 = 3;
#[cfg(target_os = "linux")]
const SIGTERM: i32 = 15;

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

#[cfg(target_os = "linux")]
pub fn terminate_processes_listening_on_addr(listen_addr: &str) -> io::Result<Vec<u32>> {
    let addr: SocketAddr = listen_addr
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    terminate_processes_listening_on_port(addr.port())
}

#[cfg(not(target_os = "linux"))]
pub fn terminate_processes_listening_on_addr(_listen_addr: &str) -> io::Result<Vec<u32>> {
    Ok(Vec::new())
}

#[cfg(target_os = "linux")]
pub fn terminate_processes_listening_on_port(port: u16) -> io::Result<Vec<u32>> {
    let current_pid = std::process::id();
    let pids = get_pids_listening_on_port(port)
        .into_iter()
        .filter(|pid| *pid != current_pid)
        .collect::<Vec<_>>();

    if pids.is_empty() {
        return Ok(Vec::new());
    }

    for pid in &pids {
        send_sigterm(*pid)?;
    }

    Ok(pids)
}

#[cfg(not(target_os = "linux"))]
pub fn terminate_processes_listening_on_port(_port: u16) -> io::Result<Vec<u32>> {
    Ok(Vec::new())
}

/// Returns a list of PIDs listening on the specified TCP port.
/// Returns a Vec because SO_REUSEPORT allows multiple processes to bind to the same port.
#[cfg(target_os = "linux")]
pub fn get_pids_listening_on_port(port: u16) -> Vec<u32> {
    let mut inodes = FxHashSet::default();

    for proc_file in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(contents) = fs::read_to_string(proc_file) {
            collect_listening_inodes_on_port(&contents, port, &mut inodes);
        }
    }

    if inodes.is_empty() {
        return Vec::new();
    }

    let mut pids = FxHashSet::default();

    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let pid_str = file_name.to_string_lossy();

            if let Ok(pid) = pid_str.parse::<u32>() {
                let fd_dir = format!("/proc/{pid}/fd");

                if let Ok(fd_entries) = fs::read_dir(fd_dir) {
                    for fd_entry in fd_entries.flatten() {
                        if let Ok(target) = fs::read_link(fd_entry.path()) {
                            let target_str = target.to_string_lossy();

                            if let Some(inode) = socket_inode_from_fd_target(&target_str)
                                && inodes.contains(&inode)
                            {
                                pids.insert(pid);
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    pids.into_iter().collect()
}

#[cfg(not(target_os = "linux"))]
pub fn get_pids_listening_on_port(_port: u16) -> Vec<u32> {
    Vec::new()
}

#[cfg(target_os = "linux")]
fn collect_listening_inodes_on_port(contents: &str, port: u16, inodes: &mut FxHashSet<u64>) {
    let port_suffix = format!(":{port:04X}");

    for line in contents.lines().skip(1) {
        let mut parts = line.split_whitespace();
        let _slot = parts.next();
        let local_addr = parts.next();
        let _remote_addr = parts.next();
        let state = parts.next();
        let inode_str = parts.nth(5);

        if let (Some(local_addr), Some("0A"), Some(inode_str)) = (local_addr, state, inode_str)
            && local_addr.ends_with(&port_suffix)
            && let Ok(inode) = inode_str.parse::<u64>()
        {
            inodes.insert(inode);
        }
    }
}

#[cfg(target_os = "linux")]
fn socket_inode_from_fd_target(target: &str) -> Option<u64> {
    target
        .strip_prefix("socket:[")
        .and_then(|target| target.strip_suffix(']'))
        .and_then(|inode| inode.parse::<u64>().ok())
}

#[cfg(target_os = "linux")]
fn send_sigterm(pid: u32) -> io::Result<()> {
    let pid = i32::try_from(pid)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "PID exceeds i32 range"))?;
    let result = unsafe { kill(pid, SIGTERM) };

    if result == 0 {
        return Ok(());
    }

    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn collects_listening_inodes_for_port() {
        let contents = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:10E0 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 12345 1
   1: 0100007F:10E1 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 23456 1
   2: 0100007F:10E0 00000000:0000 01 00000000:00000000 00:00000000 00000000  1000        0 34567 1
";
        let mut inodes = FxHashSet::default();

        collect_listening_inodes_on_port(contents, 4320, &mut inodes);

        assert!(inodes.contains(&12345));
        assert_eq!(inodes.len(), 1);
    }

    #[test]
    fn extracts_socket_inode_from_fd_target() {
        assert_eq!(socket_inode_from_fd_target("socket:[12345]"), Some(12345));
        assert_eq!(socket_inode_from_fd_target("/tmp/file"), None);
    }
}
