//! Read-only checks of the shipped scripts/schedutil-setup against the build machine (nothing is changed: status,
//! check, verify and --dry-run never write).
use std::process::Command;

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/schedutil-setup");

fn run(args: &[&str]) -> (i32, String) {
    let o = Command::new("bash").arg(SCRIPT).args(args).output().unwrap();
    (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr))
}

#[test]
fn version_matches_the_crate() {
    let (rc, out) = run(&["--version"]);
    assert_eq!((rc, out.trim()), (0, concat!("schedutil-setup ", env!("CARGO_PKG_VERSION"))));
}

#[test]
fn status_is_read_only_and_succeeds() {
    let (rc, out) = run(&["status"]);
    assert!(rc == 0 && out.contains("driver:") && out.contains("checks:"), "{out}");
}

#[test]
fn check_and_verify_exit_with_documented_codes() {
    assert!([0, 3, 4].contains(&run(&["check"]).0));
    assert!([0, 1].contains(&run(&["verify"]).0));
}

#[test]
fn dry_run_changes_nothing_and_usage_errors() {
    let (rc, out) = run(&["install", "--dry-run"]);
    assert!([0, 3, 4].contains(&rc), "{out}");
    if rc == 0 {
        assert!(out.contains("dry run: nothing was changed"), "{out}");
    }
    assert_eq!(run(&["bogus"]).0, 2);
    assert_eq!(run(&["install", "--bogus"]).0, 2);
}

#[test]
fn stock_files_ship_and_a_missing_one_stops_install_before_any_change() {
    for f in ["60-schedutil-rt-clamp.conf", "96-cpufreq-schedutil.cfg", "cpufrequtils"] {
        let p = format!("{}/share/pitch/schedutil/{f}", env!("CARGO_MANIFEST_DIR"));
        assert!(std::fs::metadata(&p).is_ok(), "{p}");
    }
    let o = Command::new("bash").arg(SCRIPT).args(["install", "--dry-run", "--force"])
        .env("SCHEDUTIL_SETUP_STOCK", "/nonexistent-stock").output().unwrap();
    let out = String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr);
    if o.status.code() != Some(3) || !out.contains("stock file") {
        // only systems where preflight itself says "unsupported" may stop earlier, also with 3
        assert_eq!(o.status.code(), Some(3), "{out}");
    }
    assert!(!out.contains("installing ("), "{out}");
}
