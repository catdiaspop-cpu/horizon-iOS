//! Read-only process memory diagnostics.
/// The process's memory counters.
#[cfg(windows)]
fn counters() -> Option<windows_sys::Win32::System::ProcessStatus::PROCESS_MEMORY_COUNTERS_EX2> {
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX2,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32;
    // SAFETY: all-zero is a valid value of this plain C struct, which starts with the fields of the
    // one the call is declared with, and the call writes at most `size` bytes into it; the current
    // process's pseudo-handle needs no closing.
    unsafe {
        let mut counters: PROCESS_MEMORY_COUNTERS_EX2 = std::mem::zeroed();
        counters.cb = size;
        (K32GetProcessMemoryInfo(GetCurrentProcess(), (&raw mut counters).cast(), size) != 0)
            .then_some(counters)
    }
}

/// The memory this process holds in RAM, in bytes.
#[cfg(windows)]
pub fn resident_bytes() -> Option<u64> {
    Some(counters()?.WorkingSetSize as u64)
}

/// The private memory this process holds in RAM, all it holds in RAM and the machine's RAM, in
/// bytes.
#[cfg(windows)]
pub fn usage() -> Option<[u64; 3]> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let counters = counters()?;
    let resident = counters.WorkingSetSize as u64;
    let private = match counters.PrivateWorkingSetSize {
        // Windows before 10 22H2 leaves it empty: the private commit is the nearest it has.
        0 => (counters.PrivateUsage as u64).min(resident),
        bytes => bytes as u64,
    };
    // SAFETY: all-zero is a valid value of this plain C struct, its length is set as the call
    // requires, and the call only writes into it.
    let ram = unsafe {
        let mut status: MEMORYSTATUSEX = std::mem::zeroed();
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        (GlobalMemoryStatusEx(&mut status) != 0).then_some(status.ullTotalPhys)
    }?;
    Some([private, resident, ram])
}

/// [`usage`]'s counters: not read on this system.
#[cfg(not(windows))]
pub fn usage() -> Option<[u64; 3]> {
    None
}

/// The memory this process holds in RAM, in bytes.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn resident_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    // SAFETY: reads one system constant.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    Some(pages * u64::try_from(page).ok()?)
}

/// The memory this process holds in RAM, in bytes.
///
/// macOS: `proc_pidinfo`. iOS keeps libproc's task info to the system, so the
/// app asks the kernel about its own task instead (`task_info`, the call
/// `mach_task_self()`'s port is for), which a sandboxed app may do.
#[cfg(target_os = "ios")]
pub fn resident_bytes() -> Option<u64> {
    // libSystem's own-task port: the variable behind Mach's `mach_task_self()`.
    // SAFETY: a plain `mach_port_t` libSystem exports and never frees.
    unsafe extern "C" {
        static mach_task_self_: libc::mach_port_t;
    }
    let mut info = std::mem::MaybeUninit::<libc::mach_task_basic_info>::uninit();
    let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
    // SAFETY: `info` is a live, writable value of exactly the `natural_t` count the call
    // is told, and the task port is the calling task's own.
    let read = unsafe {
        libc::task_info(
            mach_task_self_,
            libc::MACH_TASK_BASIC_INFO,
            info.as_mut_ptr().cast(),
            &raw mut count,
        )
    };
    if read != libc::KERN_SUCCESS {
        return None;
    }
    // SAFETY: a successful call wrote every field of the value.
    Some(unsafe { info.assume_init() }.resident_size)
}

/// The memory this process holds in RAM, in bytes.
#[cfg(target_os = "macos")]
pub fn resident_bytes() -> Option<u64> {
    let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
    // SAFETY: all-zero is a valid value of this plain C struct, and the call writes at most `size`
    // bytes into it.
    unsafe {
        let mut info: libc::proc_taskinfo = std::mem::zeroed();
        let written = libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDTASKINFO,
            0,
            (&raw mut info).cast(),
            size,
        );
        (written == size).then_some(info.pti_resident_size)
    }
}

/// The memory this process holds in RAM, in bytes: not known on this system.
#[cfg(not(any(
    windows,
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios"
)))]
pub fn resident_bytes() -> Option<u64> {
    None
}

/// The machine's RAM in bytes, as `hw.memsize` reports it. The settings use it on a phone
/// to size the world to what the device has before the first frame shows its stutter.
#[cfg(target_vendor = "apple")]
pub fn physical_memory_bytes() -> Option<u64> {
    let mut bytes: u64 = 0;
    let mut size = std::mem::size_of::<u64>();
    // SAFETY: a NUL-terminated name; `bytes` and `size` are live locals of the size passed.
    let read = unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&raw mut bytes).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (read == 0 && bytes > 0).then_some(bytes)
}

/// The machine's RAM in bytes, as `/proc/meminfo` reports it.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn physical_memory_bytes() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = text
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some(kb * 1024)
}

/// [`physical_memory_bytes`]: not known on this system.
#[cfg(not(any(target_vendor = "apple", target_os = "linux", target_os = "android")))]
pub fn physical_memory_bytes() -> Option<u64> {
    None
}

#[cfg(all(test, any(windows, target_os = "linux", target_vendor = "apple")))]
mod tests {
    use super::*;

    #[test]
    fn touched_memory_counts_as_resident() {
        const SIZE: usize = 256 << 20;
        let before = resident_bytes().unwrap();
        let block = vec![1u8; SIZE];
        let during = resident_bytes().unwrap();
        std::hint::black_box(&block);
        assert!(
            during >= before + SIZE as u64 / 2,
            "{before} then {during} bytes"
        );
    }
}
