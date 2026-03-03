use std::collections::HashSet;
use std::time::SystemTime;

use crate::model::{CronField, CronSpec, DateParts, NEXT_RUN_SCAN_MINUTES, TimeMode};
use crate::platform::{add_seconds, date_parts_from_unix, floor_to_minute, unix_secs};

impl CronSpec {
    pub fn parse(expr: &str) -> Result<Self, String> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err("expected exactly 5 fields".to_string());
        }

        let minutes = parse_cron_field(fields[0], 0, 59)?;
        let hours = parse_cron_field(fields[1], 0, 23)?;
        let dom = parse_cron_field(fields[2], 1, 31)?;
        let months = parse_cron_field(fields[3], 1, 12)?;
        let dow = parse_cron_field(fields[4], 0, 7)?;

        Ok(Self {
            minutes,
            hours,
            dom_any: dom.any,
            dow_any: dow.any,
            dom,
            months,
            dow,
        })
    }

    pub fn matches(&self, dt: DateParts) -> bool {
        if !self.minutes.matches(dt.minute) {
            return false;
        }
        if !self.hours.matches(dt.hour) {
            return false;
        }
        if !self.months.matches(dt.month) {
            return false;
        }

        let dom_match = self.dom.matches(dt.day);
        let wday = if dt.wday == 0 { 0 } else { dt.wday };
        let dow_match = self.dow.matches(wday) || (wday == 0 && self.dow.matches(7));

        if self.dom_any && self.dow_any {
            true
        } else if self.dom_any {
            dow_match
        } else if self.dow_any {
            dom_match
        } else {
            dom_match && dow_match
        }
    }
}

impl CronField {
    pub fn matches(&self, value: u32) -> bool {
        self.any || self.allowed.contains(&value)
    }
}

pub fn parse_cron_field(field: &str, min: u32, max: u32) -> Result<CronField, String> {
    if field == "*" {
        return Ok(CronField {
            allowed: HashSet::new(),
            any: true,
        });
    }

    let mut allowed = HashSet::new();
    for part in field.split(',') {
        parse_cron_part(part, min, max, &mut allowed)?;
    }

    if allowed.is_empty() {
        return Err("empty cron field".to_string());
    }

    Ok(CronField {
        allowed,
        any: false,
    })
}

pub fn parse_cron_part(
    part: &str,
    min: u32,
    max: u32,
    out: &mut HashSet<u32>,
) -> Result<(), String> {
    let (base, step) = if let Some((left, right)) = part.split_once('/') {
        let step = parse_u32(right)?;
        if step == 0 {
            return Err("step cannot be 0".to_string());
        }
        (left, Some(step))
    } else {
        (part, None)
    };

    if base == "*" {
        let s = step.unwrap_or(1);
        let mut v = min;
        while v <= max {
            out.insert(v);
            match v.checked_add(s) {
                Some(next) => v = next,
                None => break,
            }
        }
        return Ok(());
    }

    if let Some((start_raw, end_raw)) = base.split_once('-') {
        let start = parse_u32(start_raw)?;
        let end = parse_u32(end_raw)?;
        if start > end {
            return Err("range start cannot be greater than end".to_string());
        }
        if start < min || end > max {
            return Err(format!("range {start}-{end} out of bounds {min}-{max}"));
        }

        let s = step.unwrap_or(1);
        let mut v = start;
        while v <= end {
            out.insert(v);
            match v.checked_add(s) {
                Some(next) => v = next,
                None => break,
            }
        }
        return Ok(());
    }

    if step.is_some() {
        return Err("steps on a single value are not supported".to_string());
    }

    let value = parse_u32(base)?;
    if value < min || value > max {
        return Err(format!("value {value} out of bounds {min}-{max}"));
    }
    out.insert(value);
    Ok(())
}

pub fn parse_u32(raw: &str) -> Result<u32, String> {
    raw.parse::<u32>()
        .map_err(|_| format!("invalid numeric token '{raw}'"))
}

pub fn find_next_run(cron: &CronSpec, from: SystemTime, mode: TimeMode) -> Option<SystemTime> {
    let mut ts = floor_to_minute(add_seconds(from, 60)?);
    for _ in 0..NEXT_RUN_SCAN_MINUTES {
        let unix = unix_secs(ts)?;
        let parts = date_parts_from_unix(unix, mode)?;
        if cron.matches(parts) {
            return Some(ts);
        }
        ts = add_seconds(ts, 60)?;
    }
    None
}
