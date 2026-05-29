use crate::api::match_client::Match;

pub struct ScoreHelper {}

impl ScoreHelper {
    pub fn set_home_and_away_score(matches: &mut Vec<Match>) {
        for single_match in matches {
            if let Some(regular_time) = &single_match.score.regularTime {
                single_match.homeScore = regular_time.home;
                single_match.awayScore = regular_time.away;
            } else {
                single_match.homeScore = single_match.score.fullTime.home;
                single_match.awayScore = single_match.score.fullTime.away;
            }
        }
    }
}
