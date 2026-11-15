//! Time buckets for the zoomed-out view: 本週, 上週, 本月, 上個月, then
//! seasons (春 Mar–May, 夏 Jun–Aug, 秋 Sep–Nov, 冬 Dec–Feb, labelled by the
//! year the season starts) for the last two years, then whole years.
//! Buckets are disjoint, newest first, empty ones left out. Each item is
//! placed by its best time: downloaded > document created > file created.

use crate::index::Index;
use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, TimeZone};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Bucket {
    pub label: String,
    /// RFC 3339, inclusive.
    pub from: String,
    /// RFC 3339, exclusive.
    pub to: String,
    pub count: usize,
    /// The newest item in the bucket.
    pub latest_id: String,
}

fn at(tz: &FixedOffset, d: NaiveDate) -> DateTime<FixedOffset> {
    tz.from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap()).single().unwrap_or_else(|| tz.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap()))
}

fn month_start(y: i32, m: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, 1).unwrap()
}

fn prev_month(d: NaiveDate) -> NaiveDate {
    if d.month() == 1 { month_start(d.year() - 1, 12) } else { month_start(d.year(), d.month() - 1) }
}

/// (label, start) of the season containing `d`.
fn season(d: NaiveDate) -> (String, NaiveDate, NaiveDate) {
    let (y, m) = (d.year(), d.month());
    let (label_year, name, start, end) = match m {
        3..=5 => (y, "春", month_start(y, 3), month_start(y, 6)),
        6..=8 => (y, "夏", month_start(y, 6), month_start(y, 9)),
        9..=11 => (y, "秋", month_start(y, 9), month_start(y, 12)),
        12 => (y, "冬", month_start(y, 12), month_start(y + 1, 3)),
        _ => (y - 1, "冬", month_start(y - 1, 12), month_start(y, 3)),
    };
    (format!("{label_year} {name}"), start, end)
}

/// The buckets of every item in `idx`, relative to `now`.
pub fn buckets(idx: &Index, now: DateTime<FixedOffset>, filter: &lore_catalog::category::Filter) -> Vec<Bucket> {
    let tz = *now.offset();
    let today = now.date_naive();
    let week = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    let last_week = week - Duration::days(7);
    let month = month_start(today.year(), today.month());
    let last_month = prev_month(month);
    let season_floor = month_start(today.year() - 2, 1);

    // (label, from, to, count, latest (time, id))
    type Acc = (String, DateTime<FixedOffset>, DateTime<FixedOffset>, usize, Option<(DateTime<FixedOffset>, String)>);
    let mut out: Vec<Acc> = Vec::new();
    for e in idx.entries.iter().filter(|e| filter.accepts(&e.item.categories)) {
        let t = &e.item.times;
        let Some(v) = t.downloaded.as_ref().or(t.doc_created.as_ref()).or(t.created.as_ref()) else { continue };
        let Ok(when) = DateTime::parse_from_rfc3339(v) else { continue };
        let when = when.with_timezone(&tz);
        let d = when.date_naive();
        let (label, from, to) = if d >= week {
            ("本週".to_string(), at(&tz, week), at(&tz, week + Duration::days(7)))
        } else if d >= last_week {
            ("上週".into(), at(&tz, last_week), at(&tz, week))
        } else if d >= month {
            ("本月".into(), at(&tz, month), at(&tz, last_week))
        } else if d >= last_month {
            ("上個月".into(), at(&tz, last_month), at(&tz, month.min(last_week)))
        } else if d >= season_floor {
            let (l, s, e) = season(d);
            (l, at(&tz, s.max(season_floor)), at(&tz, e.min(last_month)))
        } else {
            (format!("{}", d.year()), at(&tz, month_start(d.year(), 1)), at(&tz, month_start(d.year() + 1, 1).min(season_floor)))
        };
        match out.iter_mut().find(|b| b.0 == label) {
            Some(b) => {
                b.3 += 1;
                if b.4.as_ref().is_none_or(|(lt, _)| when > *lt) {
                    b.4 = Some((when, e.item.id.clone()));
                }
            }
            None => out.push((label, from, to, 1, Some((when, e.item.id.clone())))),
        }
    }
    out.sort_by_key(|b| std::cmp::Reverse(b.1));
    out.into_iter()
        .map(|(label, from, to, count, latest)| Bucket { label, from: from.to_rfc3339(), to: to.to_rfc3339(), count, latest_id: latest.map(|l| l.1).unwrap_or_default() })
        .collect()
}
