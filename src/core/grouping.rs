use super::Meeting;
use chrono::{Local, NaiveDate};
use std::collections::BTreeMap;
#[derive(Clone, Debug)]
pub struct DayGroup {
    pub date: Option<NaiveDate>,
    pub label: String,
    pub meeting_indices: Vec<usize>,
}
pub fn day_label(date: Option<NaiveDate>, today: NaiveDate) -> String {
    match date {
        Some(date) if date == today => "Today".into(),
        Some(date) if Some(date) == today.pred_opt() => "Yesterday".into(),
        Some(date) => date.format("%A, %-d %b").to_string(),
        None => "Date unknown".into(),
    }
}
pub fn day_column(date: Option<NaiveDate>, today: NaiveDate) -> (String, String) {
    match date {
        Some(date) if date == today || Some(date) == today.pred_opt() => (
            day_label(Some(date), today),
            date.format("%a %-d").to_string(),
        ),
        Some(date) => (
            date.format("%A").to_string(),
            date.format("%-d %b").to_string(),
        ),
        None => (day_label(None, today), String::new()),
    }
}
pub fn group_by_day(meetings: &[Meeting]) -> Vec<DayGroup> {
    group_by_day_on(meetings, Local::now().date_naive())
}
pub fn group_by_day_on(meetings: &[Meeting], today: NaiveDate) -> Vec<DayGroup> {
    let mut groups: BTreeMap<Option<NaiveDate>, Vec<usize>> = BTreeMap::new();
    for (i, m) in meetings.iter().enumerate() {
        groups
            .entry(m.started.map(|s| s.date_naive()))
            .or_default()
            .push(i);
    }
    groups
        .into_iter()
        .rev()
        .map(|(date, meeting_indices)| DayGroup {
            date,
            label: day_label(date, today),
            meeting_indices,
        })
        .collect()
}
