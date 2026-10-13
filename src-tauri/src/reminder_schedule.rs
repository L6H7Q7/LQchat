//! Calendar rules matching Android CustomReminderSchedule. No network/UI or OS notification access.
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reminder {
    pub id: String,
    pub title: String,
    pub content: String,
    pub mode: String,
    #[serde(default)]
    pub date: String,
    pub time: String,
    #[serde(default)]
    pub weekday: u32,
    #[serde(default = "one_u32")]
    pub monthday: u32,
    #[serde(default)]
    pub weekdays: Vec<u32>,
    #[serde(default)]
    pub monthdays: Vec<u32>,
    #[serde(default)]
    pub countdown: bool,
    #[serde(default = "one_i64")]
    pub day_gap: i64,
    #[serde(default = "day_time")]
    pub day_time: String,
    #[serde(default)]
    pub near: bool,
    #[serde(default = "near_start")]
    pub near_start: i64,
    #[serde(default = "near_gap")]
    pub near_gap: i64,
}
fn one_u32() -> u32 {
    1
}
fn one_i64() -> i64 {
    1
}
fn day_time() -> String {
    "09:00".into()
}
fn near_start() -> i64 {
    60
}
fn near_gap() -> i64 {
    10
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Options {
    pub repeat: u32,
    pub repeat_gap: i64,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            repeat: 1,
            repeat_gap: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Push {
    pub reminder: Reminder,
    pub event_at: i64,
    pub at: i64,
    pub kind: &'static str,
    pub index: u32,
}
impl Push {
    pub fn key(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}",
            self.reminder.id, self.event_at, self.kind, self.at, self.index
        )
    }
    pub fn text(&self) -> String {
        if self.kind == "event" {
            self.reminder.content.clone()
        } else {
            format!(
                "距离下次{}还有{}",
                self.reminder.title,
                remaining(self.event_at, self.at)
            )
        }
    }
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({"id":self.reminder.id,"key":self.key(),"title":self.reminder.title,
            "content":self.text(),"at":self.at,"eventAt":self.event_at,"kind":self.kind,"index":self.index})
    }
}
pub fn valid_time(value: &str) -> bool {
    value.len() == 5
        && value.as_bytes()[2] == b':'
        && NaiveTime::parse_from_str(value, "%H:%M").is_ok()
}
pub fn parse_date(value: &str) -> Option<NaiveDate> {
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
    (date.format("%Y-%m-%d").to_string() == value).then_some(date)
}
pub fn validate(reminders: &[Reminder], options: &Options) -> Result<(), String> {
    if !(1..=20).contains(&options.repeat)
        || !(options.repeat == 1 && options.repeat_gap == 0
            || options.repeat > 1 && (1..=3600).contains(&options.repeat_gap))
    {
        return Err("通知次数或间隔无效".into());
    }
    let mut ids = HashSet::new();
    if reminders.len() > 200 {
        return Err("最多保存 200 条通知".into());
    }
    for r in reminders {
        if r.id.is_empty()
            || r.id.len() > 80
            || !r
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
            || !ids.insert(&r.id)
            || r.title.trim().is_empty()
            || r.content.trim().is_empty()
            || r.title.encode_utf16().count() > 60
            || r.content.encode_utf16().count() > 300
        {
            return Err("通知标题、内容或标识无效".into());
        }
        if !matches!(
            r.mode.as_str(),
            "once" | "daily" | "weekly" | "monthly" | "weekdays" | "monthdays"
        ) || !valid_time(&r.time)
            || !valid_time(&r.day_time)
            || r.mode == "once" && parse_date(&r.date).is_none()
            || r.mode == "weekly" && r.weekday > 6
            || r.mode == "monthly" && !(1..=31).contains(&r.monthday)
            || r.weekdays.iter().any(|d| *d > 6)
            || r.monthdays.iter().any(|d| !(1..=31).contains(d))
            || r.mode == "weekdays" && r.weekdays.is_empty()
            || r.mode == "monthdays" && r.monthdays.is_empty()
            || !(1..=365).contains(&r.day_gap)
            || !(1..=10080).contains(&r.near_start)
            || !(1..=1440).contains(&r.near_gap)
            || r.countdown && r.near && r.near_gap > r.near_start
        {
            return Err("提醒日期、时间或倒计时规则无效".into());
        }
    }
    Ok(())
}
fn at_time<T: TimeZone>(date: NaiveDate, time: &str, zone: &T) -> Option<i64> {
    let mut local = date.and_time(NaiveTime::parse_from_str(time, "%H:%M").ok()?);
    // Android Calendar moves a wall time in a DST gap forward; ambiguous times use the first occurrence.
    for _ in 0..=120 {
        if let Some(at) = zone.from_local_datetime(&local).earliest() {
            return Some(at.timestamp_millis());
        }
        local += Duration::minutes(1);
    }
    None
}
fn from_time<T: TimeZone>(at: i64, zone: &T) -> Option<DateTime<T>> {
    zone.timestamp_millis_opt(at).single()
}
fn matches_date(r: &Reminder, date: NaiveDate) -> bool {
    let weekday = date.weekday().num_days_from_sunday();
    match r.mode.as_str() {
        "daily" => true,
        "once" => Some(date) == parse_date(&r.date),
        "weekly" => weekday == r.weekday,
        "monthly" => date.day() == r.monthday,
        "weekdays" => r.weekdays.contains(&weekday),
        "monthdays" => r.monthdays.contains(&date.day()),
        _ => false,
    }
}
pub fn next_event<T: TimeZone>(r: &Reminder, from: i64, zone: &T) -> Option<i64> {
    if r.mode == "once" {
        return at_time(parse_date(&r.date)?, &r.time, zone).filter(|at| *at >= from);
    }
    let mut date = from_time(from, zone)?.date_naive();
    for _ in 0..370 {
        if matches_date(r, date) {
            if let Some(at) = at_time(date, &r.time, zone).filter(|at| *at >= from) {
                return Some(at);
            }
        }
        date = date.succ_opt()?;
    }
    None
}
pub fn remaining(event: i64, at: i64) -> String {
    let minutes = ((event - at).max(1) + 59999) / 60000;
    [
        (minutes / 1440, "天"),
        (minutes % 1440 / 60, "小时"),
        (minutes % 60, "分钟"),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, unit)| format!("{n}{unit}"))
    .collect::<Vec<_>>()
    .join(" ")
}
pub fn next<T: TimeZone>(
    reminders: &[Reminder],
    opt: &Options,
    from: i64,
    delivered: &HashSet<String>,
    zone: &T,
) -> Vec<Push> {
    if opt.repeat == 0 {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    let span = (opt.repeat - 1) as i64 * opt.repeat_gap * 1000;
    let mut offer = |r: &Reminder, event: i64, base: i64, kind: &'static str, end: i64| {
        for index in 1..=opt.repeat {
            let at = base + (index - 1) as i64 * opt.repeat_gap * 1000;
            let push = Push {
                reminder: r.clone(),
                event_at: event,
                at,
                kind,
                index,
            };
            if at >= from && at < end && !delivered.contains(&push.key()) {
                candidates.push(push);
            }
        }
    };
    for r in reminders {
        let mut event = next_event(r, from - span, zone);
        while let Some(previous) = event.filter(|at| *at < from) {
            offer(r, previous, previous, "event", i64::MAX);
            event = next_event(r, previous + 1, zone);
        }
        let Some(event) = event else {
            continue;
        };
        offer(r, event, event, "event", i64::MAX);
        if !r.countdown {
            continue;
        }
        let near_at = if r.near {
            event - r.near_start * 60000
        } else {
            event
        };
        let event_date = from_time(event, zone).unwrap().date_naive();
        let today = from_time(from, zone).unwrap().date_naive();
        let intervals = (event_date - today).num_days().max(0) / r.day_gap;
        let Some(mut date) =
            event_date.checked_sub_signed(Duration::days((intervals - 1).max(0) * r.day_gap))
        else {
            continue;
        };
        for _ in 0..4000 {
            let Some(at) = at_time(date, &r.day_time, zone) else {
                break;
            };
            if at < from - span - 2 * 86400000 {
                break;
            }
            if at < near_at {
                offer(r, event, at, "countdown", near_at);
            }
            let Some(previous) = date.checked_sub_signed(Duration::days(r.day_gap)) else {
                break;
            };
            date = previous;
        }
        if r.near {
            let interval = r.near_gap * 60000;
            let first = (from - span - near_at).div_euclid(interval).max(0);
            let last = ((from - near_at) as f64 / interval as f64).ceil().max(0.0) as i64;
            for i in first..=last {
                let at = near_at + i * interval;
                if at < event {
                    offer(r, event, at, "near", event);
                }
            }
        }
    }
    let Some(earliest) = candidates.iter().map(|p| p.at).min() else {
        return Vec::new();
    };
    candidates.retain(|p| p.at == earliest);
    candidates.sort_by_key(Push::key);
    candidates.dedup_by_key(|p| p.key());
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;
    fn zone() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).unwrap()
    }
    fn at(text: &str) -> i64 {
        zone()
            .from_local_datetime(
                &chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").unwrap(),
            )
            .unwrap()
            .timestamp_millis()
    }
    fn sample() -> Reminder {
        serde_json::from_value(serde_json::json!({"id":"harvest","title":"收菜","content":"记得收菜","mode":"once",
        "date":"2026-10-06","time":"18:00","weekday":6,"monthday":3,"weekdays":[1,3,5],"monthdays":[1,15],
        "countdown":true,"dayGap":1,"dayTime":"09:00","near":true,"nearStart":60,"nearGap":10})).unwrap()
    }
    fn first(r: Reminder, text: &str) -> Option<Push> {
        next(
            &[r],
            &Options {
                repeat: 3,
                repeat_gap: 5,
            },
            at(text),
            &HashSet::new(),
            &zone(),
        )
        .into_iter()
        .next()
    }
    #[test]
    fn android_parity_calendar_modes_and_leap_days() {
        let date = parse_date("2026-10-03").unwrap();
        let mut r = sample();
        for mode in ["daily", "weekly"] {
            r.mode = mode.into();
            assert!(matches_date(&r, date));
        }
        r.mode = "weekdays".into();
        assert!(!matches_date(&r, date));
        r.mode = "monthdays".into();
        r.monthdays = vec![3];
        assert!(matches_date(&r, date));
        r.mode = "monthly".into();
        r.monthday = 31;
        assert_eq!(
            next_event(&r, at("2026-04-01 00:00:00"), &zone()),
            Some(at("2026-05-31 18:00:00"))
        );
        assert!(parse_date("2026-02-30").is_none());
        r.mode = "once".into();
        r.date = "2028-02-29".into();
        assert_eq!(
            next_event(&r, at("2026-10-03 12:00:00"), &zone()),
            Some(at("2028-02-29 18:00:00"))
        );
    }
    #[test]
    fn android_parity_countdown_near_formal_and_repeats() {
        for (now, expected, kind, index) in [
            ("2026-10-03 12:00:00", "2026-10-04 09:00:00", "countdown", 1),
            ("2026-10-06 17:23:00", "2026-10-06 17:30:00", "near", 1),
            ("2026-10-06 17:59:00", "2026-10-06 18:00:00", "event", 1),
            ("2026-10-06 18:00:02", "2026-10-06 18:00:05", "event", 2),
            ("2026-10-06 17:20:02", "2026-10-06 17:20:05", "near", 2),
        ] {
            let p = first(sample(), now).unwrap();
            assert_eq!((p.at, p.kind, p.index), (at(expected), kind, index));
        }
        assert!(first(sample(), "2026-10-06 18:00:11").is_none());
        let second = first(sample(), "2026-10-06 18:00:02").unwrap();
        let result = next(
            &[sample()],
            &Options {
                repeat: 3,
                repeat_gap: 5,
            },
            second.at,
            &HashSet::from([second.key()]),
            &zone(),
        );
        assert_eq!(result[0].index, 3);
    }
    #[test]
    fn android_parity_ties_long_intervals_and_far_future() {
        let mut r = sample();
        r.countdown = false;
        let mut other = r.clone();
        other.id = "other".into();
        let from = at("2026-10-06 17:59:00");
        assert_eq!(
            next(
                &[r.clone(), other],
                &Options::default(),
                from,
                &HashSet::new(),
                &zone()
            )
            .len(),
            2
        );
        let opts = Options {
            repeat: 20,
            repeat_gap: 3600,
        };
        let result = next(
            &[r.clone()],
            &opts,
            at("2026-10-06 18:00:01"),
            &HashSet::new(),
            &zone(),
        );
        assert_eq!(
            (result[0].at, result[0].index),
            (at("2026-10-06 19:00:00"), 2)
        );
        r.mode = "daily".into();
        assert_eq!(
            first(r, "2026-10-06 18:01:00").unwrap().at,
            at("2026-10-07 18:00:00")
        );
        r = sample();
        r.date = "2050-10-06".into();
        r.near = false;
        assert_eq!(
            first(r, "2026-10-03 12:00:00").unwrap().at,
            at("2026-10-04 09:00:00")
        );
        r = sample();
        r.day_gap = 2;
        r.near = false;
        assert_eq!(
            first(r, "2026-10-03 12:00:00").unwrap().at,
            at("2026-10-04 09:00:00")
        );
    }
    #[test]
    fn native_validation_rejects_invalid_rules() {
        let mut r = sample();
        assert!(validate(&[r.clone()], &Options::default()).is_ok());
        r.near_gap = 61;
        assert!(validate(&[r.clone()], &Options::default()).is_err());
        r = sample();
        r.mode = "weekdays".into();
        r.weekdays.clear();
        assert!(validate(&[r], &Options::default()).is_err());
        assert!(validate(
            &[],
            &Options {
                repeat: 2,
                repeat_gap: 0
            }
        )
        .is_err());
        assert!(!valid_time("24:00"));
        assert!(!valid_time("9:00"));
    }
}
