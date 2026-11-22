# Changelog

## 1.0.0 (2026-11-22)

- First release. `pitch [-n N] -- CMD`, `pitch -n N -p PID...`, `pitch -p PID...`, `pitch`.
- N from -20 to 20 maps to the scheduler's utilization clamps (floor for +N, ceiling for -N, reset for 0); children
  inherit it. Default +10.
- Warns when a governor other than schedutil is in use or kernel.sched_util_clamp_min caps the boost; runs the command
  anyway when the pitch cannot be set (like nice). Exit codes 125/126/127 as nice and env.
- Extracted from intel-powermode's `hold fast` (a socket-wide frequency floor) once per-task clamps measured the same
  speed with less energy on a schedutil base.
