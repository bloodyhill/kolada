//! When a note was written, the way people say it, on the services' calendar.
//! The words are shared (`notsy_ui::when`); the calendar is the OS's.

use std::time::SystemTime;

use crate::services::{Calendar, Words};

pub use notsy_ui::when::{count_words, words_phrase};

/// How long ago `t` was, for a file tree row's date ("5 min ago", "Yesterday, 9:10 AM").
pub fn ago(c: &dyn Calendar, t: SystemTime) -> String {
    notsy_ui::when::ago(&Words(c), t)
}

/// When `t` was and how long ago, for the time scrubber's bubble.
pub fn moment(c: &dyn Calendar, t: SystemTime) -> (String, String) {
    notsy_ui::when::moment(&Words(c), t)
}

/// The day of `t`, short: where the scrubber's track begins.
pub fn day(c: &dyn Calendar, t: SystemTime) -> String {
    notsy_ui::when::day(&Words(c), t)
}

/// A note's dates, for the top of its page.
pub fn page_dates(c: &dyn Calendar, created: Option<SystemTime>, modified: Option<SystemTime>) -> Option<String> {
    notsy_ui::when::page_dates(&Words(c), created, modified)
}
