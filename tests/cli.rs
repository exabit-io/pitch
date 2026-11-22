//! End-to-end tests of the built binary against the running kernel. They need a kernel with utilization clamping
//! (CONFIG_UCLAMP_TASK); without it they check the failure path instead. No governor or sysfs value is changed:
//! clamps are per task and die with the test processes.
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_pitch");

fn pitch(args: &[&str]) -> (i32, String, String) {
    let o = Command::new(BIN).args(args).output().unwrap();
    (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).into(), String::from_utf8_lossy(&o.stderr).into())
}

fn uclamp_supported() -> bool {
    !pitch(&["-q", "-n", "1", "--", "true"]).2.contains("cannot set")
}

#[test]
fn run_sets_and_children_inherit() {
    if !uclamp_supported() {
        let (rc, _, err) = pitch(&["-q", "-n", "20", "--", "true"]);
        assert!(rc == 0 && err.contains("CONFIG_UCLAMP_TASK"), "{err}");
        return;
    }
    for (n, want) in [("20", "20"), ("+7", "7"), ("-20", "-20"), ("-3", "-3"), ("0", "0")] {
        let (rc, out, _) = pitch(&["-q", "-n", n, "--", BIN]); // the child pitch prints its inherited pitch
        assert_eq!((rc, out.trim()), (0, want), "pitch -n {n}");
    }
    // through a shell and a grandchild
    let (rc, out, _) = pitch(&["-q", "-n", "12", "--", "sh", "-c", BIN]);
    assert_eq!((rc, out.trim()), (0, "12"));
    // default is +20
    let (_, out, _) = pitch(&["-q", "--", BIN]);
    assert_eq!(out.trim(), "20");
}

#[test]
fn pid_mode_sets_every_thread_and_shows_it() {
    if !uclamp_supported() {
        return;
    }
    let mut child = Command::new("sleep").arg("30").stdout(Stdio::null()).spawn().unwrap();
    let pid = child.id().to_string();
    let (rc, out, _) = pitch(&["-q", "-n", "15", "-p", &pid]);
    assert!(rc == 0 && out.contains("pitch +15 (1 threads)"), "{out}");
    let (rc, out, _) = pitch(&["-p", &pid]);
    assert!(rc == 0 && out.contains("sleep pitch +15 (uclamp min=768 max=1024"), "{out}");
    let (rc, _, _) = pitch(&["-q", "-n", "0", "-p", &pid]);
    assert_eq!(rc, 0);
    assert!(pitch(&["-p", &pid]).1.contains("pitch 0 "));
    child.kill().unwrap();
    let _ = child.wait();
    let (rc, _, err) = pitch(&["-p", &pid]);
    assert!(rc == 1 && err.contains("no such process"), "{err}");
}

#[test]
fn exit_codes_follow_nice() {
    assert_eq!(pitch(&["-q", "--", "/nonexistent/cmd"]).0, 127);
    assert_eq!(pitch(&["-q", "--", "/etc/hostname"]).0, 126);
    assert_eq!(pitch(&["-n", "99", "true"]).0, 125);
    assert_eq!(pitch(&["-q", "--", "sh", "-c", "exit 7"]).0, 7);
}
