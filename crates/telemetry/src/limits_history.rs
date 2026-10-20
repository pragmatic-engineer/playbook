// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `rate_limits.jsonl`: the readings of the 5 hour and weekly windows over the
//! last days, one JSON line each, so the burn-up of a window can draw the
//! curve it really followed instead of a straight line. The statusline writes
//! it next to `rate_limits.json` (see `limits_source::remember`); only the usage
//! views read it. A line holds a time and two percentages: no account, no
//! token, no session id.
//!
//! The file is small by construction: a line is added at most once in 5
//! minutes and only when a percentage changed, lines older than 8 days go, and
//! the file never passes 256 KiB. The statusline runs this on every render, so
//! the common case reads the first and last line only and appends one line.
//! Everything is best effort: a failure here never reaches the statusline.

use crate::limits::LimitPeriod;
use crate::util::atomic::write_atomic_private;
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// The largest the file may be. A bigger file is ignored by readers and
/// replaced by the next write.
pub const MAX_BYTES: u64 = 256 * 1024;
/// Lines older than this (8 days, the weekly window and a day) are dropped.
pub const KEEP_SECONDS: i64 = 8 * 24 * 3600;
/// The fewest seconds between two lines.
pub const MIN_GAP_SECONDS: i64 = 300;
/// A trim leaves the file at this size, so the next writes append again.
const TRIM_TO_BYTES: u64 = MAX_BYTES / 4 * 3;
/// How much of the start and the end of the file one write looks at: more
/// than the longest line.
const EDGE_BYTES: u64 = 512;

/// One line of the file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    pub t: i64,
    pub five_hour: Option<f64>,
    pub seven_day: Option<f64>,
}

/// `rate_limits.jsonl` in the usage folder.
pub fn history_path(usage_dir: &Path) -> PathBuf {
    usage_dir.join("rate_limits.jsonl")
}

fn percent(v: &Value, key: &str) -> Option<f64> {
    v.get(key)?.as_f64().filter(|p| p.is_finite() && *p >= 0.0)
}

fn parse_line(line: &str) -> Option<Reading> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    Some(Reading {
        t: v.get("t")?.as_i64().filter(|t| *t > 0)?,
        five_hour: percent(&v, "five_hour"),
        seven_day: percent(&v, "seven_day"),
    })
}

fn line_of(r: &Reading) -> String {
    format!(
        "{}\n",
        json!({"t": r.t, "five_hour": r.five_hour, "seven_day": r.seven_day})
    )
}

/// The first line, the last line, and whether the file ends in a newline,
/// from two short reads. `None` for a part that is not a whole, valid line.
fn edges(path: &Path, len: u64) -> Option<(Option<Reading>, Option<Reading>, bool)> {
    let mut f = fs::File::open(path).ok()?;
    let mut head = vec![0; usize::try_from(len.min(EDGE_BYTES)).ok()?];
    f.read_exact(&mut head).ok()?;
    let first = std::str::from_utf8(&head)
        .ok()
        .and_then(|s| s.split_once('\n'))
        .and_then(|(line, _)| parse_line(line));
    let from = len.saturating_sub(EDGE_BYTES);
    f.seek(SeekFrom::Start(from)).ok()?;
    let mut tail = vec![0; usize::try_from(len - from).ok()?];
    f.read_exact(&mut tail).ok()?;
    let ends_clean = tail.last() == Some(&b'\n');
    let text = String::from_utf8_lossy(&tail);
    let body = text.trim_end_matches('\n');
    let last = match body.rsplit_once('\n') {
        Some((_, line)) => parse_line(line),
        // Only a whole file in the window is a whole line.
        None if from == 0 => parse_line(body),
        None => None,
    };
    Some((first, last, ends_clean))
}

/// Adds a line for `five_hour` and `seven_day` read at `now`, when the last
/// line is at least [`MIN_GAP_SECONDS`] older and a percentage changed. Drops
/// old lines and keeps the file under [`MAX_BYTES`]. Does nothing when both
/// are `None`.
pub fn record(usage_dir: &Path, five_hour: Option<f64>, seven_day: Option<f64>, now: i64) {
    if five_hour.is_none() && seven_day.is_none() {
        return;
    }
    let new = Reading {
        t: now,
        five_hour,
        seven_day,
    };
    let _ = append(&history_path(usage_dir), &new);
}

