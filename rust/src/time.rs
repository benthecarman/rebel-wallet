pub(crate) fn now_label() -> String {
    chrono::Local::now().format("%b %-d, %-I:%M %p").to_string()
}

pub(crate) fn now_unix() -> u64 {
    chrono::Local::now().timestamp().max(0) as u64
}
