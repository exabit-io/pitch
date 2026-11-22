# pitch

Run a command at a higher or lower CPU frequency. The inverse of `nice`: `nice` changes how much CPU time a process
gets, `pitch` changes how fast the CPU runs while the process has it.

```sh
pitch -n 20 -- echo "Maximum warp. Engage!"     # full speed
pitch -n 20 -- perl -le'print Zsb1e6pzo^chr((grep{(1x$_)!~/^(11+)\1+$/}2..2e4)%32)x9'
pitch -n -20 -- echo "One-quarter impulse."     # slowest
pitch -- make -j8                               # +10, the default
pitch -n -15 -- rsync -a mirror/ /srv/mirror/   # background, near the lowest frequency
pitch -n 20 -p 1234                             # every thread of a running process
pitch -p 1234                                   # show it
pitch                                           # this process's (inherited) pitch
```

`N` runs from -20 to 20, higher is faster. `+N` raises the task's utilization clamp floor (`uclamp.min = N/20 x 1024`),
so the CPU it runs on clocks as if it kept that CPU busy; `-N` lowers the ceiling (`uclamp.max = (20+N)/20 x 1024`);
`0` restores the default. Children inherit it. The value is absolute, not an adjustment.

## Why

Short-lived processes (configure tests, compiler invocations, package-build helpers) land on cold cores and finish
before frequency governors ramp up. A clamp floor fixes that **per task**: it follows the task from core to core and
leaves everything else on the machine alone. The kernel has had this since 5.3 (`uclampset` in util-linux exposes the
raw values); `pitch` gives it nice's shape, checks that it can take effect, and covers running processes' threads.

Measured on a Dell PowerEdge R940xa (4x Xeon Platinum 8280L, Ubuntu 24.04, kernel 7.0, intel_pstate passive +
schedutil), serial GNU hello package build, interleaved runs:

| | seconds | package energy |
|---|---|---|
| plain | 21.3-22.4 | 3,750-3,870 J |
| `pitch -n 20` | 18.0-18.1 | 3,320-3,350 J |
| socket-wide 3.3 GHz frequency floor | 18.0-18.2 | 3,350-3,380 J |

## Requirements

- A kernel with `CONFIG_UCLAMP_TASK` (Ubuntu, Debian, Fedora kernels have it).
- The **schedutil** governor. On Intel: `intel_pstate=passive cpufreq.default_governor=schedutil` on the kernel
  command line. Other governors ignore clamps; `pitch` warns.
- With schedutil, set `kernel.sched_util_clamp_min_rt_default=0`. Otherwise every runnable realtime task requests the
  top frequency; on the R940xa four 100 ms `SCHED_FIFO` heartbeats held the uncore at maximum, +72-76 W at idle.

Any user may set their own processes' pitch in both directions. Bound it with cgroup `cpu.uclamp.max` or the
`kernel.sched_util_clamp_min` sysctl.

## Build

```sh
cargo build --release          # no dependencies beyond std
cargo test                     # unit tests + end-to-end tests against the running kernel
dpkg-buildpackage -us -uc -b   # Debian package
```

x86_64 and aarch64.

## License

MIT OR Apache-2.0, at your option. Copyright 2026 Exabit, Inc.
