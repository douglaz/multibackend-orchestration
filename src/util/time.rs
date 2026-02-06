use chrono::{DateTime, Utc};

pub fn now_utc() -> DateTime<Utc> {
    Utc::now()
}

pub fn now_iso8601() -> String {
    now_utc().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
