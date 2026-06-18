extern crate core;

mod api;
mod service;

use crate::api::match_client::{ApiResult, MatchClient};
use crate::service::score_helper::ScoreHelper;
use clap::Parser;

#[derive(Parser)]
struct Args {
    /// full import (no date filter)
    #[arg(short = 'f', long = "full")]
    full: bool,
}

/// Inclusive `(dateFrom, dateTo)` window for the per-minute incremental import.
/// Spans `today-1 .. today+1` (UTC) so a match still in play across the 00:00 UTC
/// rollover stays inside the queried range and keeps updating to its final score,
/// instead of freezing at its last pre-midnight state.
fn incremental_window(today: chrono::NaiveDate) -> (String, String) {
    let one_day = chrono::Duration::days(1);
    ((today - one_day).to_string(), (today + one_day).to_string())
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let api_result: Option<ApiResult> = if args.full {
        MatchClient::get_matches(None).await
    } else {
        MatchClient::get_matches(Some(incremental_window(
            chrono::offset::Utc::now().date_naive(),
        )))
        .await
    };

    let mut api_result = match api_result {
        Some(result) => result,
        None => {
            eprintln!("macht-api: no usable upstream data — aborting import without changes");
            return;
        }
    };

    if let Some(matches) = api_result.matches.as_mut() {
        ScoreHelper::set_home_and_away_score(matches);
        MatchClient::save_matches_to_sqlite(matches).await;
    }
}

#[cfg(test)]
mod tests {
    use super::incremental_window;
    use chrono::NaiveDate;

    #[test]
    fn incremental_window_spans_yesterday_to_tomorrow() {
        let today = NaiveDate::from_ymd_opt(2026, 6, 18).expect("valid date");
        let (from, to) = incremental_window(today);
        assert_eq!(from, "2026-06-17");
        assert_eq!(to, "2026-06-19");
    }

    #[test]
    fn incremental_window_crosses_month_boundary() {
        let today = NaiveDate::from_ymd_opt(2026, 7, 1).expect("valid date");
        let (from, to) = incremental_window(today);
        assert_eq!(from, "2026-06-30");
        assert_eq!(to, "2026-07-02");
    }
}
