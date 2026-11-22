//! pitch: run a command at a higher or lower CPU frequency. The inverse of nice: nice changes how much CPU time a
//! process gets, pitch changes how fast the CPU runs while it has it, through the scheduler's utilization clamps.
//!
//!   pitch                              print this process's pitch
//!   pitch [-b] [-n N] [--] CMD [ARG...] run CMD at pitch N (default +20; -b: background tier, default -20)
//!   pitch -p PID...                    print the pitch, policy and I/O class of running processes
//!   pitch [-b] -n N -p PID...          set them on running processes (every thread)

mod check;
mod clamp;

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{exit, Command};

const DEFAULT_PITCH: i32 = 20; // measured: the middle of the scale does little, the ends do (README)
const DEFAULT_BACKGROUND_PITCH: i32 = -20;
const EXIT_USAGE: i32 = 125; // like nice(1) and env(1): 125 = pitch itself failed, 126/127 = CMD could not be run

const USAGE: &str = "\
usage: pitch [-q] [-b] [-n N] [--] COMMAND [ARG]...   run COMMAND at pitch N (default +20)
       pitch [-q] [-b] [-n N] -p PID...              set it on running processes (all their threads)
       pitch -p PID...                               show the pitch of running processes
       pitch                                         show this process's pitch

Pitch is the inverse of nice: it sets how fast the CPU runs a task, not how much CPU time the task gets.
N runs from -20 to 20. +N raises the task's utilization clamp floor (uclamp.min = N/20 x 1024): the CPU clocks
as if the task kept it busy, so short and serial work runs at speed. -N lowers the ceiling (uclamp.max =
(20+N)/20 x 1024): the task never asks for more than that share of the top frequency. 0 restores the default.
Children inherit the pitch. Works with the schedutil cpufreq governor only.

  -n, --pitch N      the pitch, -20..20 (+20 = full speed, -20 = slowest)
  -b, --background   background tier: also SCHED_IDLE (CPU only when nothing else wants it) and the idle I/O
                     class; pitch -20 unless -n is given (-b -n 20: use spare capacity at full speed)
  -p, --pid          operate on the listed process IDs instead of running a command
  -q, --quiet        no warnings
  -h, --help         this text
  -V, --version      version";

#[derive(Debug, PartialEq)]
enum Action {
    Show,
    Run { pitch: i32, bg: bool, cmd: Vec<String> },
    ShowPids(Vec<i32>),
    SetPids { pitch: i32, bg: bool, pids: Vec<i32> },
    Help,
    Version,
}

#[derive(Debug, PartialEq)]
struct Opts {
    quiet: bool,
    action: Action,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let (mut quiet, mut pitch, mut pids_mode, mut bg) = (false, None, false, false);
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let val = |i: &mut usize| -> Result<String, String> {
            *i += 1;
            args.get(*i).cloned().ok_or_else(|| format!("option {a} needs a value"))
        };
        match a {
            "--" => {
                i += 1;
                break;
            }
            "-q" | "--quiet" => quiet = true,
            "-b" | "--background" => bg = true,
            "-p" | "--pid" => pids_mode = true,
            "-h" | "--help" => return Ok(Opts { quiet, action: Action::Help }),
            "-V" | "--version" => return Ok(Opts { quiet, action: Action::Version }),
            "-n" | "--pitch" => pitch = Some(clamp::parse_pitch(&val(&mut i)?)?),
            _ if a.starts_with("--pitch=") => pitch = Some(clamp::parse_pitch(&a["--pitch=".len()..])?),
            _ if a.starts_with("-n") && a.len() > 2 => pitch = Some(clamp::parse_pitch(&a[2..])?),
            _ if a.starts_with('-') && a.len() > 1 && !pids_mode => return Err(format!("unknown option {a}")),
            _ => break,
        }
        i += 1;
    }
    let rest: Vec<String> = args[i..].to_vec();
    let default = if bg { DEFAULT_BACKGROUND_PITCH } else { DEFAULT_PITCH };
    let action = if pids_mode {
        if rest.is_empty() {
            return Err("-p needs at least one process ID".into());
        }
        let pids = rest
            .iter()
            .map(|s| s.parse::<i32>().ok().filter(|p| *p > 0).ok_or_else(|| format!("invalid process ID {s:?}")))
            .collect::<Result<Vec<_>, _>>()?;
        match (pitch, bg) {
            (None, false) => Action::ShowPids(pids),
            (p, bg) => Action::SetPids { pitch: p.unwrap_or(default), bg, pids },
        }
    } else if rest.is_empty() {
        if pitch.is_some() || bg {
            return Err("a pitch needs a COMMAND to run (or -p PID)".into());
        }
        Action::Show
    } else {
        Action::Run { pitch: pitch.unwrap_or(default), bg, cmd: rest }
    };
    Ok(Opts { quiet, action })
}

