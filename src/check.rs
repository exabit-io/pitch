//! Whether a pitch can take effect here. Utilization clamps steer only the schedutil cpufreq governor, and the system
//! limits sched_util_clamp_min/max cap every task's request. Everything is read from a root directory so tests can use
//! a fake tree; nothing is written.

use std::fs;
use std::path::{Path, PathBuf};

const CPU: &str = "sys/devices/system/cpu";

/// Warnings for a requested pitch (empty when nothing stands in its way). An entry starting with "hint: " says how to
/// fix the warning before it.
pub fn warnings(root: &Path, pitch: i32) -> Vec<String> {
    let mut w = Vec::new();
    if pitch == 0 {
        return w;
    }
    let policies = policies(root);
    let mut others: Vec<(String, usize)> = Vec::new();
    for p in &policies {
        let g = read(p, "scaling_governor").unwrap_or_else(|| "unknown".into());
        if g != "schedutil" {
            match others.iter_mut().find(|(n, _)| *n == g) {
                Some(e) => e.1 += 1,
                None => others.push((g, 1)),
            }
        }
    }
    let total = policies.len();
    if total == 0 {
        w.push("no cpufreq policies found (no frequency scaling, e.g. in a VM); a pitch changes no frequency here".into());
    } else if !others.is_empty() {
        let n: usize = others.iter().map(|o| o.1).sum();
        let list = others.iter().map(|(g, c)| format!("{g} on {c}")).collect::<Vec<_>>().join(", ");
        let scope = if n == total { format!("all {total}") } else { format!("{n} of {total}") };
        let first = policies.iter().find(|p| read(p, "scaling_governor").as_deref() != Some("schedutil")).unwrap();
        w.push(format!("{scope} cpufreq policies run another governor than schedutil ({list}); a pitch changes no frequency there"));
        w.push(format!("hint: {}", hint(first)));
    }
    if pitch > 0 {
        let (min, _) = crate::clamp::clamps(pitch).unwrap_or((0, 0));
        if let Some(cap) = read(&root.join("proc/sys/kernel"), "sched_util_clamp_min").and_then(|s| s.parse::<u32>().ok()) {
            if cap < min {
                w.push(format!(
                    "kernel.sched_util_clamp_min = {cap} caps the boost below the requested {min} (pitch +{})",
                    crate::clamp::pitch_of(cap, 1024)
                ));
            }
        }
    }
    w
}

/// How to get schedutil on a policy, by cpufreq driver.
fn hint(policy: &Path) -> String {
    let driver = read(policy, "scaling_driver").unwrap_or_default();
    let available = read(policy, "scaling_available_governors").unwrap_or_default();
    let select = "boot with cpufreq.default_governor=schedutil, or now: \
                  echo schedutil | sudo tee /sys/devices/system/cpu/cpufreq/policy*/scaling_governor";
    match driver.as_str() {
        "intel_pstate" => "intel_pstate is in active mode, which has no schedutil: boot with intel_pstate=passive \
                           cpufreq.default_governor=schedutil (or now: echo passive | sudo tee \
                           /sys/devices/system/cpu/intel_pstate/status, then select schedutil); or: sudo schedutil-setup install --now"
            .into(),
        "amd-pstate-epp" => "amd-pstate is in active (EPP) mode, which has no schedutil: boot with amd_pstate=passive \
                             (or guided) cpufreq.default_governor=schedutil (or now: echo passive | sudo tee \
                             /sys/devices/system/cpu/amd_pstate/status, then select schedutil); or: sudo schedutil-setup install --now"
            .into(),
        d if !available.split_whitespace().any(|g| g == "schedutil") => {
            format!("the {} driver offers no schedutil (available: {available})", if d.is_empty() { "cpufreq" } else { d })
        }
        _ => format!("select it: {select}; or: sudo schedutil-setup install --now"),
    }
}

fn policies(root: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(root.join(CPU).join("cpufreq"))
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| is_policy(p)).collect())
        .unwrap_or_default();
    v.sort_by_key(|p| p.file_name().and_then(|n| n.to_str()).and_then(|n| n[6..].parse::<u32>().ok()).unwrap_or(0));
    v
}

fn is_policy(p: &Path) -> bool {
    p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.strip_prefix("policy").is_some_and(|d| d.parse::<u32>().is_ok()))
}

