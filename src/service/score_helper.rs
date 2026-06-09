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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::match_client::{Score, ScoreDetail, Team};

    fn team() -> Team {
        Team {
            id: Some(1),
            name: Some("Team".to_string()),
            shortName: None,
            tla: Some("TEA".to_string()),
            flag: None,
        }
    }

    fn match_with(
        full: (Option<isize>, Option<isize>),
        regular: Option<(Option<isize>, Option<isize>)>,
    ) -> Match {
        Match {
            id: 1,
            utcDate: "2026-06-11T19:00:00Z".to_string(),
            homeTeam: team(),
            awayTeam: team(),
            score: Score {
                winner: None,
                duration: "REGULAR".to_string(),
                fullTime: ScoreDetail {
                    home: full.0,
                    away: full.1,
                },
                halfTime: ScoreDetail {
                    home: None,
                    away: None,
                },
                regularTime: regular.map(|(home, away)| ScoreDetail { home, away }),
            },
            status: "FINISHED".to_string(),
            homeScore: None,
            awayScore: None,
        }
    }

    #[test]
    fn uses_full_time_when_no_regular_time() {
        let mut matches = vec![match_with((Some(2), Some(1)), None)];
        ScoreHelper::set_home_and_away_score(&mut matches);
        assert_eq!(matches[0].homeScore, Some(2));
        assert_eq!(matches[0].awayScore, Some(1));
    }

    #[test]
    fn prefers_regular_time_over_full_time() {
        // Knockout decided in extra time: fullTime includes ET, regularTime is
        // the 90-minute score the app scores against.
        let mut matches = vec![match_with((Some(3), Some(2)), Some((Some(1), Some(1))))];
        ScoreHelper::set_home_and_away_score(&mut matches);
        assert_eq!(matches[0].homeScore, Some(1));
        assert_eq!(matches[0].awayScore, Some(1));
    }

    #[test]
    fn carries_nulls_and_handles_multiple_matches() {
        let mut matches = vec![
            match_with((None, None), None),
            match_with((Some(0), Some(0)), None),
        ];
        ScoreHelper::set_home_and_away_score(&mut matches);
        assert_eq!(matches[0].homeScore, None);
        assert_eq!(matches[0].awayScore, None);
        assert_eq!(matches[1].homeScore, Some(0));
        assert_eq!(matches[1].awayScore, Some(0));
    }
}