fn describe(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(95) => "the kernel has no utilization clamping (CONFIG_UCLAMP_TASK)".into(), // EOPNOTSUPP
        Some(1) => "permission denied (only your own processes, or root)".into(),        // EPERM
        Some(3) => "no such process".into(),                                              // ESRCH
        _ => e.to_string(),
    }
}

fn signed(p: i32) -> String {
    if p > 0 { format!("+{p}") } else { p.to_string() }
}

fn label(pitch: i32, bg: bool) -> String {
    if bg { format!("{} background", signed(pitch)) } else { signed(pitch) }
}

fn warn(quiet: bool, pitch: i32) {
    if !quiet {
        for w in check::warnings(Path::new("/"), pitch) {
            match w.strip_prefix("hint: ") {
                Some(h) => eprintln!("pitch: hint: {h}"),
                None => eprintln!("pitch: warning: {w}"),
            }
        }
    }
}

/// Apply a pitch (and with `bg` the idle CPU policy and I/O class) to one thread.
fn apply(tid: i32, pitch: i32, bg: bool) -> std::io::Result<()> {
    clamp::set(tid, pitch, bg)?;
    if bg {
        clamp::set_io_idle(tid)?;
    }
    Ok(())
}

fn threads(pid: i32) -> Vec<i32> {
    let mut t: Vec<i32> = std::fs::read_dir(format!("/proc/{pid}/task"))
        .map(|rd| rd.flatten().filter_map(|e| e.file_name().to_str()?.parse().ok()).collect())
        .unwrap_or_default();
    t.sort();
    t
}

fn comm(pid: i32) -> String {
    std::fs::read_to_string(format!("/proc/{pid}/comm")).map(|s| s.trim().to_string()).unwrap_or_else(|_| "?".into())
}

