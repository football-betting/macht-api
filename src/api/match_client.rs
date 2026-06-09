use chrono::DateTime;
use dotenvy::dotenv;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::to_string;
use std::env;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ApiResult {
    pub matches: Option<Vec<Match>>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Match {
    pub id: isize,
    pub utcDate: String,
    pub homeTeam: Team,
    pub awayTeam: Team,
    pub score: Score,
    pub status: String,
    pub homeScore: Option<isize>,
    pub awayScore: Option<isize>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Team {
    pub id: Option<isize>,
    pub name: Option<String>,
    pub shortName: Option<String>,
    pub tla: Option<String>,
    #[serde(rename = "crest")]
    pub flag: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Score {
    pub winner: Option<String>,
    pub duration: String,
    pub fullTime: ScoreDetail,
    pub halfTime: ScoreDetail,
    pub regularTime: Option<ScoreDetail>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ScoreDetail {
    pub home: Option<isize>,
    pub away: Option<isize>,
}

const TEAM_FIELD_MAX_LEN: usize = 255;

pub struct MatchClient {}

impl MatchClient {
    pub async fn get_matches(date: Option<String>) -> Option<ApiResult> {
        dotenv().ok();

        let mut uri = match env::var("API_URI") {
            Ok(v) => v.to_string(),
            Err(_) => "Error loading env variable API_URI".to_string(),
        };

        uri = match date {
            Some(d) => uri + "?dateFrom=" + d.as_str() + "&dateTo=" + d.as_str(),
            None => uri,
        };

        let token = match env::var("X_AUTH_TOKEN") {
            Ok(v) => v.to_string(),
            Err(_) => "Error loading env variable X_AUTH_TOKEN".to_string(),
        };

        let client = reqwest::Client::new();

        let response = match client.get(uri).header("X-Auth-Token", token).send().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("macht-api: upstream request failed: {e}");
                return None;
            }
        };

        let response = match response.error_for_status() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("macht-api: upstream returned a non-success status: {e}");
                return None;
            }
        };

        match response.json::<ApiResult>().await {
            Ok(result) => Some(result),
            Err(e) => {
                eprintln!("macht-api: failed to parse upstream response: {e}");
                None
            }
        }
    }

    pub async fn save_matches_to_sqlite(matches: &mut [Match]) {
        let db = match Self::get_connection().await {
            Some(db) => db,
            None => return,
        };

        let mut saved = 0usize;
        let mut skipped = 0usize;

        for single_match in matches.iter_mut() {
            let datetime = match DateTime::parse_from_rfc3339(&single_match.utcDate) {
                Ok(dt) => dt,
                Err(e) => {
                    eprintln!(
                        "macht-api: skipping match {}: invalid utcDate {:?}: {e}",
                        single_match.id, single_match.utcDate
                    );
                    skipped += 1;
                    continue;
                }
            };
            let timestamp = datetime.timestamp();

            // Undetermined pairing (e.g. knockout placeholder with empty teams):
            // don't import it. Any earlier placeholders are cleared by a DB reset.
            if !Self::team_determined(&single_match.homeTeam)
                || !Self::team_determined(&single_match.awayTeam)
            {
                skipped += 1;
                continue;
            }

            Self::normalize_team(&mut single_match.homeTeam);
            Self::normalize_team(&mut single_match.awayTeam);

            if let Err(e) = Self::persist_match(&db, single_match, timestamp) {
                eprintln!("macht-api: skipping match {}: {e}", single_match.id);
                skipped += 1;
                continue;
            }
            saved += 1;
        }

        println!("macht-api: import finished — {saved} saved, {skipped} skipped");
    }

    // A team is "determined" once the upstream provides a real name or tla.
    // Knockout placeholders arrive with all fields null/empty.
    fn team_determined(team: &Team) -> bool {
        let has_value =
            |v: &Option<String>| v.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false);
        has_value(&team.name) || has_value(&team.tla)
    }

    fn normalize_team(team: &mut Team) {
        team.name = Some(Self::sanitize_field(team.name.take()));
        team.tla = Some(Self::sanitize_field(team.tla.take()));
    }

    fn sanitize_field(value: Option<String>) -> String {
        let value = value.unwrap_or_default();
        if value.chars().count() <= TEAM_FIELD_MAX_LEN {
            return value;
        }
        value.chars().take(TEAM_FIELD_MAX_LEN).collect()
    }

    fn persist_match(
        db: &Connection,
        single_match: &Match,
        timestamp: i64,
    ) -> rusqlite::Result<()> {
        let mut stmt = db.prepare("SELECT * FROM match WHERE id = ?1")?;
        let match_already_exists = stmt.exists(rusqlite::params![single_match.id])?;

        let home_team = to_string(&single_match.homeTeam)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let away_team = to_string(&single_match.awayTeam)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let score = to_string(&single_match.score)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

        if match_already_exists {
            db.execute(
                "UPDATE match set homeTeam = ?1, awayTeam = ?2, status = ?3, utcDate = ?4, score = ?5, \
                homeScore = ?6, awayScore = ?7 WHERE id = ?8",
                (
                    home_team,
                    away_team,
                    &single_match.status,
                    timestamp,
                    score,
                    &single_match.homeScore,
                    &single_match.awayScore,
                    &single_match.id,
                ),
            )?;
        } else {
            db.execute(
                "INSERT INTO match (id, homeTeam, awayTeam, status, utcDate, score, homeScore, awayScore) \
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                (
                    &single_match.id,
                    home_team,
                    away_team,
                    &single_match.status,
                    timestamp,
                    score,
                    &single_match.homeScore,
                    &single_match.awayScore,
                ),
            )?;
        }

        Ok(())
    }

    async fn get_connection() -> Option<Connection> {
        let db_path = match env::var("DB_PATH") {
            Ok(v) => v,
            Err(_) => {
                eprintln!("macht-api: env variable DB_PATH not set");
                return None;
            }
        };

        match Connection::open(db_path) {
            Ok(conn) => {
                // Shared DB with frontend + betting-api: wait for the lock
                // instead of failing with SQLITE_BUSY, and use WAL so the
                // importer's writes don't block readers.
                if let Err(e) = conn.busy_timeout(std::time::Duration::from_millis(5000)) {
                    eprintln!("macht-api: failed to set busy_timeout: {e}");
                    return None;
                }
                if let Err(e) = conn.query_row("PRAGMA journal_mode = WAL", [], |row| {
                    row.get::<_, String>(0)
                }) {
                    eprintln!("macht-api: failed to enable WAL: {e}");
                    return None;
                }
                Some(conn)
            }
            Err(e) => {
                eprintln!("macht-api: failed to open database: {e}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn save_matches_to_sqlite_inserts_new_match() {
        dotenv().ok();
        let db_path = env::var("DB_PATH").unwrap();
        let conn = Connection::open(&db_path).unwrap();

        let mut matches = vec![Match {
            id: 11111,
            utcDate: "2022-01-01T00:00:00Z".to_string(),
            homeTeam: Team {
                id: Some(1),
                name: Some("Home Team".to_string()),
                shortName: None,
                tla: Some("HOM".to_string()),
                flag: None,
            },
            awayTeam: Team {
                id: Some(2),
                name: Some("Away Team".to_string()),
                shortName: None,
                tla: Some("AWY".to_string()),
                flag: None,
            },
            score: Score {
                winner: None,
                duration: "".to_string(),
                fullTime: ScoreDetail {
                    home: None,
                    away: None,
                },
                halfTime: ScoreDetail {
                    home: None,
                    away: None,
                },
                regularTime: None,
            },
            status: "SCHEDULED".to_string(),
            homeScore: Some(0),
            awayScore: Some(0),
        }];

        MatchClient::save_matches_to_sqlite(&mut matches).await;

        let mut stmt = conn
            .prepare("SELECT * FROM match WHERE id = 11111")
            .unwrap();
        let match_exists = stmt.exists(()).unwrap();

        conn.execute("DELETE FROM match WHERE id = 11111", ())
            .unwrap();

        assert!(match_exists);
    }

    #[tokio::test]
    async fn save_matches_to_sqlite_updates_existing_match() {
        dotenv().ok();
        let db_path = env::var("DB_PATH").unwrap();
        let conn = Connection::open(&db_path).unwrap();

        let mut matches = vec![Match {
            id: 11111,
            utcDate: "2022-01-01T00:00:00Z".to_string(),
            homeTeam: Team {
                id: Some(1),
                name: Some("Home Team".to_string()),
                shortName: None,
                tla: Some("HOM".to_string()),
                flag: None,
            },
            awayTeam: Team {
                id: Some(2),
                name: Some("Away Team".to_string()),
                shortName: None,
                tla: Some("AWY".to_string()),
                flag: None,
            },
            score: Score {
                winner: None,
                duration: "".to_string(),
                fullTime: ScoreDetail {
                    home: None,
                    away: None,
                },
                halfTime: ScoreDetail {
                    home: None,
                    away: None,
                },
                regularTime: None,
            },
            status: "SCHEDULED".to_string(),
            homeScore: Some(0),
            awayScore: Some(0),
        }];

        MatchClient::save_matches_to_sqlite(&mut matches).await;

        matches[0].status = "FINISHED".to_string();
        MatchClient::save_matches_to_sqlite(&mut matches).await;

        let mut stmt = conn
            .prepare("SELECT status FROM match WHERE id = 11111")
            .unwrap();
        let status: String = stmt.query_row((), |row| row.get(0)).unwrap();

        conn.execute("DELETE FROM match WHERE id = 11111", ())
            .unwrap();

        assert_eq!(status, "FINISHED");
    }

    fn sample_match(id: isize, utc_date: &str) -> Match {
        Match {
            id,
            utcDate: utc_date.to_string(),
            homeTeam: Team {
                id: Some(1),
                name: Some("Home Team".to_string()),
                shortName: None,
                tla: Some("HOM".to_string()),
                flag: None,
            },
            awayTeam: Team {
                id: Some(2),
                name: Some("Away Team".to_string()),
                shortName: None,
                tla: Some("AWY".to_string()),
                flag: None,
            },
            score: Score {
                winner: None,
                duration: "".to_string(),
                fullTime: ScoreDetail {
                    home: None,
                    away: None,
                },
                halfTime: ScoreDetail {
                    home: None,
                    away: None,
                },
                regularTime: None,
            },
            status: "SCHEDULED".to_string(),
            homeScore: Some(0),
            awayScore: Some(0),
        }
    }

    #[tokio::test]
    async fn bad_date_skips_only_that_match() {
        dotenv().ok();
        let db_path = env::var("DB_PATH").unwrap();
        let conn = Connection::open(&db_path).unwrap();

        let mut matches = vec![
            sample_match(11112, "not-a-date"),
            sample_match(11113, "2022-01-01T00:00:00Z"),
        ];

        MatchClient::save_matches_to_sqlite(&mut matches).await;

        let mut bad_stmt = conn
            .prepare("SELECT * FROM match WHERE id = 11112")
            .unwrap();
        let bad_exists = bad_stmt.exists(()).unwrap();
        let mut good_stmt = conn
            .prepare("SELECT * FROM match WHERE id = 11113")
            .unwrap();
        let good_exists = good_stmt.exists(()).unwrap();

        conn.execute("DELETE FROM match WHERE id IN (11112, 11113)", ())
            .unwrap();

        assert!(!bad_exists);
        assert!(good_exists);
    }

    #[tokio::test]
    async fn partial_team_fields_normalize_to_non_null_strings() {
        dotenv().ok();
        let db_path = env::var("DB_PATH").unwrap();
        let conn = Connection::open(&db_path).unwrap();

        // Determined via name, but tla missing -> imported, tla normalized to "".
        let mut m = sample_match(11114, "2022-01-01T00:00:00Z");
        m.homeTeam.tla = None;
        let mut matches = vec![m];

        MatchClient::save_matches_to_sqlite(&mut matches).await;

        let mut stmt = conn
            .prepare("SELECT homeTeam FROM match WHERE id = 11114")
            .unwrap();
        let home_team: String = stmt.query_row((), |row| row.get(0)).unwrap();

        conn.execute("DELETE FROM match WHERE id = 11114", ())
            .unwrap();

        let home: Team = serde_json::from_str(&home_team).unwrap();
        assert_eq!(home.name.as_deref(), Some("Home Team"));
        assert_eq!(home.tla.as_deref(), Some(""));
    }

    #[tokio::test]
    async fn undetermined_team_match_is_skipped() {
        dotenv().ok();
        let db_path = env::var("DB_PATH").unwrap();
        let conn = Connection::open(&db_path).unwrap();

        // An undetermined pairing (both teams null) plus a determined one.
        let mut placeholder = sample_match(11116, "2022-01-01T00:00:00Z");
        placeholder.homeTeam = Team {
            id: None,
            name: None,
            shortName: None,
            tla: None,
            flag: None,
        };
        placeholder.awayTeam = Team {
            id: None,
            name: None,
            shortName: None,
            tla: None,
            flag: None,
        };
        let mut matches = vec![placeholder, sample_match(11117, "2022-01-01T00:00:00Z")];

        MatchClient::save_matches_to_sqlite(&mut matches).await;

        let undetermined_exists = conn
            .prepare("SELECT * FROM match WHERE id = 11116")
            .unwrap()
            .exists(())
            .unwrap();
        let determined_exists = conn
            .prepare("SELECT * FROM match WHERE id = 11117")
            .unwrap()
            .exists(())
            .unwrap();

        conn.execute("DELETE FROM match WHERE id IN (11116, 11117)", ())
            .unwrap();

        // Undetermined: not imported. Determined neighbour: imported.
        assert!(!undetermined_exists);
        assert!(determined_exists);
    }

    #[test]
    fn sanitize_field_caps_length_and_defaults_null() {
        assert_eq!(MatchClient::sanitize_field(None), "");
        assert_eq!(MatchClient::sanitize_field(Some("GER".to_string())), "GER");

        let long = "a".repeat(TEAM_FIELD_MAX_LEN + 50);
        assert_eq!(
            MatchClient::sanitize_field(Some(long)).len(),
            TEAM_FIELD_MAX_LEN
        );
    }

    fn team(name: Option<&str>, tla: Option<&str>) -> Team {
        Team {
            id: None,
            name: name.map(str::to_string),
            shortName: None,
            tla: tla.map(str::to_string),
            flag: None,
        }
    }

    #[test]
    fn team_determined_true_when_name_or_tla_present() {
        assert!(MatchClient::team_determined(&team(Some("Germany"), None)));
        assert!(MatchClient::team_determined(&team(None, Some("GER"))));
        assert!(MatchClient::team_determined(&team(
            Some("Germany"),
            Some("GER")
        )));
    }

    #[test]
    fn team_determined_false_when_null_or_blank() {
        assert!(!MatchClient::team_determined(&team(None, None)));
        // Whitespace-only / empty strings are treated as undetermined.
        assert!(!MatchClient::team_determined(&team(Some("   "), Some(""))));
    }

    #[test]
    fn normalize_team_fills_nulls_with_empty_strings() {
        let mut t = team(None, Some("GER"));
        MatchClient::normalize_team(&mut t);
        assert_eq!(t.name.as_deref(), Some(""));
        assert_eq!(t.tla.as_deref(), Some("GER"));
    }
}
