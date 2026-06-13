use std::path::PathBuf;

use crate::WalletNetwork;

pub(super) fn load_wallet_metadata_value(
    data_dir: &PathBuf,
    network: WalletNetwork,
    key: &str,
) -> Option<String> {
    let db_path = data_dir.join(network.db_file_name());
    let conn = rusqlite::Connection::open(db_path).ok()?;
    ensure_wallet_metadata_table(&conn).ok()?;
    conn.query_row(
        "SELECT value FROM rebel_wallet_metadata WHERE key = ?1",
        [key],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .filter(|value| !value.trim().is_empty())
}

pub(super) fn save_wallet_metadata_value(
    data_dir: &PathBuf,
    network: WalletNetwork,
    key: &str,
    value: &str,
) -> rusqlite::Result<()> {
    std::fs::create_dir_all(data_dir).ok();
    let db_path = data_dir.join(network.db_file_name());
    let conn = rusqlite::Connection::open(db_path)?;
    ensure_wallet_metadata_table(&conn)?;
    conn.execute(
        "INSERT INTO rebel_wallet_metadata (key, value)
         VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        (key, value),
    )?;
    Ok(())
}

fn ensure_wallet_metadata_table(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS rebel_wallet_metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        )",
        [],
    )?;
    Ok(())
}
