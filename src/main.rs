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

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let mut api_result: ApiResult;
    if args.full {
        api_result = MatchClient::get_matches(None).await;
    } else {
        api_result =
            MatchClient::get_matches(Some(chrono::offset::Utc::now().date_naive().to_string()))
                .await;
    }

    if let Some(matches) = api_result.matches.as_mut() {
        ScoreHelper::set_home_and_away_score(matches);
        MatchClient::save_matches_to_sqlite(matches).await;
    }
}
