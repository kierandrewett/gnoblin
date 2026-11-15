//! What the kernel says a session's processes use, read from `/proc`.
//!
//! The performance baseline runs the compositor as a separate process tree (a launcher and everything it starts) and
//! measures it from outside, so the three targets are accounted the same way. A tree is found by its process group: the
//! launcher is started in a group of its own, and the compositor, the private bus and the helpers inherit it. A process
//! file system that does not show a file reports the field as absent and never as zero, so a figure that is missing is
//! not mistaken for a small one.

use std::path::Path;

use rustix::param::{clock_ticks_per_second, page_size};

/// One process of a session at one moment.
#[derive(Debug, Clone)]
pub(crate) struct Sample {
    pub pid: u32,
    /// The program name the kernel keeps, 15 characters at most.
    pub comm: String,
    /// User plus system CPU time so far, in clock ticks.
    pub ticks: u64,
    /// Resident set, in KiB.
    pub rss_kib: u64,
    /// Proportional set size in KiB, where `smaps_rollup` is readable.
    pub pss_kib: Option<u64>,
}

/// Clock ticks per second, the unit of [`Sample::ticks`].
pub(crate) fn ticks_per_second() -> f64 {
    clock_ticks_per_second() as f64
}

/// The processes in process group `group`, one sample each. A process that exits while it is read is left out.
pub(crate) fn members(group: u32) -> Vec<Sample> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut samples = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else {
            continue;
        };
        if let Some(sample) = sample(&entry.path(), pid, group) {
            samples.push(sample);
        }
    }
    samples.sort_by_key(|sample| sample.pid);
    samples
}

/// The sample of the process at `directory` when it belongs to `group`.
fn sample(directory: &Path, pid: u32, group: u32) -> Option<Sample> {
    let stat = std::fs::read_to_string(directory.join("stat")).ok()?;
    let parsed = parse_stat(&stat)?;
    if parsed.group != group {
        return None;
    }
    let pages: u64 = std::fs::read_to_string(directory.join("statm"))
        .ok()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    let rss_kib = pages * u64::try_from(page_size()).unwrap_or(4096) / 1024;
    let pss_kib = std::fs::read_to_string(directory.join("smaps_rollup"))
        .ok()
        .and_then(|text| parse_pss(&text));
    Some(Sample {
        pid,
        comm: parsed.comm,
        ticks: parsed.ticks,
        rss_kib,
        pss_kib,
    })
}

/// The fields of `/proc/<pid>/stat` the baseline reads.
#[derive(Debug, PartialEq, Eq)]
struct Stat {
    comm: String,
    group: u32,
    ticks: u64,
}

/// Parses one `stat` line. The program name is in parentheses and may itself hold spaces and parentheses, so the fields
/// are counted from the last closing one.
fn parse_stat(line: &str) -> Option<Stat> {
    let open = line.find('(')?;
    let close = line.rfind(')')?;
    let comm = line.get(open + 1..close)?.to_owned();
    // After the name: state, ppid, pgrp, session, tty_nr, tpgid, flags, minflt, cminflt, majflt, cmajflt, utime, stime.
    let rest: Vec<&str> = line.get(close + 1..)?.split_whitespace().collect();
    let group = rest.get(2)?.parse().ok()?;
    let utime: u64 = rest.get(11)?.parse().ok()?;
    let stime: u64 = rest.get(12)?.parse().ok()?;
    Some(Stat {
        comm,
        group,
        ticks: utime + stime,
    })
}

/// The `Pss:` line of an `smaps_rollup` file, in KiB.
fn parse_pss(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix("Pss:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|number| number.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stat_line_is_read_from_the_last_closing_parenthesis() {
        let line = "4242 (weird (name) x) S 1 777 777 0 -1 4194560 100 0 0 0 30 12 0 0 20 0 1 0 5 1000 50 18446744073709551615";
        let stat = parse_stat(line).expect("a stat line");
        assert_eq!(stat.comm, "weird (name) x");
        assert_eq!(stat.group, 777);
        assert_eq!(stat.ticks, 42, "user 30 plus system 12");
    }

    #[test]
    fn a_truncated_stat_line_is_not_a_sample() {
        assert_eq!(parse_stat("1 (init) S 0 1"), None);
    }

    #[test]
    fn pss_is_read_from_the_rollup() {
        let text = "00400000-ffffffff ---p 00000000 00:00 0 [rollup]\nRss:        1000 kB\nPss:         640 kB\nShared_Clean: 3 kB\n";
        assert_eq!(parse_pss(text), Some(640));
        assert_eq!(parse_pss("Rss: 1 kB\n"), None);
    }

    #[test]
    fn this_process_is_a_member_of_its_own_group() {
        let own = rustix::process::getpgrp();
        let pgid = u32::try_from(own.as_raw_nonzero().get()).expect("a process group id");
        let found = members(pgid).into_iter().any(|sample| sample.pid == std::process::id());
        assert!(found, "the test process is in its own process group");
    }
}