/// The effective clamp floor the scheduler applies (after cgroup and system limits), when /proc/PID/sched shows it.
fn effective_min(pid: i32) -> Option<u32> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/sched")).ok()?;
    s.lines().find(|l| l.starts_with("effective uclamp.min"))?.split(':').nth(1)?.trim().parse().ok()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let o = match parse(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("pitch: {e}\n{USAGE}");
            exit(EXIT_USAGE);
        }
    };
    match o.action {
        Action::Help => println!("{USAGE}"),
        Action::Version => println!("pitch {}", env!("CARGO_PKG_VERSION")),
        Action::Show => match clamp::get(0) {
            Ok(t) => println!("{}", clamp::pitch_of(t.min, t.max)),
            Err(e) => {
                eprintln!("pitch: cannot read the pitch: {}", describe(&e));
                exit(EXIT_USAGE);
            }
        },
        Action::Run { pitch, bg, cmd } => {
            warn(o.quiet, pitch);
            if let Err(e) = apply(0, pitch, bg) {
                // like nice(1): report, then run the command anyway
                eprintln!("pitch: cannot set pitch {}: {}", label(pitch, bg), describe(&e));
            }
            let err = Command::new(&cmd[0]).args(&cmd[1..]).exec();
            eprintln!("pitch: {}: {err}", cmd[0]);
            exit(if err.kind() == std::io::ErrorKind::NotFound { 127 } else { 126 });
        }
        Action::ShowPids(pids) => {
            let mut rc = 0;
            for pid in pids {
                match clamp::get(pid) {
                    Ok(t) => {
                        let eff = effective_min(pid).map(|e| format!(" effective_min={e}")).unwrap_or_default();
                        let io = clamp::io_prio(pid).unwrap_or_else(|_| "?".into());
                        println!(
                            "{pid} {} pitch {} (uclamp min={} max={}{eff}) sched={} io={io}",
                            comm(pid),
                            signed(clamp::pitch_of(t.min, t.max)),
                            t.min,
                            t.max,
                            clamp::policy_name(t.policy)
                        );
                    }
                    Err(e) => {
                        eprintln!("pitch: {pid}: {}", describe(&e));
                        rc = 1;
                    }
                }
            }
            exit(rc);
        }
        Action::SetPids { pitch, bg, pids } => {
            warn(o.quiet, pitch);
            let mut rc = 0;
            for pid in pids {
                let tids = threads(pid);
                if tids.is_empty() {
                    eprintln!("pitch: {pid}: no such process");
                    rc = 1;
                    continue;
                }
                let (mut ok, mut first_err) = (0, None);
                for tid in &tids {
                    match apply(*tid, pitch, bg) {
                        Ok(()) => ok += 1,
                        Err(e) if e.raw_os_error() == Some(3) => {} // thread exited meanwhile
                        Err(e) => {
                            first_err.get_or_insert(e);
                        }
                    }
                }
                match first_err {
                    Some(e) => {
                        eprintln!("pitch: {pid}: {} ({ok} of {} threads set)", describe(&e), tids.len());
                        rc = 1;
                    }
                    None => println!("{pid} {} pitch {} ({ok} threads)", comm(pid), label(pitch, bg)),
                }
            }
            exit(rc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Opts, String> {
        parse(&s.split_whitespace().map(String::from).collect::<Vec<_>>())
    }
    fn run(pitch: i32, bg: bool, cmd: &[&str]) -> Action {
        Action::Run { pitch, bg, cmd: cmd.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn command_forms() {
        assert_eq!(p("make -j8").unwrap().action, run(20, false, &["make", "-j8"]));
        assert_eq!(p("-n 20 -- echo hi").unwrap().action, run(20, false, &["echo", "hi"]));
        assert_eq!(p("-n -5 nice -n 19 tar").unwrap().action, run(-5, false, &["nice", "-n", "19", "tar"])); // CMD's options
        assert_eq!(p("-n+20 x").unwrap().action, run(20, false, &["x"]));
        assert_eq!(p("--pitch=-20 x").unwrap().action, run(-20, false, &["x"]));
        assert_eq!(p("-q -n 0 -- -weird-name").unwrap(), Opts { quiet: true, action: run(0, false, &["-weird-name"]) });
    }

    #[test]
    fn background_forms() {
        assert_eq!(p("-b job").unwrap().action, run(-20, true, &["job"]));
        assert_eq!(p("--background -n 20 -- job").unwrap().action, run(20, true, &["job"]));
        assert_eq!(p("-n -7 -b job").unwrap().action, run(-7, true, &["job"]));
        assert_eq!(p("-b -p 42").unwrap().action, Action::SetPids { pitch: -20, bg: true, pids: vec![42] });
        assert_eq!(p("-b -n 5 -p 42").unwrap().action, Action::SetPids { pitch: 5, bg: true, pids: vec![42] });
        assert!(p("-b").is_err());
    }

    #[test]
    fn show_and_pid_forms() {
        assert_eq!(p("").unwrap().action, Action::Show);
        assert_eq!(p("-p 1 22").unwrap().action, Action::ShowPids(vec![1, 22]));
        assert_eq!(p("-n 20 -p 7").unwrap().action, Action::SetPids { pitch: 20, bg: false, pids: vec![7] });
        assert_eq!(p("-p -n -3 7").unwrap().action, Action::SetPids { pitch: -3, bg: false, pids: vec![7] });
        assert_eq!(p("--help").unwrap().action, Action::Help);
    }

    #[test]
    fn errors() {
        for bad in ["-n", "-n 21 x", "-n abc x", "-x cmd", "-p", "-p 0", "-p abc", "-n 5", "-n 5 -p -1"] {
            assert!(p(bad).is_err(), "{bad:?}");
        }
    }
}
