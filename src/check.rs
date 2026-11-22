//! Whether a pitch can take effect here. Utilization clamps steer only the schedutil cpufreq governor, and the system
//! limits sched_util_clamp_min/max cap every task's request. Everything is read from a root directory so tests can use
//! a fake tree; nothing is written.

use std::fs;
use std::path::Path;

/// Warnings for a requested pitch (empty when nothing stands in its way).
pub fn warnings(root: &Path, pitch: i32) -> Vec<String> {
    let mut w = Vec::new();
    if pitch == 0 {
        return w;
    }
    let mut total = 0;
    let mut others: Vec<(String, usize)> = Vec::new();
    if let Ok(rd) = fs::read_dir(root.join("sys/devices/system/cpu/cpufreq")) {
        let mut names: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| is_policy(p)).collect();
        names.sort();
        for p in names {
            let g = fs::read_to_string(p.join("scaling_governor")).unwrap_or_default().trim().to_string();
            total += 1;
            if g != "schedutil" {
                let g = if g.is_empty() { "unknown".to_string() } else { g };
                match others.iter_mut().find(|(n, _)| *n == g) {
                    Some(e) => e.1 += 1,
                    None => others.push((g, 1)),
                }
            }
        }
    }
    if total == 0 {
        w.push("no cpufreq policies found; a pitch has no frequency effect without the schedutil governor".into());
    } else if !others.is_empty() {
        let n: usize = others.iter().map(|o| o.1).sum();
        let list = others.iter().map(|(g, c)| format!("{g} on {c}")).collect::<Vec<_>>().join(", ");
        let scope = if n == total { format!("all {total}") } else { format!("{n} of {total}") };
        w.push(format!("{scope} cpufreq policies run another governor than schedutil ({list}); a pitch changes no frequency there"));
    }
    if pitch > 0 {
        let (min, _) = crate::clamp::clamps(pitch).unwrap_or((0, 0));
        if let Some(cap) = read_num(root, "proc/sys/kernel/sched_util_clamp_min") {
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

fn is_policy(p: &Path) -> bool {
    p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.strip_prefix("policy").is_some_and(|d| d.parse::<u32>().is_ok()))
}

fn read_num(root: &Path, rel: &str) -> Option<u32> {
    fs::read_to_string(root.join(rel)).ok()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tree(govs: &[&str], clamp_min: Option<u32>) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pitch-chk-{}-{}", std::process::id(), govs.join("_")));
        let _ = fs::remove_dir_all(&d);
        for (i, g) in govs.iter().enumerate() {
            let p = d.join(format!("sys/devices/system/cpu/cpufreq/policy{i}"));
            fs::create_dir_all(&p).unwrap();
            fs::write(p.join("scaling_governor"), format!("{g}\n")).unwrap();
        }
        fs::create_dir_all(d.join("sys/devices/system/cpu/cpufreq/other")).unwrap();
        if let Some(c) = clamp_min {
            fs::create_dir_all(d.join("proc/sys/kernel")).unwrap();
            fs::write(d.join("proc/sys/kernel/sched_util_clamp_min"), format!("{c}\n")).unwrap();
        }
        d
    }

    #[test]
    fn schedutil_everywhere_is_quiet() {
        let d = tree(&["schedutil", "schedutil"], Some(1024));
        assert!(warnings(&d, 20).is_empty());
        assert!(warnings(&d, -20).is_empty());
    }

    #[test]
    fn other_governors_are_named_with_counts() {
        let d = tree(&["ondemand", "ondemand", "schedutil", "performance"], None);
        let w = warnings(&d, 10);
        assert_eq!(w.len(), 1);
        assert!(w[0].starts_with("3 of 4 cpufreq policies run") && w[0].contains("ondemand on 2, performance on 1"), "{w:?}");
        let d = tree(&["ondemand"], None);
        assert!(warnings(&d, 5)[0].starts_with("all 1 cpufreq"));
    }

    #[test]
    fn system_clamp_cap_is_reported_for_boosts_only() {
        let d = tree(&["schedutil"], Some(512));
        let w = warnings(&d, 20);
        assert!(w.len() == 1 && w[0].contains("= 512") && w[0].contains("+10"), "{w:?}");
        assert!(warnings(&d, 10).is_empty()); // 512 asked, 512 allowed
        assert!(warnings(&d, -20).is_empty());
    }

    #[test]
    fn pitch_zero_and_missing_tree() {
        let d = tree(&["ondemand"], None);
        assert!(warnings(&d, 0).is_empty());
        let w = warnings(Path::new("/nonexistent-pitch-root"), 10);
        assert!(w[0].starts_with("no cpufreq policies"));
    }
}
