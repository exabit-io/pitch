# Changelog

## 1.1.0 (2026-11-22)

- `-b, --background`: the background tier in one flag. Adds SCHED_IDLE (CPU only when nothing else wants it) and the
  idle I/O class to the pitch, which defaults to -20 (`-b -n 20`: spare capacity at full speed). Works with `-p` too.
- `-p PID` also shows the scheduling policy and I/O class.
- The governor warning names the fix for the cpufreq driver in use (intel_pstate active: `intel_pstate=passive`;
  amd-pstate-epp: `amd_pstate=passive`; others: select schedutil) on its own `pitch: hint:` line.
- riscv64 and loongarch64 system call numbers (the generic table, as arm64).

## 1.0.0 (2026-11-22)

- First release. `pitch [-n N] -- CMD`, `pitch -n N -p PID...`, `pitch -p PID...`, `pitch`.
- N from -20 to 20 maps to the scheduler's utilization clamps (floor for +N, ceiling for -N, reset for 0); children
  inherit it. Default +20: on the R940xa +5 and +10 barely changed a serial build (0 to -2 %), +18 and +20 cut
  it by 15 %.
- Warns when a governor other than schedutil is in use or kernel.sched_util_clamp_min caps the boost; runs the command
  anyway when the pitch cannot be set (like nice). Exit codes 125/126/127 as nice and env.
- Extracted from intel-powermode's `hold fast` (a socket-wide frequency floor) once per-task clamps measured the same
  speed with less energy on a schedutil base.
