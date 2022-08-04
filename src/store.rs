use std::path::Path;

use chrono::{Local, NaiveDateTime, TimeZone, Utc};
use color_eyre::eyre::bail;
use serde::{Deserialize, Serialize};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
    Sqlite, SqlitePool, Transaction,
};
use tracing::{debug, info, warn};

use crate::midi::{RECORDING_PPQ, RECORDING_TEMPO, RECORDING_BPM};

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
pub struct RecordingInfo {
    pub id: RecordingId,
    pub name: String,
    pub created_at: chrono::DateTime<Utc>,
    pub length_seconds: f64,
    pub note_count: u32,
}

#[derive(Debug)]
pub struct RecordingStore {
    pool: SqlitePool,
}

impl RecordingStore {
    pub async fn open(directory: &Path) -> color_eyre::Result<Self> {
        let dbfile = directory.join("autorec.db");

        let conn_opts = SqliteConnectOptions::new()
            .filename(dbfile)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Delete);
        let pool = SqlitePoolOptions::new().connect_with(conn_opts).await?;

        migrate(&pool, directory).await?;

        Ok(Self { pool })
    }

    pub async fn get_recording_infos(&self) -> color_eyre::Result<Vec<RecordingInfo>> {
        let recordings = sqlx::query_as::<_, RecordingInfo>(
            "SELECT id, name, created_at, length_seconds, note_count FROM recordings ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(recordings)
    }

    pub async fn get_recording_info_by_id(
        &self,
        id: RecordingId,
    ) -> color_eyre::Result<RecordingInfo> {
        let recording = sqlx::query_as::<_, RecordingInfo>(
            "SELECT id, name, created_at, length_seconds, note_count FROM recordings WHERE id = ?",
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

    pub async fn rename_recording_by_id(
        &self,
        id: RecordingId,
        new_name: String,
    ) -> color_eyre::Result<()> {
        let recording = sqlx::query("UPDATE recordings SET name = ? WHERE id = ?")
            .bind(new_name)
            .bind(id)
            .execute(&self.pool)
            .await?;
        if recording.rows_affected() == 0 {
            bail!("No recording found with id {}", id.0)
        }
        Ok(())
    }

    pub async fn insert_recording(
        &self,
        midi: midly::Smf<'static>,
    ) -> color_eyre::Result<RecordingInfo> {
        let mut midi_data = vec![];
        midi.write_std(&mut midi_data)
            .expect("writing to vec doesn't fail");
        let compressed_midi = compress_midi(midi_data);

        let (length, note_count) = midi
            .tracks
            .first()
            .map_or((std::time::Duration::default(), 0), compute_midi_stats);

        let rec = sqlx::query_as::<_, RecordingInfo>(
            "INSERT INTO recordings (created_at, length_seconds, note_count, midi)
                VALUES (?, ?, ?, ?)
                RETURNING id, name, created_at, length_seconds, note_count",
        )
        .bind(Utc::now())
        .bind(length.as_secs_f64())
        .bind(u32::try_from(note_count).unwrap_or(u32::MAX))
        .bind(compressed_midi)
        .fetch_one(&self.pool)
        .await?;
        Ok(rec)
    }

    pub async fn get_recording_midi(&self, id: RecordingId) -> color_eyre::Result<Vec<u8>> {
        let (compressed_midi,) =
            sqlx::query_as::<_, (Vec<u8>,)>("SELECT midi FROM recordings WHERE id = ?")
                .bind(id)
                .fetch_one(&self.pool)
                .await?;
        let midi = decompress_midi(compressed_midi);
        Ok(midi)
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

    const LATEST_VERSION: i32 = 2;

    loop {
        if let Some(version) = version {
            if version < LATEST_VERSION {
                // create a backup
                let orig = directory.join("autorec.db");
                let backup = directory.join(format!("autorec.db.v{}", version));
                if backup.exists() {
                    bail!("Backup file '{}' already exists", backup.display());
                }
                std::fs::copy(orig, backup)?;
            }
        }

        let mut transaction = pool.begin().await?;

        // Insert new migrations here
        match version {
            None => migrate_000_init(&mut transaction, directory).await?,
            Some(0) => {
                migrate_001_inline_midi_storage_and_meta(&mut transaction, directory).await?
            }
            Some(1) => migrate_002_fix_length_seconds(&mut transaction).await?,
            Some(LATEST_VERSION) => {
                debug!("No more migrations");
                break;
            }
            Some(_) => {
                bail!("Version too new!")
            }
        };

        let new_version = version.map_or(0, |v| v + 1);
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

    let mut recordings = Vec::new();

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

                debug!("Found {} with timestamp {}", filename, created_at);

                recordings.push((created_at, filename.to_owned()));
            }
        }
    }

    // Sort to keep id's ascending in chronological order
    recordings.sort();

    for (created_at, filename) in recordings {
        sqlx::query("INSERT INTO recordings (filename, created_at) VALUES (?, ?)")
            .bind(filename)
            .bind(created_at)
            .execute(&mut *transaction)
            .await?;
    }

    Ok(())
}

/// Switch to storing the MIDI files inline with the SQLite database. They are quite small and not
/// dealing with files on disk hopefully simplifies things.
///
/// Additionally, we will store the length of the recordings as well.
async fn migrate_001_inline_midi_storage_and_meta(
    transaction: &mut Transaction<'_, Sqlite>,
    directory: &Path,
) -> color_eyre::Result<()> {
    debug!("Querying current recordings");

    let recordings = sqlx::query_as::<_, (RecordingId, String)>(
        "SELECT id, filename FROM recordings ORDER BY id",
    )
    .fetch_all(&mut *transaction)
    .await?;

    debug!("Adding new columns");

    sqlx::query("ALTER TABLE recordings ADD COLUMN midi BLOB NOT NULL DEFAULT x''")
        .execute(&mut *transaction)
        .await?;
    sqlx::query("ALTER TABLE recordings ADD COLUMN length_seconds REAL NOT NULL DEFAULT 0")
        .execute(&mut *transaction)
        .await?;
    sqlx::query("ALTER TABLE recordings ADD COLUMN note_count INTEGER NOT NULL DEFAULT 0")
        .execute(&mut *transaction)
        .await?;

    debug!("Processing recordings");

    for (id, filename) in recordings {
        let path = directory.join(&filename);

        debug!("Loading {}", path.display());

        let midi_data = std::fs::read(path)?;
        let mut midi = midly::Smf::parse(&midi_data)?;

        // Fixup tempo (MIDI files should explicitly set it - I previously relied on the aplaymidi
        // default of 120)
        for track in midi.tracks.iter_mut() {
            let tempo_msg = midly::TrackEvent {
                delta: 0.into(),
                kind: midly::TrackEventKind::Meta(midly::MetaMessage::Tempo(RECORDING_TEMPO.into())),
            };
            track.insert(0, tempo_msg);
        }
        // Shadow old data with corrected version
        let mut midi_data = vec![];
        midi.write_std(&mut midi_data)
            .expect("writing to vec doesn't fail");

        // Compress using default LZ4 with size prefix
        //let compressed_midi = lz4::block::compress(&midi_data, None, true)?;
        let compressed_midi = compress_midi(midi_data);

        if let Some(track) = midi.tracks.first() {
            // Our MIDI files should have just a single track
            let (length, note_count) = compute_midi_stats(track);

            debug!("Updating entry for {id:?}");
            let result = sqlx::query(
                r"
                UPDATE recordings
                    SET
                        midi = ?,
                        length_seconds = ?,
                        note_count = ?
                    WHERE
                        id = ?
            ",
            )
            .bind(&compressed_midi)
            .bind(length.as_secs_f64())
            .bind(u32::try_from(note_count).unwrap_or(u32::MAX))
            .bind(id)
            .execute(&mut *transaction)
            .await?;
            if result.rows_affected() != 1 {
                bail!(
                    "Update to {id:?} did not affect exactly one row, {}",
                    result.rows_affected()
                );
            }
        } else {
            bail!("Recording {id:?} at {filename} does not have a MIDI track",);
        }
    }

    info!("Forgetting filename");

    sqlx::query("ALTER TABLE recordings DROP COLUMN filename")
        .execute(&mut *transaction)
        .await?;

    Ok(())
}

/// Due to a bug, the length of a track in seconds was overestimated.
async fn migrate_002_fix_length_seconds(
    transaction: &mut Transaction<'_, Sqlite>,
) -> color_eyre::Result<()> {
    let res = sqlx::query("UPDATE recordings SET length_seconds = length_seconds * 96 / 120")
        .execute(&mut *transaction)
        .await?;
    info!("Updated {} recordings", res.rows_affected());
    Ok(())
}

fn compute_midi_stats(track: &midly::Track) -> (std::time::Duration, usize) {
    let length_ticks = track.iter().map(|event| event.delta.as_int()).sum::<u32>();
    let length = std::time::Duration::from_micros(
        (length_ticks as u64) * 1000000 * 60 / (RECORDING_BPM as u64 * RECORDING_PPQ as u64),
    );
    let note_count = track
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                midly::TrackEventKind::Midi {
                    channel: _,
                    message: midly::MidiMessage::NoteOn { .. }
                }
            )
        })
        .count();

    (length, note_count)
}

fn compress_midi<T: AsRef<[u8]>>(midi: T) -> Vec<u8> {
    zstd::encode_all(midi.as_ref(), 5).expect("compressing in memory should not fail")
}

fn decompress_midi<T: AsRef<[u8]>>(midi: T) -> Vec<u8> {
    zstd::decode_all(midi.as_ref()).expect("decompressing in memory should not fail")
}
