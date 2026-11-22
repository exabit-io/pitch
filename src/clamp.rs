//! Pitch <-> utilization clamp mapping, and the sched_setattr/sched_getattr system calls (no libc crate: std already
//! links libc, so `syscall` is declared directly).
//!
//! Pitch N in -20..=20:  N > 0 raises the clamp floor   uclamp.min = N/20 * 1024 (uclamp.max = 1024)
//!                       N < 0 lowers the clamp ceiling uclamp.max = (20+N)/20 * 1024 (uclamp.min = 0)
//!                       N = 0 resets both to the kernel default (the task follows its cgroup and the system defaults)
//! Under the schedutil governor a clamp floor makes the CPU the task runs on clock as if the task kept it that busy; a
//! ceiling caps the frequency it may ask for. Children inherit the clamps across fork and exec.

use std::io;

pub const MAX_PITCH: i32 = 20;
pub const SCALE: u32 = 1024; // SCHED_CAPACITY_SCALE

const SCHED_FLAG_KEEP_POLICY: u64 = 0x08;
const SCHED_FLAG_KEEP_PARAMS: u64 = 0x10;
const SCHED_FLAG_UTIL_CLAMP_MIN: u64 = 0x20;
const SCHED_FLAG_UTIL_CLAMP_MAX: u64 = 0x40;
const RESET: u32 = u32::MAX; // (u32)-1: back to the default clamp

#[cfg(target_arch = "x86_64")]
const SYS_SCHED_SETATTR: i64 = 314;
#[cfg(target_arch = "x86_64")]
const SYS_SCHED_GETATTR: i64 = 315;
#[cfg(target_arch = "aarch64")]
const SYS_SCHED_SETATTR: i64 = 274;
#[cfg(target_arch = "aarch64")]
const SYS_SCHED_GETATTR: i64 = 275;
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!("pitch knows the sched_setattr system call numbers for x86_64 and aarch64 only");

extern "C" {
    fn syscall(num: i64, ...) -> i64;
}

/// struct sched_attr, SCHED_ATTR_SIZE_VER1 (56 bytes): the first layout with utilization clamps.
#[repr(C)]
#[derive(Default, Debug)]
struct SchedAttr {
    size: u32,
    sched_policy: u32,
    sched_flags: u64,
    sched_nice: i32,
    sched_priority: u32,
    sched_runtime: u64,
    sched_deadline: u64,
    sched_period: u64,
    sched_util_min: u32,
    sched_util_max: u32,
}

/// The (uclamp.min, uclamp.max) a pitch requests; None = reset to the default.
pub fn clamps(pitch: i32) -> Option<(u32, u32)> {
    let p = pitch.clamp(-MAX_PITCH, MAX_PITCH);
    let scaled = |n: i32| ((n as u32 * SCALE) as f64 / MAX_PITCH as f64).round() as u32;
    match p {
        0 => None,
        p if p > 0 => Some((scaled(p), SCALE)),
        p => Some((0, scaled(MAX_PITCH + p))),
    }
}

/// The pitch that best describes a task's clamps. A floor wins over a ceiling when someone else set both.
pub fn pitch_of(min: u32, max: u32) -> i32 {
    let to = |v: u32| ((v.min(SCALE) as f64) * MAX_PITCH as f64 / SCALE as f64).round() as i32;
    if min > 0 {
        to(min).max(1)
    } else if max < SCALE {
        -(MAX_PITCH - to(max)).max(1)
    } else {
        0
    }
}

/// Parse a pitch argument: an integer, optionally signed ("+20", "-5"), within -20..=20.
pub fn parse_pitch(s: &str) -> Result<i32, String> {
    let n: i32 = s.trim().parse().map_err(|_| format!("invalid pitch {s:?} (an integer from -20 to 20)"))?;
    if !(-MAX_PITCH..=MAX_PITCH).contains(&n) {
        return Err(format!("pitch {n} out of range (-20 to 20)"));
    }
    Ok(n)
}

/// Set the clamps of one task (thread id; 0 = the calling thread). Policy, priority and nice are kept.
pub fn set(tid: i32, pitch: i32) -> io::Result<()> {
    let (min, max) = clamps(pitch).unwrap_or((RESET, RESET));
    let attr = SchedAttr {
        size: std::mem::size_of::<SchedAttr>() as u32,
        sched_flags: SCHED_FLAG_KEEP_POLICY | SCHED_FLAG_KEEP_PARAMS | SCHED_FLAG_UTIL_CLAMP_MIN | SCHED_FLAG_UTIL_CLAMP_MAX,
        sched_util_min: min,
        sched_util_max: max,
        ..Default::default()
    };
    // SAFETY: the kernel reads `size` bytes of a valid, initialized repr(C) struct that outlives the call.
    let r = unsafe { syscall(SYS_SCHED_SETATTR, tid as i64, &attr as *const SchedAttr, 0u64) };
    if r < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

/// The requested (uclamp.min, uclamp.max) of one task.
pub fn get(tid: i32) -> io::Result<(u32, u32)> {
    let mut attr = SchedAttr::default();
    let size = std::mem::size_of::<SchedAttr>() as u32;
    // SAFETY: the kernel writes at most `size` bytes into a valid repr(C) struct that outlives the call.
    let r = unsafe { syscall(SYS_SCHED_GETATTR, tid as i64, &mut attr as *mut SchedAttr, size as u64, 0u64) };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    if attr.size < size {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "kernel reports no utilization clamps"));
    }
    Ok((attr.sched_util_min, attr.sched_util_max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_is_ver1() {
        assert_eq!(std::mem::size_of::<SchedAttr>(), 56);
    }

    #[test]
    fn mapping_ends_and_middle() {
        assert_eq!(clamps(20), Some((1024, 1024)));
        assert_eq!(clamps(10), Some((512, 1024)));
        assert_eq!(clamps(1), Some((51, 1024)));
        assert_eq!(clamps(0), None);
        assert_eq!(clamps(-10), Some((0, 512)));
        assert_eq!(clamps(-20), Some((0, 0)));
        assert_eq!(clamps(99), Some((1024, 1024))); // saturates
    }

    #[test]
    fn every_pitch_round_trips() {
        for p in -20..=20 {
            let (mn, mx) = clamps(p).unwrap_or((0, SCALE));
            assert_eq!(pitch_of(mn, mx), p, "pitch {p}");
        }
        assert_eq!(pitch_of(3, 1024), 1); // any floor counts as at least +1
        assert_eq!(pitch_of(0, 1020), -1);
        assert_eq!(pitch_of(256, 512), 5); // floor wins
    }

    #[test]
    fn parse_accepts_signs_and_rejects_junk() {
        assert_eq!(parse_pitch("+20"), Ok(20));
        assert_eq!(parse_pitch("-5"), Ok(-5));
        assert_eq!(parse_pitch("0"), Ok(0));
        for bad in ["21", "-21", "x", "", "1.5"] {
            assert!(parse_pitch(bad).is_err(), "{bad:?}");
        }
    }
}
