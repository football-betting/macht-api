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
const BUSY_TIMEOUT_MS: u64 = 5000;

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
            Ok(conn) => match Self::configure_connection(&conn) {
                Ok(()) => Some(conn),
                Err(e) => {
                    eprintln!("macht-api: failed to configure connection: {e}");
                    None
                }
            },
            Err(e) => {
                eprintln!("macht-api: failed to open database: {e}");
                None
            }
        }
    }

    // Shared DB with frontend + betting-api. busy_timeout: wait for the lock
    // instead of failing with SQLITE_BUSY. WAL: the importer's writes don't
    // block readers. synchronous=NORMAL: safe under WAL and shortens the fsync,
    // so the write lock is held for less time — less contention between the two
    // writers (frontend tip writes + this importer).
    fn configure_connection(conn: &Connection) -> rusqlite::Result<()> {
        conn.busy_timeout(std::time::Duration::from_millis(BUSY_TIMEOUT_MS))?;
        conn.query_row("PRAGMA journal_mode = WAL", [], |row| {
            row.get::<_, String>(0)
        })?;
        conn.execute_batch("PRAGMA synchronous = NORMAL;")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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

    // --- Upstream API contract: deserialization (no DB, no network) ---

    #[test]
    fn deserializes_full_upstream_match() {
        let json = r#"{
            "matches": [{
                "id": 537901,
                "utcDate": "2026-06-11T19:00:00Z",
                "homeTeam": {"id": 759, "name": "Germany", "shortName": "Germany", "tla": "GER", "crest": "https://crests.football-data.org/759.png"},
                "awayTeam": {"id": 760, "name": "Scotland", "shortName": "Scotland", "tla": "SCO", "crest": "https://crests.football-data.org/760.png"},
                "score": {
                    "winner": "HOME_TEAM",
                    "duration": "REGULAR",
                    "fullTime": {"home": 5, "away": 1},
                    "halfTime": {"home": 2, "away": 1},
                    "regularTime": {"home": 5, "away": 1}
                },
                "status": "FINISHED",
                "homeScore": 5,
                "awayScore": 1
            }]
        }"#;

        let result: ApiResult = serde_json::from_str(json).unwrap();
        let matches = result.matches.expect("matches present");
        assert_eq!(matches.len(), 1);
        let m = &matches[0];
        assert_eq!(m.id, 537901);
        assert_eq!(m.status, "FINISHED");
        assert_eq!(m.homeTeam.tla.as_deref(), Some("GER"));
        // serde rename: upstream "crest" maps onto the internal `flag` field.
        assert_eq!(
            m.homeTeam.flag.as_deref(),
            Some("https://crests.football-data.org/759.png")
        );
        assert_eq!(m.score.fullTime.home, Some(5));
        assert_eq!(m.score.regularTime.as_ref().unwrap().home, Some(5));
        assert_eq!(m.score.winner.as_deref(), Some("HOME_TEAM"));
    }

    #[test]
    fn deserializes_response_without_matches_as_none() {
        // football-data.org returns an error/quota object with no "matches"
        // array. main.rs relies on this becoming None -> abort without changes.
        let json = r#"{ "message": "You reached your request limit.", "errorCode": 429 }"#;
        let result: ApiResult = serde_json::from_str(json).unwrap();
        assert!(result.matches.is_none());
    }

    #[test]
    fn deserializes_knockout_placeholder_with_null_teams() {
        // A not-yet-determined knockout fixture: teams and scores all null,
        // regularTime absent. Must deserialize, then read as undetermined.
        let json = r#"{
            "matches": [{
                "id": 600000,
                "utcDate": "2026-07-09T19:00:00Z",
                "homeTeam": {"id": null, "name": null, "shortName": null, "tla": null, "crest": null},
                "awayTeam": {"id": null, "name": null, "shortName": null, "tla": null, "crest": null},
                "score": {
                    "winner": null,
                    "duration": "REGULAR",
                    "fullTime": {"home": null, "away": null},
                    "halfTime": {"home": null, "away": null}
                },
                "status": "TIMED",
                "homeScore": null,
                "awayScore": null
            }]
        }"#;

        let result: ApiResult = serde_json::from_str(json).unwrap();
        let m = &result.matches.unwrap()[0];
        // regularTime omitted upstream -> None (falls back to fullTime later).
        assert!(m.score.regularTime.is_none());
        assert!(!MatchClient::team_determined(&m.homeTeam));
        assert!(!MatchClient::team_determined(&m.awayTeam));
    }

    // --- Cross-service serialization contract (schema lockstep) ---

    #[test]
    fn team_serializes_to_frontend_contract_keys() {
        // The frontend schema and betting-api read homeTeam/awayTeam JSON as
        // {name, tla, crest?}. The internal field is `flag`, serialized as
        // "crest" — that rename must hold on the way out, too.
        let t = Team {
            id: Some(759),
            name: Some("Germany".to_string()),
            shortName: Some("Germany".to_string()),
            tla: Some("GER".to_string()),
            flag: Some("https://crests.football-data.org/759.png".to_string()),
        };

        let value: serde_json::Value = serde_json::from_str(&to_string(&t).unwrap()).unwrap();
        assert_eq!(value["name"], "Germany");
        assert_eq!(value["tla"], "GER");
        assert_eq!(value["crest"], "https://crests.football-data.org/759.png");
        assert!(
            value.get("flag").is_none(),
            "internal field name `flag` must not leak into the shared DB JSON"
        );
    }

    // --- sanitize_field: multibyte safety ---

    #[test]
    fn sanitize_field_truncates_on_char_boundary_not_bytes() {
        // Each 'ü' is 2 bytes: a byte-based cap could split a char and panic.
        let input = "ü".repeat(TEAM_FIELD_MAX_LEN + 10);
        let out = MatchClient::sanitize_field(Some(input));
        assert_eq!(out.chars().count(), TEAM_FIELD_MAX_LEN);
        // Valid UTF-8 with no split char: the end is a char boundary.
        assert!(out.is_char_boundary(out.len()));
    }

    // --- Connection robustness under concurrent access ---

    #[test]
    fn configure_connection_sets_wal_and_normal_synchronous() {
        let conn = Connection::open_in_memory().unwrap();
        MatchClient::configure_connection(&conn).unwrap();
        // in-memory DBs report "memory" journal mode, so assert synchronous
        // (1 = NORMAL) which applies regardless of the backing store.
        let synchronous: i64 = conn
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .unwrap();
        assert_eq!(synchronous, 1, "synchronous should be NORMAL (1)");
    }

    #[test]
    fn concurrent_writers_and_reader_share_one_db_without_lock_errors() {
        use std::thread;

        const WRITERS: i64 = 4;
        const ROWS_PER_WRITER: i64 = 50;

        let path =
            std::env::temp_dir().join(format!("macht_api_concurrency_{}.db", std::process::id()));
        let cleanup = |p: &std::path::Path| {
            let _ = std::fs::remove_file(p);
            let _ = std::fs::remove_file(format!("{}-wal", p.display()));
            let _ = std::fs::remove_file(format!("{}-shm", p.display()));
        };
        cleanup(&path);

        {
            let conn = Connection::open(&path).unwrap();
            MatchClient::configure_connection(&conn).unwrap();
            conn.execute(
                "CREATE TABLE IF NOT EXISTS match (id INTEGER PRIMARY KEY, status TEXT)",
                [],
            )
            .unwrap();
        }

        let mut handles = Vec::new();

        for w in 0..WRITERS {
            let p = path.clone();
            handles.push(thread::spawn(move || {
                let conn = Connection::open(&p).unwrap();
                MatchClient::configure_connection(&conn).unwrap();
                for i in 0..ROWS_PER_WRITER {
                    // .unwrap() here is the assertion: a write that could not
                    // acquire the lock within busy_timeout would panic and fail
                    // the test — i.e. the "connection is gone" failure mode.
                    conn.execute(
                        "INSERT OR REPLACE INTO match (id, status) VALUES (?1, ?2)",
                        rusqlite::params![w * 1000 + i, "SCHEDULED"],
                    )
                    .unwrap();
                }
            }));
        }

        // A reader running concurrently with the writers must never be blocked
        // out under WAL.
        {
            let p = path.clone();
            handles.push(thread::spawn(move || {
                let conn = Connection::open(&p).unwrap();
                MatchClient::configure_connection(&conn).unwrap();
                for _ in 0..ROWS_PER_WRITER {
                    let _count: i64 = conn
                        .query_row("SELECT count(*) FROM match", [], |row| row.get(0))
                        .unwrap();
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let conn = Connection::open(&path).unwrap();
        MatchClient::configure_connection(&conn).unwrap();
        let total: i64 = conn
            .query_row("SELECT count(*) FROM match", [], |row| row.get(0))
            .unwrap();
        cleanup(&path);

        assert_eq!(total, WRITERS * ROWS_PER_WRITER);
    }

    // --- get_matches: upstream HTTP paths (mock server) ---
    //
    // These mutate process-global env (API_URI / X_AUTH_TOKEN / DB_PATH). The
    // suite already runs serially in CI (RUST_TEST_THREADS=1) because the
    // DB-backed tests share one DB_PATH, so the env mutation is safe here too.
    // dotenvy::dotenv() does not override already-set vars, so the values set
    // below win over any .env file.

    const SINGLE_MATCH_BODY: &str = r#"{
        "matches": [{
            "id": 700001,
            "utcDate": "2026-06-11T19:00:00Z",
            "homeTeam": {"id": 1, "name": "Germany", "shortName": "GER", "tla": "GER", "crest": null},
            "awayTeam": {"id": 2, "name": "Scotland", "shortName": "SCO", "tla": "SCO", "crest": null},
            "score": {"winner": null, "duration": "REGULAR", "fullTime": {"home": null, "away": null}, "halfTime": {"home": null, "away": null}},
            "status": "TIMED",
            "homeScore": null,
            "awayScore": null
        }]
    }"#;

    #[tokio::test]
    async fn get_matches_parses_successful_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(SINGLE_MATCH_BODY, "application/json"),
            )
            .mount(&server)
            .await;
        std::env::set_var("API_URI", server.uri());
        std::env::set_var("X_AUTH_TOKEN", "test-token");

        let result = MatchClient::get_matches(None).await;

        let matches = result.expect("Some result").matches.expect("matches");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, 700001);
        assert_eq!(matches[0].homeTeam.tla.as_deref(), Some("GER"));
    }

    #[tokio::test]
    async fn get_matches_sends_auth_header_and_date_query() {
        let server = MockServer::start().await;
        // The mock only matches when the auth header and both date params are
        // present, so a 200 here proves get_matches built the request correctly.
        Mock::given(method("GET"))
            .and(header("X-Auth-Token", "secret-xyz"))
            .and(query_param("dateFrom", "2026-06-11"))
            .and(query_param("dateTo", "2026-06-11"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(r#"{"matches":[]}"#, "application/json"),
            )
            .mount(&server)
            .await;
        std::env::set_var("API_URI", server.uri());
        std::env::set_var("X_AUTH_TOKEN", "secret-xyz");

        let result = MatchClient::get_matches(Some("2026-06-11".to_string())).await;

        assert_eq!(
            result.expect("Some result").matches.expect("matches").len(),
            0
        );
    }

    #[tokio::test]
    async fn get_matches_returns_none_on_error_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;
        std::env::set_var("API_URI", server.uri());
        std::env::set_var("X_AUTH_TOKEN", "test-token");

        assert!(MatchClient::get_matches(None).await.is_none());
    }

    #[tokio::test]
    async fn get_matches_returns_none_on_malformed_json() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw("definitely not json", "application/json"),
            )
            .mount(&server)
            .await;
        std::env::set_var("API_URI", server.uri());
        std::env::set_var("X_AUTH_TOKEN", "test-token");

        assert!(MatchClient::get_matches(None).await.is_none());
    }

    // --- get_connection: error branches (save/restore DB_PATH for the suite) ---

    #[tokio::test]
    async fn save_matches_is_noop_when_db_path_unset() {
        let saved = std::env::var("DB_PATH").ok();
        std::env::remove_var("DB_PATH");

        let mut matches = vec![sample_match(199990, "2022-01-01T00:00:00Z")];
        // Graceful no-op: get_connection returns None, save_matches returns.
        MatchClient::save_matches_to_sqlite(&mut matches).await;

        if let Some(v) = saved {
            std::env::set_var("DB_PATH", v);
        }
    }

    #[tokio::test]
    async fn save_matches_is_noop_when_db_path_unopenable() {
        let saved = std::env::var("DB_PATH").ok();
        std::env::set_var("DB_PATH", "/nonexistent-dir-xyz/cannot/open.db");

        let mut matches = vec![sample_match(199991, "2022-01-01T00:00:00Z")];
        // Connection::open fails -> get_connection None -> no panic, no write.
        MatchClient::save_matches_to_sqlite(&mut matches).await;

        match saved {
            Some(v) => std::env::set_var("DB_PATH", v),
            None => std::env::remove_var("DB_PATH"),
        }
    }
}