fn append(path: &Path, new: &Reading) -> std::io::Result<()> {
    let line = line_of(new);
    let len = match fs::metadata(path) {
        Ok(m) if m.is_file() => m.len(),
        Ok(_) => return Ok(()),
        Err(_) => 0,
    };
    if len == 0 || len > MAX_BYTES {
        // New, or too big to trust: start again from this line.
        return write_atomic_private(path, line);
    }
    let Some((first, last, ends_clean)) = edges(path, len) else {
        return Ok(());
    };
    if let Some(last) = last {
        let unchanged = last.five_hour == new.five_hour && last.seven_day == new.seven_day;
        if new.t - last.t < MIN_GAP_SECONDS || unchanged {
            return Ok(());
        }
    }
    let cutoff = new.t - KEEP_SECONDS;
    let old = first.is_some_and(|f| f.t < cutoff);
    if ends_clean && !old && len + line.len() as u64 <= MAX_BYTES {
        let mut f = OpenOptions::new().append(true).open(path)?;
        return f.write_all(line.as_bytes());
    }
    trim_and_append(path, cutoff, &line)
}

/// Rewrites the file without the lines older than `cutoff`, and without the
/// oldest ones until it fits [`TRIM_TO_BYTES`], then adds `line`.
fn trim_and_append(path: &Path, cutoff: i64, line: &str) -> std::io::Result<()> {
    let text = String::from_utf8_lossy(&fs::read(path)?).into_owned();
    let mut kept: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    // The lines are in time order, so the old ones are at the front.
    let old = kept
        .iter()
        .take_while(|l| parse_line(l).is_none_or(|r| r.t < cutoff))
        .count();
    kept.drain(..old);
    let mut size: u64 = kept.iter().map(|l| l.len() as u64 + 1).sum::<u64>() + line.len() as u64;
    let mut skip = 0;
    while size > TRIM_TO_BYTES && skip < kept.len() {
        size -= kept[skip].len() as u64 + 1;
        skip += 1;
    }
    let mut out = String::with_capacity(size as usize);
    for l in &kept[skip..] {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str(line);
    replace(path, out.as_bytes())
}

/// Replaces the file through an owner-only temp file and a rename, without the
/// flush to disk that `write_atomic_private` does. A trim runs inside the
/// statusline, the history can be lost with the machine, and nothing else
/// depends on it.
fn replace(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(".rate_limits.jsonl.{}.tmp", std::process::id()));
    let _ = fs::remove_file(&tmp);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = options
        .open(&tmp)
        .and_then(|mut f| f.write_all(content))
        .and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// The readings in the file as `(window, time, percent)`, oldest first. An
/// unreadable or oversized file gives none, and a line that is not valid is
/// skipped.
pub fn load(usage_dir: &Path) -> Vec<(LimitPeriod, i64, f64)> {
    let path = history_path(usage_dir);
    let Ok(meta) = fs::metadata(&path) else {
        return Vec::new();
    };
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return Vec::new();
    }
    let Ok(bytes) = fs::read(&path) else {
        return Vec::new();
    };
    let mut out: Vec<(LimitPeriod, i64, f64)> = String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(parse_line)
        .flat_map(|r| {
            [
                r.five_hour.map(|p| (LimitPeriod::FiveHour, r.t, p)),
                r.seven_day.map(|p| (LimitPeriod::SevenDay, r.t, p)),
            ]
        })
        .flatten()
        .collect();
    out.sort_by_key(|(_, t, _)| *t);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pb-history-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn lines(d: &Path) -> Vec<String> {
        fs::read_to_string(history_path(d))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    fn times(d: &Path) -> Vec<i64> {
        lines(d)
            .iter()
            .filter_map(|l| parse_line(l))
            .map(|r| r.t)
            .collect()
    }

    #[test]
    fn a_line_holds_only_the_time_and_the_two_percentages() {
        let d = dir("shape");

        record(&d, Some(12.5), None, NOW);

        assert_eq!(
            lines(&d),
            vec![format!(
                r#"{{"t":{NOW},"five_hour":12.5,"seven_day":null}}"#
            )]
        );
    }

    #[test]
    fn two_readings_a_minute_apart_leave_one_line() {
        let d = dir("minute");

        record(&d, Some(10.0), Some(20.0), NOW);
        record(&d, Some(11.0), Some(20.0), NOW + 60);

        assert_eq!(times(&d), vec![NOW]);
    }

    #[test]
    fn the_gap_rule_is_299_no_300_yes_and_an_equal_value_never() {
        let d = dir("gap");
        record(&d, Some(10.0), None, NOW);

        record(&d, Some(11.0), None, NOW + 299);
        assert_eq!(times(&d), vec![NOW]);
        record(&d, Some(11.0), None, NOW + 300);
        assert_eq!(times(&d), vec![NOW, NOW + 300]);
        record(&d, Some(11.0), None, NOW + 300 + 301);
        assert_eq!(times(&d), vec![NOW, NOW + 300]);
        record(&d, Some(12.0), Some(1.0), NOW + 300 + 301);
        assert_eq!(times(&d), vec![NOW, NOW + 300, NOW + 601]);
    }

    #[test]
    fn a_change_in_the_other_window_alone_is_a_change() {
        let d = dir("other");
        record(&d, Some(10.0), Some(20.0), NOW);

        record(&d, Some(10.0), Some(21.0), NOW + 300);

        assert_eq!(times(&d), vec![NOW, NOW + 300]);
    }

    #[test]
    fn nothing_is_written_without_a_window() {
        let d = dir("none");

        record(&d, None, None, NOW);

        assert!(!history_path(&d).exists());
    }

    #[test]
    fn lines_older_than_8_days_are_pruned_at_the_next_append() {
        let d = dir("prune");
        let old = NOW - KEEP_SECONDS - 1;
        let edge = NOW - KEEP_SECONDS;
        let body = format!(
            "{}{}{}",
            line_of(&Reading {
                t: old,
                five_hour: Some(1.0),
                seven_day: None
            }),
            line_of(&Reading {
                t: edge,
                five_hour: Some(2.0),
                seven_day: None
            }),
            line_of(&Reading {
                t: NOW - 600,
                five_hour: Some(3.0),
                seven_day: None
            }),
        );
        fs::write(history_path(&d), body).unwrap();

        record(&d, Some(4.0), None, NOW);

        assert_eq!(times(&d), vec![edge, NOW - 600, NOW]);
    }

    #[test]
    fn a_corrupt_line_is_skipped_by_readers_and_does_not_block_a_write() {
        let d = dir("corrupt");
        let good = line_of(&Reading {
            t: NOW - 1000,
            five_hour: Some(5.0),
            seven_day: None,
        });
        fs::write(history_path(&d), format!("not json\n{good}{{\"t\":\n")).unwrap();

        record(&d, Some(6.0), None, NOW);

        let got = load(&d);
        assert_eq!(
            got,
            vec![
                (LimitPeriod::FiveHour, NOW - 1000, 5.0),
                (LimitPeriod::FiveHour, NOW, 6.0)
            ]
        );
    }

    #[test]
    fn a_file_over_the_cap_is_ignored_by_readers_and_replaced_by_the_next_write() {
        let d = dir("oversize");
        let big = line_of(&Reading {
            t: NOW - 1000,
            five_hour: Some(5.0),
            seven_day: None,
        });
        fs::write(
            history_path(&d),
            big.repeat(MAX_BYTES as usize / big.len() + 2),
        )
        .unwrap();
        assert!(fs::metadata(history_path(&d)).unwrap().len() > MAX_BYTES);
        assert!(load(&d).is_empty());

        record(&d, Some(6.0), None, NOW);

        assert_eq!(times(&d), vec![NOW]);
        assert_eq!(load(&d), vec![(LimitPeriod::FiveHour, NOW, 6.0)]);
    }

    #[test]
    fn a_full_file_is_trimmed_from_the_front_and_never_passes_the_cap() {
        let d = dir("full");
        let one = |t: i64| {
            line_of(&Reading {
                t,
                five_hour: Some(5.0),
                seven_day: Some(7.0),
            })
        };
        let n = MAX_BYTES as usize / one(NOW - 1000).len();
        let body: String = (0..n as i64)
            .map(|i| one(NOW - 1000 - (n as i64 - i)))
            .collect();
        fs::write(history_path(&d), &body).unwrap();
        let before = fs::metadata(history_path(&d)).unwrap().len();
        assert!(before <= MAX_BYTES && before + 80 > MAX_BYTES, "{before}");

        record(&d, Some(6.0), Some(7.0), NOW);

        let after = fs::metadata(history_path(&d)).unwrap().len();
        assert!(after <= TRIM_TO_BYTES, "{after}");
        let t = times(&d);
        assert_eq!(t.last(), Some(&NOW));
        assert!(t.windows(2).all(|w| w[0] < w[1]));
        assert!(t.len() > 1000);
        // the next writes append again, without a rewrite
        record(&d, Some(7.0), Some(7.0), NOW + 300);
        assert_eq!(times(&d).last(), Some(&(NOW + 300)));
    }

    #[test]
    fn a_torn_last_line_is_cleaned_by_the_next_write() {
        let d = dir("torn");
        let good = line_of(&Reading {
            t: NOW - 1000,
            five_hour: Some(5.0),
            seven_day: None,
        });
        fs::write(history_path(&d), format!("{good}{{\"t\":17")).unwrap();

        record(&d, Some(6.0), None, NOW);

        let all = lines(&d);
        assert_eq!(all.len(), 3);
        assert_eq!(times(&d), vec![NOW - 1000, NOW]);
    }

    #[test]
    fn the_cap_and_the_age_are_the_documented_bounds() {
        assert_eq!(MAX_BYTES, 262_144);
        assert_eq!(KEEP_SECONDS, 691_200);
        assert_eq!(MIN_GAP_SECONDS, 300);
    }

    #[test]
    fn an_unwritable_folder_is_ignored() {
        let d = dir("ro");
        let missing = d.join("nope/deeper");

        record(&missing, Some(1.0), None, NOW);

        assert!(!history_path(&missing).exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&d, fs::Permissions::from_mode(0o500)).unwrap();
            // a user that can write anyway (root) is not the case under test
            let can_write = fs::write(d.join("probe"), "x").is_ok();
            record(&d, Some(1.0), None, NOW);
            let wrote = history_path(&d).exists();
            fs::set_permissions(&d, fs::Permissions::from_mode(0o700)).unwrap();
            assert!(can_write || !wrote);
        }
    }

    #[test]
    fn a_new_file_is_private() {
        let d = dir("private");

        record(&d, Some(1.0), None, NOW);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(history_path(&d)).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "{mode:o}");
        }
    }

    #[test]
    fn a_full_history_adds_at_most_5_ms_to_a_remember_call() {
        use std::time::{Duration, Instant};
        let reading = format!(
            r#"{{"rate_limits":{{"five_hour":{{"used_percentage":55,"resets_at":{}}}}}}}"#,
            NOW + 3600
        );
        let one = |t: i64| {
            line_of(&Reading {
                t,
                five_hour: Some(1.0),
                seven_day: Some(2.0),
            })
        };
        let n = MAX_BYTES as usize / one(NOW - 100_000).len();
        let body: String = (0..n as i64).map(|i| one(NOW - 100_000 + i)).collect();
        let best = |with_history: bool| {
            (0..7)
                .map(|i| {
                    let d = dir(&format!("timing-{with_history}-{i}"));
                    if with_history {
                        fs::write(history_path(&d), &body).unwrap();
                    } else {
                        // a folder in the file's place: the history step is a no-op
                        fs::create_dir(history_path(&d)).unwrap();
                    }
                    let start = Instant::now();
                    crate::limits_source::remember(&d, &reading, NOW);
                    start.elapsed()
                })
                .min()
                .unwrap()
        };

        let (with, without) = (best(true), best(false));

        assert!(
            with <= without + Duration::from_millis(5),
            "with {with:?}, without {without:?}"
        );
    }
}
