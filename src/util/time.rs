use chrono::{DateTime, Utc};

pub fn now_utc() -> DateTime<Utc> {
    Utc::now()
}

pub fn now_iso8601() -> String {
    now_utc().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn format_timestamp_yyyymmddhhmmss(dt: DateTime<Utc>) -> String {
    dt.format("%Y%m%d%H%M%S").to_string()
}

pub fn now_timestamp_yyyymmddhhmmss() -> String {
    format_timestamp_yyyymmddhhmmss(now_utc())
}
