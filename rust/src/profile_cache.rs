use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS profiles (
        pubkey TEXT PRIMARY KEY,
        metadata JSONB,
        name TEXT,
        picture_remote_url TEXT,
        picture_cached_url TEXT,
        picture_cached_at INTEGER NOT NULL DEFAULT 0,
        lightning_address TEXT,
        lnurl TEXT,
        event_created_at INTEGER NOT NULL DEFAULT 0
    );
";

#[derive(Clone, Debug)]
pub(crate) struct ProfileCacheEntry {
    pub(crate) pubkey: String,
    pub(crate) metadata_json: String,
    pub(crate) name: String,
    pub(crate) picture_remote_url: String,
    pub(crate) picture_cached_url: String,
    pub(crate) picture_cached_at: u64,
    pub(crate) lightning_address: String,
    pub(crate) lnurl: String,
    pub(crate) event_created_at: u64,
}

pub(crate) fn open_profile_cache(data_dir: &Path) -> rusqlite::Result<Connection> {
    let path = data_dir.join("profiles.sqlite3");
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

pub(crate) fn load_profile(
    conn: &Connection,
    pubkey: &str,
) -> rusqlite::Result<Option<ProfileCacheEntry>> {
    conn.query_row(
        "SELECT pubkey,
                COALESCE(json(metadata), '{}'),
                COALESCE(name, ''),
                COALESCE(picture_remote_url, ''),
                COALESCE(picture_cached_url, ''),
                picture_cached_at,
                COALESCE(lightning_address, ''),
                COALESCE(lnurl, ''),
                event_created_at
         FROM profiles
         WHERE pubkey = ?1",
        [pubkey],
        row_to_entry,
    )
    .optional()
}

pub(crate) fn save_profile(conn: &Connection, entry: &ProfileCacheEntry) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO profiles (
            pubkey,
            metadata,
            name,
            picture_remote_url,
            picture_cached_url,
            picture_cached_at,
            lightning_address,
            lnurl,
            event_created_at
         )
         VALUES (?1, jsonb(?2), ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(pubkey) DO UPDATE SET
            metadata = excluded.metadata,
            name = excluded.name,
            picture_remote_url = excluded.picture_remote_url,
            picture_cached_url = CASE
                WHEN profiles.picture_remote_url = excluded.picture_remote_url
                THEN COALESCE(NULLIF(excluded.picture_cached_url, ''), profiles.picture_cached_url)
                ELSE excluded.picture_cached_url
            END,
            picture_cached_at = CASE
                WHEN profiles.picture_remote_url = excluded.picture_remote_url
                THEN MAX(profiles.picture_cached_at, excluded.picture_cached_at)
                ELSE excluded.picture_cached_at
            END,
            lightning_address = excluded.lightning_address,
            lnurl = excluded.lnurl,
            event_created_at = excluded.event_created_at
         WHERE excluded.event_created_at >= profiles.event_created_at",
        params![
            entry.pubkey,
            entry.metadata_json,
            entry.name,
            entry.picture_remote_url,
            entry.picture_cached_url,
            entry.picture_cached_at as i64,
            entry.lightning_address,
            entry.lnurl,
            entry.event_created_at as i64,
        ],
    )?;
    Ok(())
}

pub(crate) fn update_cached_picture(
    conn: &Connection,
    pubkey: &str,
    remote_url: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE profiles
         SET picture_cached_url = ?2,
             picture_cached_at = ?3
         WHERE pubkey = ?1
           AND picture_remote_url = ?2",
        params![pubkey, remote_url, now_unix() as i64],
    )?;
    Ok(())
}

pub(crate) fn clear_profile_cache(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM profiles", [])?;
    Ok(())
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProfileCacheEntry> {
    Ok(ProfileCacheEntry {
        pubkey: row.get(0)?,
        metadata_json: row.get(1)?,
        name: row.get(2)?,
        picture_remote_url: row.get(3)?,
        picture_cached_url: row.get(4)?,
        picture_cached_at: row.get::<_, i64>(5)?.max(0) as u64,
        lightning_address: row.get(6)?,
        lnurl: row.get(7)?,
        event_created_at: row.get::<_, i64>(8)?.max(0) as u64,
    })
}

pub(crate) fn ensure_profile_picture_dir(data_dir: &Path) {
    let dir = profile_picture_dir(data_dir);
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|ext| ext.to_str()) == Some("tmp") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

pub(crate) fn profile_picture_path(data_dir: &Path, pubkey: &str) -> PathBuf {
    profile_picture_dir(data_dir).join(pubkey)
}

pub(crate) fn clear_profile_picture_dir(data_dir: &Path) -> std::io::Result<()> {
    let dir = profile_picture_dir(data_dir);
    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
        return Ok(());
    }
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(())
}

pub(crate) fn profile_picture_file_url(data_dir: &Path, pubkey: &str) -> Option<String> {
    let path = profile_picture_path(data_dir, pubkey);
    let meta = path.metadata().ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    Some(format!("file://{}?v={}", path.display(), mtime))
}

pub(crate) async fn download_profile_picture(
    client: reqwest::Client,
    data_dir: PathBuf,
    pubkey: String,
    remote_url: String,
) -> anyhow::Result<(String, String)> {
    let response = client
        .get(&remote_url)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await?
        .error_for_status()?;
    let bytes = response.bytes().await?;
    if bytes.len() > 5_000_000 {
        anyhow::bail!("profile image too large");
    }
    let dest = profile_picture_path(&data_dir, &pubkey);
    let tmp = dest.with_extension("tmp");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &dest)?;
    Ok((pubkey, remote_url))
}

fn profile_picture_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("profile_pictures")
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}
