use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDateTime, TimeZone, Utc};
use color_eyre::eyre::bail;
use serde::{Deserialize, Serialize};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
    Sqlite, SqlitePool, Transaction,
};
use tracing::{debug, info, warn};

#[derive(
    Debug,
    Copy,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    sqlx::Decode,
    sqlx::Encode,
)]
pub struct RecordingId(pub i32);

impl<DB: sqlx::Database> sqlx::Type<DB> for RecordingId
where
    i32: sqlx::Type<DB>,
{
    fn type_info() -> DB::TypeInfo {
        <i32 as sqlx::Type<DB>>::type_info()
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RecordingEntry {
    pub id: RecordingId,
    pub name: String,
    pub filename: String,
    pub created_at: chrono::DateTime<Utc>,
}

#[derive(Debug)]
pub struct RecordingStore {
    directory: PathBuf,
    pool: SqlitePool,
}

impl RecordingStore {
    pub async fn open(directory: &Path) -> color_eyre::Result<Self> {
        let dbfile = directory.join("autorec.db");
        let directory = directory.to_owned();

        let conn_opts = SqliteConnectOptions::new()
            .filename(dbfile)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Delete);
        let pool = SqlitePoolOptions::new().connect_with(conn_opts).await?;

        migrate(&pool, &directory).await?;

        Ok(Self { pool, directory })
    }

    pub async fn get_recordings(&self) -> color_eyre::Result<Vec<RecordingEntry>> {
        let recordings = sqlx::query_as::<_, RecordingEntry>(
            "SELECT id, name, filename, created_at FROM recordings ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(recordings)
    }

    pub async fn get_recording_by_id(&self, id: RecordingId) -> color_eyre::Result<RecordingEntry> {
        let recording = sqlx::query_as::<_, RecordingEntry>(
            "SELECT id, name, filename, created_at FROM recordings WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        Ok(recording)
    }

    pub async fn delete_recording_by_id(&self, id: RecordingId) -> color_eyre::Result<()> {
        let recording = sqlx::query("DELETE FROM recordings WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if recording.rows_affected() == 0 {
            bail!("No recording found with id {}", id.0)
        }
        Ok(())
    }

    pub async fn insert_recording(&self, filename: &Path, midi_data: Vec<u8>) -> color_eyre::Result<RecordingEntry> {
        let path = self.directory.join(filename);
        std::fs::write(path, midi_data)?;

        let filename_utf8 = filename
            .to_str()
            .ok_or(color_eyre::eyre::eyre!("Path must be a valid string"))?;
        let rec = sqlx::query_as::<_, RecordingEntry>(
            "INSERT INTO recordings (filename, created_at) VALUES (?, ?) RETURNING id, name, filename, created_at",
        )
        .bind(filename_utf8)
        .bind(Utc::now())
        .fetch_one(&self.pool)
        .await?;
        Ok(rec)
    }
}

async fn migrate(pool: &SqlitePool, directory: &Path) -> color_eyre::Result<()> {
    info!("Checking for migrations");

    // Make sure we have a table to query our versions from
    sqlx::query(
        r"
    CREATE TABLE IF NOT EXISTS migrations (
        id INTEGER PRIMARY KEY NOT NULL,
        applied_at TEXT NOT NULL
    )",
    )
    .execute(pool)
    .await?;

    // Get version of file
    let mut version =
        sqlx::query_scalar::<_, Option<i32>>("SELECT MAX(id) AS version FROM migrations")
            .fetch_one(pool)
            .await?;

    info!("Database version: {:?}", version);

    loop {
        let new_version = version.map_or(0, |v| v + 1);
        let mut transaction = pool.begin().await?;

        // Insert new migrations here
        match version {
            None => {
                migrate_000_init(&mut transaction, directory).await?;
            }
            Some(_) => {
                debug!("No more migrations");
                break;
            }
        };

        info!("Migrated to version {}", new_version);

        let timestamp = chrono::Utc::now();
        sqlx::query("INSERT INTO migrations VALUES (?, ?)")
            .bind(new_version)
            .bind(timestamp)
            .execute(&mut transaction)
            .await?;
        version = Some(new_version);

        transaction.commit().await?;
    }

    Ok(())
}

/// Initial database migration. Setting up the table to store recordings, and populating it from the
/// recordings that are already there.
async fn migrate_000_init(
    transaction: &mut Transaction<'_, Sqlite>,
    directory: &Path,
) -> color_eyre::Result<()> {
    sqlx::query(
        r"
        CREATE TABLE recordings (
            id INTEGER PRIMARY KEY NOT NULL,
            created_at TEXT NOT NULL,
            filename TEXT NOT NULL,
            name TEXT NOT NULL DEFAULT ''
        )
    ",
    )
    .execute(&mut *transaction)
    .await?;

    for recording in std::fs::read_dir(directory)? {
        if let Some(filename) = recording?.file_name().to_str() {
            if let Some(name) = filename.strip_suffix(".mid") {
                // Parse name as date
                let created_at = NaiveDateTime::parse_from_str(name, "%Y%m%d-%H%M%S")
                    .ok()
                    .and_then(|naive| Local.from_local_datetime(&naive).latest())
                    .map_or_else(
                        || {
                            warn!(
                                "Could not parse name {:?} as timestamp, falling back to current time",
                                name
                            );
                            chrono::Utc::now()
                        },
                        |local| local.into(),
                    );

                debug!(
                    "Found and inserting {} with timestamp {}",
                    filename, created_at
                );

                sqlx::query("INSERT INTO recordings (filename, created_at) VALUES (?, ?)")
                    .bind(filename)
                    .bind(created_at)
                    .execute(&mut *transaction)
                    .await?;
            }
        }
    }

    Ok(())
}