fn read(dir: &Path, name: &str) -> Option<String> {
    fs::read_to_string(dir.join(name)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// policies: (governor, driver, available governors)
    fn tree(tag: &str, pols: &[(&str, &str, &str)], clamp_min: Option<u32>) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pitch-chk-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        for (i, (g, drv, av)) in pols.iter().enumerate() {
            let p = d.join(format!("{CPU}/cpufreq/policy{i}"));
            fs::create_dir_all(&p).unwrap();
            fs::write(p.join("scaling_governor"), format!("{g}\n")).unwrap();
            fs::write(p.join("scaling_driver"), format!("{drv}\n")).unwrap();
            fs::write(p.join("scaling_available_governors"), format!("{av}\n")).unwrap();
        }
        fs::create_dir_all(d.join(format!("{CPU}/cpufreq/other"))).unwrap();
        if let Some(c) = clamp_min {
            fs::create_dir_all(d.join("proc/sys/kernel")).unwrap();
            fs::write(d.join("proc/sys/kernel/sched_util_clamp_min"), format!("{c}\n")).unwrap();
        }
        d
    }
    const PASSIVE: &str = "conservative ondemand userspace powersave performance schedutil";

    #[test]
    fn schedutil_everywhere_is_quiet() {
        let d = tree("su", &[("schedutil", "intel_cpufreq", PASSIVE), ("schedutil", "intel_cpufreq", PASSIVE)], Some(1024));
        assert!(warnings(&d, 20).is_empty());
        assert!(warnings(&d, -20).is_empty());
    }

    #[test]
    fn intel_active_names_passive_mode() {
        let d = tree("ia", &[("powersave", "intel_pstate", "performance powersave"); 3], None);
        let w = warnings(&d, 20);
        assert!(w.len() == 2 && w[0].starts_with("all 3 cpufreq policies") && w[0].contains("powersave on 3"), "{w:?}");
        assert!(w[1].starts_with("hint: intel_pstate is in active mode") && w[1].contains("boot with intel_pstate=passive cpufreq.default_governor=schedutil"), "{w:?}");
        assert!(w[1].ends_with("or: sudo schedutil-setup install --now"), "{w:?}");
    }

    #[test]
    fn amd_epp_names_passive_mode() {
        let d = tree("amd", &[("powersave", "amd-pstate-epp", "performance powersave")], None);
        assert!(warnings(&d, -5)[1].contains("boot with amd_pstate=passive (or guided)"));
    }

    #[test]
    fn passive_driver_with_other_governor_says_select_it() {
        let d = tree("od", &[("ondemand", "intel_cpufreq", PASSIVE), ("ondemand", "intel_cpufreq", PASSIVE),
                             ("schedutil", "intel_cpufreq", PASSIVE), ("performance", "acpi-cpufreq", PASSIVE)], None);
        let w = warnings(&d, 10);
        assert!(w[0].starts_with("3 of 4 cpufreq policies run") && w[0].contains("ondemand on 2, performance on 1"), "{w:?}");
        assert!(w[1].starts_with("hint: select it: boot with cpufreq.default_governor=schedutil"), "{w:?}");
    }

    #[test]
    fn driver_without_schedutil_is_named() {
        let d = tree("ns", &[("performance", "weird-cpufreq", "performance")], None);
        assert_eq!(warnings(&d, 10)[1], "hint: the weird-cpufreq driver offers no schedutil (available: performance)");
    }

    #[test]
    fn system_clamp_cap_is_reported_for_boosts_only() {
        let d = tree("cap", &[("schedutil", "intel_cpufreq", PASSIVE)], Some(512));
        let w = warnings(&d, 20);
        assert!(w.len() == 1 && w[0].contains("= 512") && w[0].contains("+10"), "{w:?}");
        assert!(warnings(&d, 10).is_empty()); // 512 asked, 512 allowed
        assert!(warnings(&d, -20).is_empty());
    }

    #[test]
    fn pitch_zero_and_missing_tree() {
        let d = tree("z", &[("ondemand", "intel_cpufreq", PASSIVE)], None);
        assert!(warnings(&d, 0).is_empty());
        let w = warnings(Path::new("/nonexistent-pitch-root"), 10);
        assert!(w[0].starts_with("no cpufreq policies"));
    }
}
