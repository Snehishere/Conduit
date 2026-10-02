use rusqlite::{Connection, params};
use rusqlite_migration::{M, Migrations};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::commands::ConduitSettings;
use crate::encryption::{KEYRING_SERVICE, SQLITE_KEY_ACCOUNT};
use crate::error::{ConduitError, Result};

/// Hard ceiling on any `LIMIT` passed to SQLite from this module.
///
/// SQLite reads a **negative** `LIMIT` as "no limit at all", so an unclamped
/// limit — including a hostile one, since these arrive from JS and from the LAN
/// protocol — silently turns a bounded query into a full-table dump. Command
/// handlers clamp at the boundary; this is the second layer, for the calls
/// that come from handlers nobody remembered to audit.
const MAX_QUERY_LIMIT: i64 = 1_000;

/// Clamp a caller-supplied row limit into a range `LIMIT ?1` can honour.
fn clamp_limit(limit: i64) -> i64 {
    limit.clamp(1, MAX_QUERY_LIMIT)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredDevice {
    pub id: String,
    pub name: String,
    pub device_type: String,
    pub os: String,
    pub public_key: String,
    pub shared_secret: String,
    pub paired_at: i64,
    pub last_seen: i64,
    pub battery: Option<i32>,
    pub signal: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredNotification {
    pub id: String,
    pub device_id: String,
    pub app: String,
    pub title: String,
    pub body: String,
    pub timestamp: i64,
    pub actions: Option<String>,
    pub dismissed: bool,
}

pub struct NotificationParams<'a> {
    pub id: &'a str,
    pub device_id: &'a str,
    pub app: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub timestamp: i64,
    pub actions: Option<&'a str>,
}

pub struct FileTransferParams<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub size: i64,
    pub mime: &'a str,
    pub from_device: &'a str,
    pub to_device: &'a str,
    pub status: &'a str,
    pub chunks_received: i32,
    pub total_chunks: i32,
    pub saved_path: Option<&'a str>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ClipboardEntry {
    pub id: i64,
    pub content: String,
    pub mime: String,
    pub source_device: String,
    pub timestamp: i64,
    pub pinned: bool,
}

pub struct Storage {
    conn: Mutex<Connection>,
}

/// Embedded migrations are always present at compile time. Runtime discovery
/// directories allow dropping additional `NNN_name.sql` files next to the
/// binary (or in the crate's migrations dirs) to upgrade existing databases
/// without a rebuild.
const EMBEDDED_MIGRATIONS: &[(&str, &str)] = &[(
    "001_initial.sql",
    include_str!("migrations/001_initial.sql"),
)];

const SCHEMA_VERSION_TABLE_SQL: &str = "CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER NOT NULL PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT (datetime('now'))
)";

fn db_error(msg: String) -> ConduitError {
    ConduitError::Database(rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
        Some(msg),
    ))
}

// ─── On-disk database file format ─────────────────────────────────────────────

/// The 16-byte header every unencrypted SQLite database starts with.
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// Smallest page size SQLite — and therefore SQLCipher — can use. Every valid
/// page size is a power of two from 512 upwards, so a real database's length is
/// always a whole multiple of this.
const MIN_PAGE_SIZE: u64 = 512;

/// What the first bytes of the database file say about it.
///
/// This exists because "the database would not open" and "the database is
/// corrupt" are *not* the same event, and conflating them is what destroyed
/// user data: a SQLCipher database opened with the wrong key fails to decrypt
/// exactly like a corrupt file does, so the old code renamed the user's
/// database to `*.bak` and started a fresh, empty one — on every launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DbFileKind {
    /// Zero bytes, or no such file. SQLite treats this as a brand new
    /// database, so there is nothing to lose.
    Empty,
    /// A plaintext SQLite database (`SQLite format 3\0`).
    PlainSqlite,
    /// An encrypted SQLCipher database.
    SqlCipher,
    /// Bytes that match no format Conduit writes.
    Unrecognised,
}

/// Classify the database file at `path`.
///
/// The classification is deliberately biased towards [`DbFileKind::SqlCipher`]
/// / [`DbFileKind::PlainSqlite`] ("intact"). A database misread as corrupt gets
/// quarantined, which is how a user loses their settings; a database misread as
/// intact merely produces a clear error and leaves the file exactly where it
/// is. Only the first mistake costs data, and only the second is annoying.
///
/// Note there is no plaintext marker to grep for in an encrypted file: SQLCipher
/// v4 leaves only the 16-byte salt readable and encrypts the rest of the first
/// page, including what older versions stored as a cleartext KDF parameter
/// block. (Verified against the SQLCipher build `rusqlite`'s
/// `bundled-sqlcipher` feature links.) Two structural signals remain:
///
/// * the file length is a whole number of pages, and
/// * those first 16 bytes are not a run of zeros.
///
/// A truncated or garbage file satisfies neither with probability
/// 1 − 1/512.
pub(crate) fn classify_db_file(path: &Path) -> DbFileKind {
    let Ok(meta) = std::fs::metadata(path) else {
        // No such file — or no permission to stat it, which is just as much a
        // reason not to move anything as it is a reason not to create it.
        return DbFileKind::Empty;
    };
    if meta.len() == 0 {
        return DbFileKind::Empty;
    }

    let Ok(file) = std::fs::File::open(path) else {
        return DbFileKind::Empty;
    };
    let mut header = [0u8; 16];
    let mut reader = file.take(header.len() as u64);
    match reader.read_exact(&mut header) {
        Ok(()) => {}
        // Shorter than a header despite a non-zero length: a partial write.
        Err(_) => return DbFileKind::Unrecognised,
    }

    if &header == SQLITE_MAGIC {
        return DbFileKind::PlainSqlite;
    }
    if meta.len() % MIN_PAGE_SIZE == 0 && header.iter().any(|b| *b != 0) {
        DbFileKind::SqlCipher
    } else {
        DbFileKind::Unrecognised
    }
}

/// How many `conduit.corrupt_<ts>.bak` quarantines to keep.
///
/// They accumulated without bound before: one per launch, forever, each the
/// same size as the database.
const MAX_CORRUPT_BACKUPS: usize = 3;

/// Move a corrupt database aside and cap how many quarantines accumulate.
///
/// Returns the backup path. The rename is *not* a deletion — the data is still
/// on disk for manual recovery — but keeping every copy forever turned a
/// recoverable incident into a full disk.
pub(crate) fn quarantine_corrupt_db(db_path: &Path) -> Result<PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let backup = db_path.with_extension(format!("corrupt_{stamp}.bak"));
    std::fs::rename(db_path, &backup).map_err(|e| {
        ConduitError::Storage(format!(
            "Cannot quarantine the unreadable database {}: {e}. \
             Refusing to continue: the original file has been left in place rather than \
             overwritten.",
            db_path.display()
        ))
    })?;
    log::error!(
        "DATA LOSS RISK: {} was unreadable and has been moved to {}. \
         Settings, paired devices, notifications and clipboard history are NOT in the new \
         database. Do not delete the backup.",
        db_path.display(),
        backup.display()
    );
    // Also to stderr: the log file is the only channel in a GUI build, and the
    // very first thing a user checks is the terminal the app was launched from.
    eprintln!(
        "Conduit: the database at {} could not be read and was moved to {}. \
         Your settings and paired devices are in that file, not in the new one.",
        db_path.display(),
        backup.display()
    );
    prune_corrupt_backups(db_path, MAX_CORRUPT_BACKUPS);
    Ok(backup)
}

/// Delete the oldest quarantines beyond `keep`, newest first.
fn prune_corrupt_backups(db_path: &Path, keep: usize) {
    let Some(dir) = db_path.parent() else { return };
    let Some(file_name) = db_path.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    // `conduit.db` -> `conduit.corrupt_`; matches `conduit.corrupt_1699.bak`.
    let stem = match file_name.split_once('.') {
        Some((stem, _)) => format!("{stem}.corrupt_"),
        None => format!("{file_name}.corrupt_"),
    };

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut backups: Vec<(u64, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            let rest = name.strip_prefix(&stem)?;
            let ts = rest.strip_suffix(".bak")?;
            let ts: u64 = ts.parse().ok()?;
            Some((ts, entry.path()))
        })
        .collect();
    // Newest first.
    backups.sort_by_key(|(ts, _)| std::cmp::Reverse(*ts));

    for (_, path) in backups.into_iter().skip(keep) {
        match std::fs::remove_file(&path) {
            Ok(()) => log::info!("Pruned old corrupt-database backup {}", path.display()),
            Err(e) => log::warn!("Could not prune {}: {e}", path.display()),
        }
    }
}

/// Parse the numeric version from a `NNN_name.sql` filename prefix.
fn parse_migration_version(filename: &str) -> Result<i64> {
    filename
        .split('_')
        .next()
        .unwrap_or("")
        .parse::<i64>()
        .map_err(|_| {
            db_error(format!(
                "Cannot parse version from migration filename: {}",
                filename
            ))
        })
}

/// Collect `(version, filename, sql)` from embedded constants plus runtime
/// discovery directories. Embedded entries win on filename collision. Sorted
/// by `(version, filename)` for deterministic application order.
fn collect_migration_scripts() -> Result<Vec<(i64, String, String)>> {
    let mut scripts: Vec<(i64, String, String)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (filename, sql) in EMBEDDED_MIGRATIONS {
        let version = parse_migration_version(filename)?;
        seen.insert((*filename).to_string());
        scripts.push((version, (*filename).to_string(), (*sql).to_string()));
    }

    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut dirs: Vec<PathBuf> = vec![
        manifest_dir.join("src").join("migrations"),
        manifest_dir.join("migrations"),
    ];
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        dirs.push(dir.join("migrations"));
    }

    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("sql") {
                continue;
            }
            let Some(filename) = path.file_name().and_then(|f| f.to_str()) else {
                continue;
            };
            if seen.contains(filename) {
                continue;
            }
            let Ok(version) = parse_migration_version(filename) else {
                log::warn!(
                    "Skipping migration file with unparsable version: {}",
                    path.display()
                );
                continue;
            };
            let Ok(sql) = std::fs::read_to_string(&path) else {
                log::warn!("Skipping unreadable migration file: {}", path.display());
                continue;
            };
            seen.insert(filename.to_string());
            scripts.push((version, filename.to_string(), sql));
        }
    }

    if scripts.is_empty() {
        return Err(db_error("No migration scripts found".to_string()));
    }

    scripts.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    Ok(scripts)
}

/// Create `schema_version` if missing. Returns `true` when the table was just
/// created (i.e. this is the first time the new migration system sees the DB).
fn ensure_schema_version_table(conn: &Connection) -> Result<bool> {
    let exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_version'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|c| c > 0)?;
    if exists {
        return Ok(false);
    }
    conn.execute_batch(SCHEMA_VERSION_TABLE_SQL)?;
    Ok(true)
}

/// Overwrite the single `schema_version` row (single-row invariant).
fn record_schema_version(conn: &Connection, version: i64) -> Result<()> {
    conn.execute("DELETE FROM schema_version", [])?;
    conn.execute(
        "INSERT INTO schema_version (version) VALUES (?1)",
        params![version],
    )?;
    Ok(())
}

/// Current recorded schema version; `0` when no row exists.
fn current_schema_version(conn: &Connection) -> Result<i64> {
    match conn.query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
        row.get::<_, i64>(0)
    }) {
        Ok(v) => Ok(v),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
        Err(e) => Err(e.into()),
    }
}

/// Apply every script whose version is newer than the recorded version, each
/// inside its own transaction (rollback on failure), then sync
/// `PRAGMA user_version` for external observability. Idempotent.
fn apply_pending_migrations(
    conn: &mut Connection,
    scripts: &[(i64, String, String)],
) -> Result<()> {
    let mut current = current_schema_version(conn)?;
    for (version, filename, sql) in scripts {
        if *version <= current {
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(sql)
            .map_err(|e| db_error(format!("Migration {} failed: {}", filename, e)))?;
        let updated = tx.execute("UPDATE schema_version SET version = ?1", params![version])?;
        if updated == 0 {
            tx.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![version],
            )?;
        }
        tx.commit()?;
        current = *version;
    }
    conn.execute_batch(&format!("PRAGMA user_version = {};", current))?;
    Ok(())
}

/// Initialize or upgrade `conn` to the latest known schema version.
///
/// - Fresh DBs (or legacy DBs without `schema_version`): bootstrap the
///   embedded baseline via `rusqlite_migration`, record it, then apply any
///   newer runtime scripts.
/// - Already-tracked DBs: apply only pending scripts.
///
/// Idempotent and safe to call from `Storage::new` and test helpers.
pub(crate) fn run_migrations(conn: &mut Connection) -> Result<()> {
    let fresh = ensure_schema_version_table(conn)?;
    let scripts = collect_migration_scripts()?;
    if fresh {
        let migrations = Migrations::new(
            EMBEDDED_MIGRATIONS
                .iter()
                .map(|(_, sql)| M::up(sql))
                .collect(),
        );
        migrations
            .to_latest(conn)
            .map_err(|e| db_error(format!("Migration error: {}", e)))?;
        let baseline = EMBEDDED_MIGRATIONS
            .iter()
            .filter_map(|(filename, _)| parse_migration_version(filename).ok())
            .max()
            .unwrap_or(0);
        record_schema_version(conn, baseline)?;
    }
    apply_pending_migrations(conn, &scripts)?;
    Ok(())
}

/// The SQLCipher key must be exactly 64 hex characters (32 bytes).
///
/// This is the *only* reason the `KEY "x'…'"` / `PRAGMA key = "x'…'"`
/// interpolations below are safe: a value that is known to be hex cannot
/// terminate the string literal. Rejecting anything else here is what makes
/// that argument hold.
fn validate_db_key(db_key: &str) -> Result<()> {
    if db_key.len() == 64 && db_key.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(db_error(
            "Database key must be exactly 64 hex characters".to_string(),
        ))
    }
}

/// Open the encrypted database and bring it up to the current schema.
fn open_encrypted(path: &Path, db_key: &str) -> Result<Connection> {
    let mut conn = Connection::open(path)?;
    // Hex literal form, so the key is a byte string rather than a passphrase.
    conn.execute_batch(&format!("PRAGMA key = \"x'{db_key}'\";"))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
    run_migrations(&mut conn)?;
    Ok(conn)
}

/// Delete the `-wal` / `-shm` sidecars of a database we are about to replace.
fn remove_sidecar_files(db_path: &Path) {
    for ext in ["db-wal", "db-shm"] {
        let sidecar = db_path.with_extension(ext);
        if let Err(e) = std::fs::remove_file(&sidecar)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("Could not remove {}: {e}", sidecar.display());
        }
    }
}

/// Copy an unencrypted database into a SQLCipher-encrypted one at `enc_path`.
///
/// `db_key` must already have passed [`validate_db_key`]; the key appears in two
/// statements that cannot take a bound parameter, and the hex validation is what
/// makes those two interpolations inert.
fn export_plaintext_to_sqlcipher(plain_path: &Path, enc_path: &Path, db_key: &str) -> Result<()> {
    validate_db_key(db_key)?;
    let plain_conn = Connection::open(plain_path)?;

    // BUG (injection): the destination path used to be formatted straight into
    // the SQL text with only `\` → `/` substituted, so a `'` anywhere in the
    // path — legal on every platform Conduit runs on — closed the literal and
    // the rest of the path was executed as SQL. `ATTACH` takes an *expression*
    // for the file name, so the path is bound: a quote in the path is then just
    // a quote in a filename, and `?1` is a value that SQLite can never be
    // talked into reading as code.
    //
    // The forward-slash normalisation is kept because SQLite accepts it on
    // every platform, including Windows.
    let enc_path_str = enc_path.to_string_lossy().replace('\\', "/");
    plain_conn.execute("ATTACH DATABASE ?1 AS encrypted", params![enc_path_str])?;

    // BUG (wrong key): SQLCipher 4 removed the `KEY` clause from `ATTACH`; the
    // key is now set on the attached schema with a PRAGMA. The old statement
    // still *parsed* — `KEY "x'…'"` is a legal-looking string token — so it
    // exported a database that could not be reopened with the key Conduit went
    // on to use, turning the encryption upgrade into data loss. `PRAGMA` values
    // cannot be bound, so the key is interpolated; it is inert because
    // `validate_db_key` proved it is exactly 64 hex characters.
    plain_conn.execute_batch(&format!("PRAGMA encrypted.key = \"x'{db_key}'\";"))?;

    {
        let mut stmt = plain_conn.prepare("SELECT sqlcipher_export('encrypted')")?;
        let mut rows = stmt.query([])?;
        // Drain rather than `query_row`: the export reports a page count, but
        // the statement must be stepped to completion either way.
        while rows.next()?.is_some() {}
    }
    // DETACH is what commits the attached database to disk.
    plain_conn.execute_batch("DETACH DATABASE encrypted;")?;
    Ok(())
}

/// Encrypt an existing plaintext database in place, keeping a plaintext copy.
///
/// No-op unless the file really is a plaintext SQLite database.
fn migrate_plaintext_db_to_encrypted(db_path: &Path, db_key: &str) -> Result<()> {
    if !db_path.exists() || classify_db_file(db_path) != DbFileKind::PlainSqlite {
        return Ok(());
    }
    log::info!("Detected an unencrypted database; encrypting it with SQLCipher...");
    let temp_enc_path = db_path.with_extension("encrypted_tmp");
    let _ = std::fs::remove_file(&temp_enc_path);

    // Step 1: export. Nothing on disk has changed yet, so failing here costs
    // nothing — which is exactly why the old code, which renamed the
    // plaintext file to `plaintext_bak` *before* trying the export and then
    // started a fresh, empty database, was destroying the user's data here.
    if let Err(e) = export_plaintext_to_sqlcipher(db_path, &temp_enc_path, db_key) {
        let _ = std::fs::remove_file(&temp_enc_path);
        return Err(ConduitError::Storage(format!(
            "Failed to encrypt the existing database at {}: {e}. \
             The file has been left exactly where it was and Conduit will not start, because \
             starting would open a different, empty database and hide the settings and paired \
             devices stored in it. Move conduit.db aside to start from scratch.",
            db_path.display()
        )));
    }

    // Step 2: swap, checking every step. The plaintext copy is the rollback
    // for a failed swap, so it is only deleted once the encrypted file is in
    // place and the encrypted file is verifiably there.
    let plaintext_backup = db_path.with_extension("plaintext_bak");
    let _ = std::fs::remove_file(&plaintext_backup);
    std::fs::rename(db_path, &plaintext_backup).map_err(|e| {
        ConduitError::Storage(format!(
            "Encrypted the database to {} but could not move the original aside: {e}. \
             The encrypted copy is intact; restore it to {} and retry.",
            temp_enc_path.display(),
            db_path.display()
        ))
    })?;
    if let Err(e) = std::fs::rename(&temp_enc_path, db_path) {
        // Put the user's data back where the app expects it before giving up.
        let rollback = std::fs::rename(&plaintext_backup, db_path);
        return Err(ConduitError::Storage(format!(
            "Could not move the encrypted database into place: {e}. Rollback {}.",
            match &rollback {
                Ok(()) => "succeeded; the original database is back in place".to_string(),
                Err(rb) => format!("FAILED ({rb}); recover from {}", plaintext_backup.display()),
            }
        )));
    }
    remove_sidecar_files(db_path);
    log::info!(
        "Database encrypted. A plaintext copy was kept at {} and can be deleted once \
         the app starts cleanly.",
        plaintext_backup.display()
    );
    Ok(())
}

/// Open the database, quarantining it **only** if it is genuinely corrupt.
///
/// DATA LOSS FIX. The previous recovery path renamed `conduit.db` to
/// `corrupt_<ts>.bak` on *any* open failure and started a brand-new empty
/// database. A SQLCipher database opened with the wrong key fails to decrypt
/// exactly the way a corrupt file does, so a single discarded keyring write
/// (see [`crate::encryption::load_or_create_secret`]) turned every launch into
/// a silent reset of the user's settings, paired devices, notifications and
/// clipboard history.
///
/// The header bytes decide:
/// * [`DbFileKind::SqlCipher`] / [`DbFileKind::PlainSqlite`] — the file is
///   structurally intact, so the cause is the *key*, not the data. The file is
///   left untouched and the failure is surfaced.
/// * [`DbFileKind::Empty`] — nothing to lose.
/// * [`DbFileKind::Unrecognised`] — genuinely unreadable; quarantine it.
fn open_or_recover(db_path: &Path, db_key: &str) -> Result<Connection> {
    let open_err = match open_encrypted(db_path, db_key) {
        Ok(conn) => return Ok(conn),
        Err(e) => e,
    };

    match classify_db_file(db_path) {
        kind @ (DbFileKind::SqlCipher | DbFileKind::PlainSqlite) => {
            Err(ConduitError::Storage(format!(
                "The database at {} is INTACT ({kind:?}) but could not be opened: {open_err}\n\
                 This is a key problem, not a data problem — the encryption key that was used \
                 to write this file is no longer the one Conduit has. Your data has NOT been \
                 moved, renamed or overwritten, and Conduit will not overwrite it either.\n\
                 Recover the previous key and Conduit will start normally:\n  \
                   * {} (used when the OS keyring was unavailable), or\n  \
                   * the `{SQLITE_KEY_ACCOUNT}` entry of the `{KEYRING_SERVICE}` keyring.\n\
                 If neither exists, the database cannot be decrypted by any version of Conduit; \
                 move it aside to start with an empty profile.",
                db_path.display(),
                crate::encryption::KeyFileStore::path_for_account(SQLITE_KEY_ACCOUNT).display(),
            )))
        }
        DbFileKind::Empty => {
            log::error!(
                "The database at {} is empty or zero-length and could not be initialised: \
                 {open_err}. Recreating it (there is no data to lose).",
                db_path.display()
            );
            let _ = std::fs::remove_file(db_path);
            open_encrypted(db_path, db_key)
        }
        DbFileKind::Unrecognised => {
            log::error!(
                "The database at {} is not a SQLite or SQLCipher file and could not be opened: \
                 {open_err}. Quarantining it.",
                db_path.display()
            );
            quarantine_corrupt_db(db_path)?;
            remove_sidecar_files(db_path);
            open_encrypted(db_path, db_key)
        }
    }
}

impl Storage {
    /// Maximum unpinned clipboard entries to retain before oldest are pruned.
    const MAX_CLIPBOARD_ENTRIES: usize = 500;

    pub fn new() -> Result<Self> {
        let db_path = Self::get_db_path();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        // P1.4: SQLCipher key management.
        //
        // DATA LOSS FIX. This used to inline a keyring read, and on a read
        // failure generate a new key and *discard* the result of
        // `set_password`. On any Linux host without a running Secret Service —
        // headless servers, minimal containers, most CI images, and this
        // repo's own `docker-compose.yml` — `keyring` 4.2's zbus-secret-service
        // backend returns `PlatformFailure` for every call, so a new key was
        // minted on every launch, `PRAGMA key` no longer matched, `open_and_init`
        // failed, and the recovery path below renamed the user's database to a
        // fresh `.bak` on every single launch.
        //
        // `load_or_create_secret` never returns a secret that has not been read
        // back from a durable store: the OS keyring when it is writable, a
        // `0600` key file under the app data directory otherwise. A database
        // you cannot decrypt is not a database you should overwrite.
        let db_key = crate::encryption::load_or_create_secret(SQLITE_KEY_ACCOUNT)?;
        validate_db_key(&db_key)?;

        migrate_plaintext_db_to_encrypted(&db_path, &db_key)?;
        let conn = open_or_recover(&db_path, &db_key)?;

        Ok(Storage {
            conn: Mutex::new(conn),
        })
    }

    fn get_db_path() -> PathBuf {
        // Same directory as the encryption key file, from one definition, so
        // the database and the key that decrypts it cannot drift apart.
        crate::encryption::app_data_dir().join("conduit.db")
    }

    pub async fn save_device(&self, device: &StoredDevice) -> Result<()> {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            conn.execute(
            "INSERT OR REPLACE INTO devices (id, name, device_type, os, public_key, shared_secret, paired_at, last_seen, battery, signal, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                device.id,
                device.name,
                device.device_type,
                device.os,
                device.public_key,
                device.shared_secret,
                device.paired_at,
                device.last_seen,
                device.battery,
                device.signal,
                device.status,
            ],
        )?;
            Ok(())
        })
    }

    pub async fn get_device(&self, id: &str) -> Result<Option<StoredDevice>> {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
            "SELECT id, name, device_type, os, public_key, shared_secret, paired_at, last_seen, battery, signal, status
             FROM devices WHERE id = ?1",
        )?;

            let mut rows = stmt.query_map(params![id], |row| {
                Ok(StoredDevice {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    device_type: row.get(2)?,
                    os: row.get(3)?,
                    public_key: row.get(4)?,
                    shared_secret: row.get(5)?,
                    paired_at: row.get(6)?,
                    last_seen: row.get(7)?,
                    battery: row.get(8)?,
                    signal: row.get(9)?,
                    status: row.get(10)?,
                })
            })?;

            match rows.next() {
                Some(row) => Ok(Some(row?)),
                None => Ok(None),
            }
        })
    }

    pub async fn get_all_devices(&self) -> Result<Vec<StoredDevice>> {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
            "SELECT id, name, device_type, os, public_key, shared_secret, paired_at, last_seen, battery, signal, status
             FROM devices ORDER BY last_seen DESC",
        )?;

            let devices = stmt
                .query_map([], |row| {
                    Ok(StoredDevice {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        device_type: row.get(2)?,
                        os: row.get(3)?,
                        public_key: row.get(4)?,
                        shared_secret: row.get(5)?,
                        paired_at: row.get(6)?,
                        last_seen: row.get(7)?,
                        battery: row.get(8)?,
                        signal: row.get(9)?,
                        status: row.get(10)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();

            Ok(devices)
        })
    }

    pub async fn delete_device(&self, id: &str) -> Result<()> {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            conn.execute("DELETE FROM devices WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    pub async fn update_battery(&self, id: &str, battery: i32) -> Result<()> {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "UPDATE devices SET battery = ?1 WHERE id = ?2",
                params![battery, id],
            )?;
            Ok(())
        })
    }

    pub async fn update_status(&self, id: &str, status: &str) -> Result<()> {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            let last_seen = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;
            conn.execute(
                "UPDATE devices SET status = ?1, last_seen = ?2 WHERE id = ?3",
                params![status, last_seen, id],
            )?;
            Ok(())
        })
    }

    pub async fn save_notification(&self, p: &NotificationParams<'_>) -> Result<()> {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            conn.execute(
            "INSERT OR REPLACE INTO notifications (id, device_id, app, title, body, timestamp, actions)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![p.id, p.device_id, p.app, p.title, p.body, p.timestamp, p.actions],
        )?;
            Ok(())
        })
    }

    pub async fn get_notifications(&self, limit: i64) -> Result<Vec<StoredNotification>> {
        let limit = clamp_limit(limit);
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, device_id, app, title, body, timestamp, actions, dismissed
             FROM notifications ORDER BY timestamp DESC LIMIT ?1",
            )?;

            let notifications = stmt
                .query_map(params![limit], |row| {
                    Ok(StoredNotification {
                        id: row.get(0)?,
                        device_id: row.get(1)?,
                        app: row.get(2)?,
                        title: row.get(3)?,
                        body: row.get(4)?,
                        timestamp: row.get(5)?,
                        actions: row.get(6)?,
                        dismissed: row.get::<_, i64>(7)? != 0,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();

            Ok(notifications)
        })
    }

    pub async fn dismiss_notification(&self, id: &str) -> Result<()> {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "UPDATE notifications SET dismissed = 1 WHERE id = ?1",
                params![id],
            )?;
            Ok(())
        })
    }

    pub async fn save_clipboard(
        &self,
        content: &str,
        mime: &str,
        source_device: &str,
        timestamp: i64,
    ) -> Result<()> {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO clipboard_history (content, mime, source_device, timestamp)
             VALUES (?1, ?2, ?3, ?4)",
                params![content, mime, source_device, timestamp],
            )?;
            // Auto-prune oldest unpinned entries beyond the cap.
            conn.execute(
                "DELETE FROM clipboard_history
             WHERE pinned = 0
               AND id NOT IN (
                   SELECT id FROM clipboard_history
                   WHERE pinned = 0
                   ORDER BY timestamp DESC
                   LIMIT ?1
               )",
                params![Self::MAX_CLIPBOARD_ENTRIES as i64],
            )?;
            Ok(())
        })
    }

    pub async fn get_clipboard_history(&self, limit: i64) -> Result<Vec<ClipboardEntry>> {
        let limit = clamp_limit(limit);
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, content, mime, source_device, timestamp, pinned
             FROM clipboard_history ORDER BY pinned DESC, timestamp DESC LIMIT ?1",
            )?;

            let entries = stmt
                .query_map(params![limit], |row| {
                    Ok(ClipboardEntry {
                        id: row.get(0)?,
                        content: row.get(1)?,
                        mime: row.get(2)?,
                        source_device: row.get(3)?,
                        timestamp: row.get(4)?,
                        pinned: row.get::<_, i64>(5)? != 0,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();

            Ok(entries)
        })
    }

    pub async fn toggle_clipboard_pin(&self, id: i64) -> Result<()> {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            conn.execute(
            "UPDATE clipboard_history SET pinned = CASE WHEN pinned = 1 THEN 0 ELSE 1 END WHERE id = ?1",
            params![id],
        )?;
            Ok(())
        })
    }

    pub async fn delete_clipboard_entry(&self, id: i64) -> Result<()> {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            conn.execute("DELETE FROM clipboard_history WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    pub async fn clear_clipboard_history(&self) -> Result<()> {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            conn.execute("DELETE FROM clipboard_history WHERE pinned = 0", params![])?;
            Ok(())
        })
    }

    /// Read a single setting row, telling "never set" apart from "the read failed".
    ///
    /// BUG (silent data loss). The old accessor was a three-link `.ok()` chain
    /// (`prepare().ok().and_then(|stmt| …query_map(…).ok())`), so a transient
    /// SQL failure — a locked database, an I/O fault, a corrupt page — was
    /// indistinguishable from "this key has no row" and the caller was handed
    /// the hard-coded default. Because `get_settings` backs most of the settings
    /// page, the user's real value was then overwritten by the next save.
    ///
    /// `Ok(None)` now means *only* "there is no such key".
    fn read_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
        let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get::<_, String>(0)?)),
            None => Ok(None),
        }
    }

    /// [`read_setting`] with a default for the genuine "never set" case.
    ///
    /// A real query error is propagated, so a settings page backed by a
    /// temporarily unreadable database shows an error instead of a form full
    /// of defaults that the user will then save over their own data.
    fn setting_or_default(conn: &Connection, key: &str, default: &str) -> Result<String> {
        Ok(Self::read_setting(conn, key)?.unwrap_or_else(|| default.to_string()))
    }

    pub async fn get_settings(&self) -> Result<ConduitSettings> {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            // The empty-database default is the *same* list serde and
            // `ConduitSettings::default()` use (`default_notification_apps`), so
            // there is exactly one definition of the seeded app allowlist.
            let default_apps = serde_json::to_string(&crate::commands::default_notification_apps())
                .unwrap_or_default();

            // NOTE: the *parse* fallbacks below (`.parse().unwrap_or(5)`,
            // `serde_json::from_str(…).unwrap_or_default()`) are about a stored
            // value that is present but unusable, and are deliberately kept —
            // `save_settings_field_edge_non_numeric_max_devices_silently_falls_
            // back_to_default` pins that contract. Only *query* failures, which
            // used to be swallowed here, are now surfaced.
            Ok(ConduitSettings {
                device_name: Self::setting_or_default(
                    &conn,
                    "device_name",
                    &gethostname::gethostname().to_string_lossy(),
                )?,
                max_devices: Self::setting_or_default(&conn, "max_devices", "5")?
                    .parse()
                    .unwrap_or(5),
                sync_notifications: Self::setting_or_default(&conn, "sync_notifications", "true")?
                    == "true",
                sync_clipboard: Self::setting_or_default(&conn, "sync_clipboard", "true")?
                    == "true",
                sync_files: Self::setting_or_default(&conn, "sync_files", "true")? == "true",
                notification_apps: serde_json::from_str(&Self::setting_or_default(
                    &conn,
                    "notification_apps",
                    &default_apps,
                )?)
                .unwrap_or_default(),
                theme: Self::setting_or_default(&conn, "theme", "dark")?,
                accent_color: Self::setting_or_default(&conn, "accent_color", "#00f0ff")?,
                last_version: Self::setting_or_default(&conn, "last_version", "")?,
                minimize_to_tray: Self::setting_or_default(&conn, "minimize_to_tray", "true")?
                    == "true",
                default_download_folder: Self::setting_or_default(
                    &conn,
                    "default_download_folder",
                    "",
                )?,
                auto_accept_files: Self::setting_or_default(&conn, "auto_accept_files", "true")?
                    == "true",
                notifications_enabled: Self::setting_or_default(
                    &conn,
                    "notifications_enabled",
                    "true",
                )? == "true",
                relay_url: Self::setting_or_default(
                    &conn,
                    "relay_url",
                    crate::commands::DEFAULT_RELAY_URL,
                )?,
                // The default is taken from `ConduitSettings::default()` rather
                // than spelled out here, for the reason the app cares about:
                // `main.rs` reads this struct and, when `relay_enabled` is false,
                // returns *before* both `relay_host.start()` and
                // `spawn_relay_client`. This literal used to be "false", which no
                // migration seeds and nothing writes before that read — so on a
                // fresh install the relay never started and the desktop never
                // joined its own relay, while `ConduitSettings::default()` and
                // `settingsTypes.ts`'s `DEFAULT_SETTINGS` both said "on".
                relay_enabled: Self::setting_or_default(
                    &conn,
                    "relay_enabled",
                    &crate::commands::ConduitSettings::default()
                        .relay_enabled
                        .to_string(),
                )? == "true",
                relay_port: Self::setting_or_default(
                    &conn,
                    "relay_port",
                    &crate::relay::DEFAULT_RELAY_PORT.to_string(),
                )?
                .parse()
                .unwrap_or(crate::relay::DEFAULT_RELAY_PORT),
                relay_health_port: Self::setting_or_default(
                    &conn,
                    "relay_health_port",
                    &crate::relay::DEFAULT_RELAY_HEALTH_PORT.to_string(),
                )?
                .parse()
                .unwrap_or(crate::relay::DEFAULT_RELAY_HEALTH_PORT),
                relay_hostname: Self::setting_or_default(&conn, "relay_hostname", "")?,
                relay_cert_pin: Self::setting_or_default(&conn, "relay_cert_pin", "")?,
            })
        })
    }

    pub async fn save_setting(&self, key: &str, value: &str) -> Result<()> {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
                params![key, value],
            )?;
            Ok(())
        })
    }

    /// Fallible read of a single setting: `Ok(None)` only when the key has
    /// never been written.
    ///
    /// Prefer this over [`Storage::get_setting`] wherever a failed read would
    /// otherwise be silently treated as "unset".
    pub async fn try_get_setting(&self, key: &str) -> Result<Option<String>> {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            Self::read_setting(&conn, key)
        })
    }

    /// Infallible-shaped read kept for the existing call sites.
    ///
    /// BUG (silent data loss). This returned `Option<String>` from a bare `.ok()`,
    /// which collapsed `QueryReturnedNoRows` and a real SQL error (locked
    /// database, I/O fault, corrupt page) into the same `None`. Callers that
    /// treat `None` as "use the default" — window restoration, the
    /// `auto_accept_files` gate, the notification/clipboard/file gates — then
    /// silently did the wrong thing, and any subsequent save wrote the default
    /// over the user's real value.
    ///
    /// The signature is unchanged so `main.rs`'s `generate_handler!` list and
    /// the protocol dispatch keep working; the failure is now logged loudly
    /// instead of vanishing. New code should call [`Storage::try_get_setting`]
    /// and handle the error.
    pub async fn get_setting(&self, key: &str) -> Option<String> {
        match self.try_get_setting(key).await {
            Ok(value) => value,
            Err(e) => {
                log::error!(
                    "Failed to read setting `{key}`: {e}. Reported as unset, which may cause \
                     the default to be written over the real value on the next save."
                );
                None
            }
        }
    }

    /// Synchronous save for callers that cannot `.await` (e.g. Tauri window
    /// event callbacks invoked outside the async runtime). Retries `try_lock`
    /// with a short deadline — the async lock is only held for brief SQLite ops.
    pub fn save_setting_sync(&self, key: &str, value: &str) -> Result<()> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        loop {
            if let Ok(conn) = self.conn.try_lock() {
                conn.execute(
                    "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
                    params![key, value],
                )?;
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(ConduitError::Storage(
                    "Timed out waiting for storage lock".into(),
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Test-only constructor so sibling modules can build an in-memory Storage.
    #[cfg(test)]
    pub(crate) fn from_connection(conn: Connection) -> Self {
        Storage {
            conn: Mutex::new(conn),
        }
    }

    /// Persist every field of `ConduitSettings` in one transaction-free batch of
    /// upserts.
    ///
    /// The `(key, value)` list below is deliberately data-driven and is pinned
    /// against `ConduitSettings`' serialised key set by
    /// `test_save_settings_writes_every_settings_key`. The previous hand-written
    /// chain of `conn.execute` calls silently omitted `relay_url`, so a relay URL
    /// chosen in the UI was dropped on every save and reverted to the default on
    /// the next read.
    pub async fn save_settings(&self, settings: &ConduitSettings) -> Result<()> {
        let notification_apps =
            serde_json::to_string(&settings.notification_apps).map_err(|e| {
                ConduitError::Storage(format!(
                    "failed to serialise notification_apps for storage: {e}"
                ))
            })?;

        let rows: Vec<(&str, String)> = vec![
            ("device_name", settings.device_name.clone()),
            ("max_devices", settings.max_devices.to_string()),
            (
                "sync_notifications",
                settings.sync_notifications.to_string(),
            ),
            ("sync_clipboard", settings.sync_clipboard.to_string()),
            ("sync_files", settings.sync_files.to_string()),
            ("notification_apps", notification_apps),
            ("theme", settings.theme.clone()),
            ("accent_color", settings.accent_color.clone()),
            ("last_version", settings.last_version.clone()),
            ("minimize_to_tray", settings.minimize_to_tray.to_string()),
            (
                "default_download_folder",
                settings.default_download_folder.clone(),
            ),
            ("auto_accept_files", settings.auto_accept_files.to_string()),
            (
                "notifications_enabled",
                settings.notifications_enabled.to_string(),
            ),
            ("relay_url", settings.relay_url.clone()),
            ("relay_enabled", settings.relay_enabled.to_string()),
            ("relay_port", settings.relay_port.to_string()),
            ("relay_health_port", settings.relay_health_port.to_string()),
            ("relay_hostname", settings.relay_hostname.clone()),
            ("relay_cert_pin", settings.relay_cert_pin.clone()),
        ];

        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            let upsert = "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)";
            for (key, value) in rows.iter() {
                conn.execute(upsert, params![*key, value])?;
            }
            Ok(())
        })
    }

    pub async fn save_file_transfer(&self, p: &FileTransferParams<'_>) -> Result<()> {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            conn.execute(
            "INSERT OR REPLACE INTO file_transfers (id, name, size, mime, from_device, to_device, status, chunks_received, total_chunks, saved_path, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![p.id, p.name, p.size, p.mime, p.from_device, p.to_device, p.status, p.chunks_received, p.total_chunks, p.saved_path, p.timestamp],
        )?;
            Ok(())
        })
    }

    pub async fn update_file_transfer_progress(
        &self,
        id: &str,
        status: &str,
        chunks_received: i32,
        saved_path: Option<&str>,
    ) -> Result<()> {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            conn.execute(
            "UPDATE file_transfers SET status = ?2, chunks_received = ?3, saved_path = COALESCE(?4, saved_path) WHERE id = ?1",
            params![id, status, chunks_received, saved_path],
        )?;
            Ok(())
        })
    }

    pub async fn save_automation_rule(
        &self,
        rule: &crate::automation::AutomationRule,
    ) -> Result<()> {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            let trigger_json = serde_json::to_string(&rule.trigger).unwrap_or_default();
            let action_json = serde_json::to_string(&rule.action).unwrap_or_default();
            let trigger_tag = crate::automation::trigger_tag(&rule.trigger);
            let action_tag = crate::automation::action_tag(&rule.action);
            conn.execute(
            "INSERT INTO automation_rules (id, name, enabled, trigger_type, trigger_config, action_type, action_config, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                enabled = excluded.enabled,
                trigger_type = excluded.trigger_type,
                trigger_config = excluded.trigger_config,
                action_type = excluded.action_type,
                action_config = excluded.action_config",
            params![
                rule.id,
                rule.name,
                rule.enabled as i32,
                trigger_tag,
                trigger_json,
                action_tag,
                action_json,
                chrono::Utc::now().timestamp(),
            ],
        )?;
            Ok(())
        })
    }

    pub async fn get_all_automation_rules(&self) -> Result<Vec<crate::automation::AutomationRule>> {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, name, enabled, trigger_type, trigger_config, action_type, action_config
             FROM automation_rules ORDER BY created_at DESC",
            )?;

            let rules = stmt
                .query_map([], |row| {
                    let trigger_json: String = row.get(4)?;
                    let action_json: String = row.get(6)?;
                    let trigger: crate::automation::TriggerType = serde_json::from_str(
                        &trigger_json,
                    )
                    .unwrap_or(crate::automation::TriggerType::DeviceConnect {
                        device_id: "*".to_string(),
                    });
                    let action: crate::automation::ActionType = serde_json::from_str(&action_json)
                        .unwrap_or(crate::automation::ActionType::SendNotification {
                            title: String::new(),
                            body: String::new(),
                        });
                    Ok(crate::automation::AutomationRule {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        enabled: row.get::<_, i32>(2)? != 0,
                        trigger,
                        action,
                        trusted_source_only: false,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();

            Ok(rules)
        })
    }

    pub async fn delete_automation_rule(&self, id: &str) -> Result<()> {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            conn.execute("DELETE FROM automation_rules WHERE id = ?1", params![id])?;
            conn.execute(
                "DELETE FROM automation_logs WHERE rule_id = ?1",
                params![id],
            )?;
            Ok(())
        })
    }

    pub async fn log_automation_execution(
        &self,
        rule_id: &str,
        trigger_type: &str,
        timestamp: i64,
        success: bool,
        message: Option<&str>,
    ) -> Result<()> {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO automation_logs (rule_id, trigger_type, timestamp, success, message)
             VALUES (?1, ?2, ?3, ?4, ?5)",
                params![rule_id, trigger_type, timestamp, success as i32, message],
            )?;
            Ok(())
        })
    }

    pub async fn get_automation_logs(
        &self,
        limit: i64,
    ) -> Result<Vec<crate::automation::RuleExecutionLog>> {
        let limit = clamp_limit(limit);
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT rule_id AS id, trigger_type, timestamp, success, message
             FROM automation_logs ORDER BY timestamp DESC LIMIT ?1",
            )?;

            let logs = stmt
                .query_map(params![limit], |row| {
                    Ok(crate::automation::RuleExecutionLog {
                        id: row.get(0)?,
                        trigger_type: row.get(1)?,
                        timestamp: row.get(2)?,
                        success: row.get::<_, i32>(3)? != 0,
                        message: row.get(4)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();

            Ok(logs)
        })
    }

    pub async fn get_file_transfers(
        &self,
        limit: i64,
    ) -> Result<Vec<crate::file_transfer::FileTransferInfo>> {
        let limit = clamp_limit(limit);
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
            "SELECT id, name, size, mime, from_device, to_device, status, chunks_received, total_chunks, saved_path, timestamp
             FROM file_transfers ORDER BY timestamp DESC LIMIT ?1",
        )?;

            let transfers = stmt
                .query_map(params![limit], |row| {
                    Ok(crate::file_transfer::FileTransferInfo {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        size: row.get::<_, i64>(2)? as u64,
                        mime: row.get(3)?,
                        from_device: row.get(4)?,
                        to_device: row.get(5)?,
                        status: row.get(6)?,
                        chunks_received: row.get::<_, i32>(7)? as u32,
                        total_chunks: row.get::<_, i32>(8)? as u32,
                        saved_path: row.get(9)?,
                        timestamp: row.get(10)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();

            Ok(transfers)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    // ── Test helpers ──────────────────────────────────────────────────────────

    /// Create an in-memory Storage instance with the production schema applied.
    /// No encryption is used so tests stay fast and portable.
    fn create_test_storage() -> Storage {
        let mut db = Connection::open_in_memory().expect("failed to open in-memory db");
        db.execute_batch("PRAGMA journal_mode=WAL;")
            .expect("failed to set WAL");
        run_migrations(&mut db).expect("failed to run migrations");
        Storage::from_connection(db)
    }

    fn make_device(id: &str) -> StoredDevice {
        StoredDevice {
            id: id.to_string(),
            name: format!("Device {}", id),
            device_type: "phone".to_string(),
            os: "Android".to_string(),
            public_key: format!("pk_{}", id),
            shared_secret: format!("secret_{}", id),
            paired_at: 1000,
            last_seen: 2000,
            battery: Some(85),
            signal: Some("strong".to_string()),
            status: "paired".to_string(),
        }
    }

    fn make_rule(id: &str) -> crate::automation::AutomationRule {
        crate::automation::AutomationRule {
            id: id.to_string(),
            name: format!("Rule {}", id),
            enabled: true,
            trigger: crate::automation::TriggerType::DeviceConnect {
                device_id: "*".to_string(),
            },
            action: crate::automation::ActionType::SendNotification {
                title: "Alert".to_string(),
                body: "Device connected".to_string(),
            },
            trusted_source_only: false,
        }
    }

    // ── Device storage tests ──────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_insert_device() {
        let storage = create_test_storage();
        let device = make_device("dev_001");
        storage.save_device(&device).await.unwrap();

        let retrieved = storage.get_device("dev_001").await.unwrap();
        assert!(retrieved.is_some(), "device should exist after insert");

        let d = retrieved.unwrap();
        assert_eq!(d.id, "dev_001");
        assert_eq!(d.name, "Device dev_001");
        assert_eq!(d.device_type, "phone");
        assert_eq!(d.os, "Android");
        assert_eq!(d.public_key, "pk_dev_001");
        assert_eq!(d.shared_secret, "secret_dev_001");
        assert_eq!(d.paired_at, 1000);
        assert_eq!(d.last_seen, 2000);
        assert_eq!(d.battery, Some(85));
        assert_eq!(d.signal, Some("strong".to_string()));
        assert_eq!(d.status, "paired");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_get_all_devices() {
        let storage = create_test_storage();
        storage.save_device(&make_device("d1")).await.unwrap();
        storage.save_device(&make_device("d2")).await.unwrap();
        storage.save_device(&make_device("d3")).await.unwrap();

        let devices = storage.get_all_devices().await.unwrap();
        assert_eq!(devices.len(), 3);
        let ids: Vec<&str> = devices.iter().map(|d| d.id.as_str()).collect();
        assert!(ids.contains(&"d1"));
        assert!(ids.contains(&"d2"));
        assert!(ids.contains(&"d3"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_delete_device() {
        let storage = create_test_storage();
        storage.save_device(&make_device("d1")).await.unwrap();
        assert!(storage.get_device("d1").await.unwrap().is_some());

        storage.delete_device("d1").await.unwrap();
        assert!(
            storage.get_device("d1").await.unwrap().is_none(),
            "device should be gone after delete"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_update_device_status() {
        let storage = create_test_storage();
        storage.save_device(&make_device("d1")).await.unwrap();

        storage.update_status("d1", "connected").await.unwrap();
        let d = storage.get_device("d1").await.unwrap().unwrap();
        assert_eq!(d.status, "connected");
        // last_seen should have been updated (>= the original value)
        assert!(d.last_seen >= 2000);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_update_battery() {
        let storage = create_test_storage();
        storage.save_device(&make_device("d1")).await.unwrap();

        storage.update_battery("d1", 42).await.unwrap();
        let d = storage.get_device("d1").await.unwrap().unwrap();
        assert_eq!(d.battery, Some(42));

        // Update to None-like value (0 is still Some)
        storage.update_battery("d1", 0).await.unwrap();
        let d = storage.get_device("d1").await.unwrap().unwrap();
        assert_eq!(d.battery, Some(0));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_device_not_found() {
        let storage = create_test_storage();
        let result = storage.get_device("nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_device_upsert() {
        let storage = create_test_storage();
        let mut device = make_device("d1");
        storage.save_device(&device).await.unwrap();

        // Modify and re-save — INSERT OR REPLACE should update the row
        device.name = "Updated Name".to_string();
        device.battery = Some(10);
        device.status = "online".to_string();
        storage.save_device(&device).await.unwrap();

        let d = storage.get_device("d1").await.unwrap().unwrap();
        assert_eq!(d.name, "Updated Name");
        assert_eq!(d.battery, Some(10));
        assert_eq!(d.status, "online");
        // Should still be only one device
        assert_eq!(storage.get_all_devices().await.unwrap().len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_get_all_devices_ordered_by_last_seen_desc() {
        let storage = create_test_storage();
        let mut d1 = make_device("d1");
        d1.last_seen = 100;
        let mut d2 = make_device("d2");
        d2.last_seen = 300;
        let mut d3 = make_device("d3");
        d3.last_seen = 200;

        storage.save_device(&d1).await.unwrap();
        storage.save_device(&d2).await.unwrap();
        storage.save_device(&d3).await.unwrap();

        let devices = storage.get_all_devices().await.unwrap();
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].id, "d2"); // 300 — most recent
        assert_eq!(devices[1].id, "d3"); // 200
        assert_eq!(devices[2].id, "d1"); // 100 — oldest
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_device_with_null_optional_fields() {
        let storage = create_test_storage();
        let mut device = make_device("d1");
        device.battery = None;
        device.signal = None;
        storage.save_device(&device).await.unwrap();

        let d = storage.get_device("d1").await.unwrap().unwrap();
        assert!(d.battery.is_none());
        assert!(d.signal.is_none());
    }

    // ── Clipboard tests ───────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_add_clipboard_entry() {
        let storage = create_test_storage();
        storage
            .save_clipboard("hello world", "text/plain", "dev_001", 1000)
            .await
            .unwrap();

        let entries = storage.get_clipboard_history(10).await.unwrap();
        assert_eq!(entries.len(), 1);

        let e = &entries[0];
        assert!(e.id > 0, "autoincrement id should be positive");
        assert_eq!(e.content, "hello world");
        assert_eq!(e.mime, "text/plain");
        assert_eq!(e.source_device, "dev_001");
        assert_eq!(e.timestamp, 1000);
        assert!(!e.pinned);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_get_clipboard_history_order() {
        let storage = create_test_storage();
        storage
            .save_clipboard("first", "text/plain", "d", 100)
            .await
            .unwrap();
        storage
            .save_clipboard("second", "text/plain", "d", 200)
            .await
            .unwrap();
        storage
            .save_clipboard("third", "text/plain", "d", 300)
            .await
            .unwrap();

        let entries = storage.get_clipboard_history(10).await.unwrap();
        assert_eq!(entries.len(), 3);
        // Newest first (timestamp DESC)
        assert_eq!(entries[0].content, "third");
        assert_eq!(entries[0].timestamp, 300);
        assert_eq!(entries[1].content, "second");
        assert_eq!(entries[1].timestamp, 200);
        assert_eq!(entries[2].content, "first");
        assert_eq!(entries[2].timestamp, 100);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_clipboard_pinned_appears_first() {
        let storage = create_test_storage();
        storage
            .save_clipboard("old_unpinned", "text/plain", "d", 100)
            .await
            .unwrap();
        storage
            .save_clipboard("new_unpinned", "text/plain", "d", 200)
            .await
            .unwrap();

        // Pin the older entry
        let entries = storage.get_clipboard_history(10).await.unwrap();
        let old_id = entries[1].id; // "old_unpinned" is second (newest first)
        storage.toggle_clipboard_pin(old_id).await.unwrap();

        let entries = storage.get_clipboard_history(10).await.unwrap();
        // Pinned entries sort first, then by timestamp DESC within each group
        assert_eq!(entries[0].content, "old_unpinned");
        assert!(entries[0].pinned);
        assert_eq!(entries[1].content, "new_unpinned");
        assert!(!entries[1].pinned);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_clipboard_history_limit() {
        let storage = create_test_storage();
        for i in 0..20 {
            storage
                .save_clipboard(&format!("entry_{}", i), "text/plain", "d", i)
                .await
                .unwrap();
        }
        let entries = storage.get_clipboard_history(5).await.unwrap();
        assert_eq!(entries.len(), 5);
        // Should be the 5 newest
        assert_eq!(entries[0].content, "entry_19");
        assert_eq!(entries[1].content, "entry_18");
        assert_eq!(entries[4].content, "entry_15");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_delete_clipboard_entry() {
        let storage = create_test_storage();
        storage
            .save_clipboard("to_delete", "text/plain", "d", 100)
            .await
            .unwrap();
        storage
            .save_clipboard("to_keep", "text/plain", "d", 200)
            .await
            .unwrap();

        let entries = storage.get_clipboard_history(10).await.unwrap();
        // get_clipboard_history returns newest first, so entries[1] is "to_delete" (timestamp 100)
        let delete_id = entries[1].id;
        storage.delete_clipboard_entry(delete_id).await.unwrap();

        let entries = storage.get_clipboard_history(10).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].content, "to_keep");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_toggle_clipboard_pin() {
        let storage = create_test_storage();
        storage
            .save_clipboard("hello", "text/plain", "d", 100)
            .await
            .unwrap();

        let entries = storage.get_clipboard_history(10).await.unwrap();
        let id = entries[0].id;
        assert!(!entries[0].pinned);

        // Pin
        storage.toggle_clipboard_pin(id).await.unwrap();
        let entries = storage.get_clipboard_history(10).await.unwrap();
        assert!(entries[0].pinned);

        // Unpin
        storage.toggle_clipboard_pin(id).await.unwrap();
        let entries = storage.get_clipboard_history(10).await.unwrap();
        assert!(!entries[0].pinned);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_clear_clipboard_history_preserves_pinned() {
        let storage = create_test_storage();
        storage
            .save_clipboard("a", "text/plain", "d", 100)
            .await
            .unwrap();
        storage
            .save_clipboard("b", "text/plain", "d", 200)
            .await
            .unwrap();
        storage
            .save_clipboard("c", "text/plain", "d", 300)
            .await
            .unwrap();

        // Pin entry "a" (oldest, so it's last in the list)
        let entries = storage.get_clipboard_history(10).await.unwrap();
        let pin_id = entries[2].id; // "a"
        storage.toggle_clipboard_pin(pin_id).await.unwrap();

        storage.clear_clipboard_history().await.unwrap();

        let entries = storage.get_clipboard_history(10).await.unwrap();
        assert_eq!(entries.len(), 1, "only pinned entry should survive");
        assert_eq!(entries[0].content, "a");
        assert!(entries[0].pinned);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_clipboard_auto_prune_unpinned() {
        let storage = create_test_storage();
        // Insert MAX_CLIPBOARD_ENTRIES + 10 unpinned entries
        let count = Storage::MAX_CLIPBOARD_ENTRIES + 10;
        for i in 0..count {
            storage
                .save_clipboard(&format!("entry_{}", i), "text/plain", "d", i as i64)
                .await
                .unwrap();
        }
        let entries = storage.get_clipboard_history(count as i64).await.unwrap();
        assert_eq!(
            entries.len(),
            Storage::MAX_CLIPBOARD_ENTRIES,
            "oldest unpinned entries should be auto-pruned"
        );
    }

    // ── Notification tests ────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_add_notification() {
        let storage = create_test_storage();
        let p = NotificationParams {
            id: "n1",
            device_id: "d1",
            app: "WhatsApp",
            title: "New Message",
            body: "Hello from Alice!",
            timestamp: 1000,
            actions: Some("[\"reply\"]"),
        };
        storage.save_notification(&p).await.unwrap();

        let notifications = storage.get_notifications(10).await.unwrap();
        assert_eq!(notifications.len(), 1);

        let n = &notifications[0];
        assert_eq!(n.id, "n1");
        assert_eq!(n.device_id, "d1");
        assert_eq!(n.app, "WhatsApp");
        assert_eq!(n.title, "New Message");
        assert_eq!(n.body, "Hello from Alice!");
        assert_eq!(n.timestamp, 1000);
        assert_eq!(n.actions, Some("[\"reply\"]".to_string()));
        assert!(!n.dismissed);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dismiss_notification() {
        let storage = create_test_storage();
        let p = NotificationParams {
            id: "n1",
            device_id: "d1",
            app: "test",
            title: "t",
            body: "b",
            timestamp: 1000,
            actions: None,
        };
        storage.save_notification(&p).await.unwrap();

        // Before dismiss
        let n = &storage.get_notifications(10).await.unwrap()[0];
        assert!(!n.dismissed);

        storage.dismiss_notification("n1").await.unwrap();

        let n = &storage.get_notifications(10).await.unwrap()[0];
        assert!(n.dismissed);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_notification_history_order() {
        let storage = create_test_storage();
        for i in 0..5 {
            let p = NotificationParams {
                id: &format!("n{}", i),
                device_id: "d1",
                app: "test",
                title: &format!("title_{}", i),
                body: "b",
                timestamp: i * 100,
                actions: None,
            };
            storage.save_notification(&p).await.unwrap();
        }

        let notifications = storage.get_notifications(10).await.unwrap();
        assert_eq!(notifications.len(), 5);
        // Newest first
        assert_eq!(notifications[0].timestamp, 400);
        assert_eq!(notifications[0].title, "title_4");
        assert_eq!(notifications[4].timestamp, 0);
        assert_eq!(notifications[4].title, "title_0");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_notification_limit() {
        let storage = create_test_storage();
        for i in 0..10 {
            let p = NotificationParams {
                id: &format!("n{}", i),
                device_id: "d1",
                app: "test",
                title: "t",
                body: "b",
                timestamp: i,
                actions: None,
            };
            storage.save_notification(&p).await.unwrap();
        }

        let notifications = storage.get_notifications(3).await.unwrap();
        assert_eq!(notifications.len(), 3);
        // Should be the 3 newest (timestamps 9, 8, 7)
        assert_eq!(notifications[0].timestamp, 9);
        assert_eq!(notifications[1].timestamp, 8);
        assert_eq!(notifications[2].timestamp, 7);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_notification_upsert_same_id() {
        let storage = create_test_storage();
        let p = NotificationParams {
            id: "n1",
            device_id: "d1",
            app: "test",
            title: "old title",
            body: "old body",
            timestamp: 100,
            actions: None,
        };
        storage.save_notification(&p).await.unwrap();

        let p2 = NotificationParams {
            id: "n1",
            device_id: "d1",
            app: "test",
            title: "new title",
            body: "new body",
            timestamp: 200,
            actions: Some("[\"dismiss\"]"),
        };
        storage.save_notification(&p2).await.unwrap();

        let notifications = storage.get_notifications(10).await.unwrap();
        assert_eq!(notifications.len(), 1, "should be upserted, not duplicated");
        assert_eq!(notifications[0].title, "new title");
        assert_eq!(notifications[0].body, "new body");
        assert_eq!(notifications[0].timestamp, 200);
    }

    // ── File transfer tests ───────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_add_file_transfer() {
        let storage = create_test_storage();
        let p = FileTransferParams {
            id: "ft1",
            name: "photo.jpg",
            size: 1024,
            mime: "image/jpeg",
            from_device: "d1",
            to_device: "d2",
            status: "pending",
            chunks_received: 0,
            total_chunks: 5,
            saved_path: None,
            timestamp: 1000,
        };
        storage.save_file_transfer(&p).await.unwrap();

        let transfers = storage.get_file_transfers(10).await.unwrap();
        assert_eq!(transfers.len(), 1);

        let t = &transfers[0];
        assert_eq!(t.id, "ft1");
        assert_eq!(t.name, "photo.jpg");
        assert_eq!(t.size, 1024);
        assert_eq!(t.mime, "image/jpeg");
        assert_eq!(t.from_device, "d1");
        assert_eq!(t.to_device, "d2");
        assert_eq!(t.status, "pending");
        assert_eq!(t.chunks_received, 0);
        assert_eq!(t.total_chunks, 5);
        assert!(t.saved_path.is_none());
        assert_eq!(t.timestamp, 1000);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_update_transfer_status() {
        let storage = create_test_storage();
        let p = FileTransferParams {
            id: "ft1",
            name: "f.bin",
            size: 256,
            mime: "application/octet-stream",
            from_device: "d1",
            to_device: "d2",
            status: "pending",
            chunks_received: 0,
            total_chunks: 4,
            saved_path: None,
            timestamp: 1000,
        };
        storage.save_file_transfer(&p).await.unwrap();

        storage
            .update_file_transfer_progress("ft1", "transferring", 2, None)
            .await
            .unwrap();

        let t = &storage.get_file_transfers(10).await.unwrap()[0];
        assert_eq!(t.status, "transferring");
        assert_eq!(t.chunks_received, 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_update_transfer_saved_path() {
        let storage = create_test_storage();
        let p = FileTransferParams {
            id: "ft1",
            name: "f.bin",
            size: 64,
            mime: "application/octet-stream",
            from_device: "d1",
            to_device: "d2",
            status: "transferring",
            chunks_received: 1,
            total_chunks: 1,
            saved_path: None,
            timestamp: 1000,
        };
        storage.save_file_transfer(&p).await.unwrap();

        storage
            .update_file_transfer_progress("ft1", "complete", 1, Some("/downloads/f.bin"))
            .await
            .unwrap();

        let t = &storage.get_file_transfers(10).await.unwrap()[0];
        assert_eq!(t.status, "complete");
        assert_eq!(t.saved_path, Some("/downloads/f.bin".to_string()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_get_file_transfers_limit() {
        let storage = create_test_storage();
        for i in 0..5 {
            let p = FileTransferParams {
                id: &format!("ft{}", i),
                name: "f",
                size: 0,
                mime: "text/plain",
                from_device: "d1",
                to_device: "d2",
                status: "pending",
                chunks_received: 0,
                total_chunks: 0,
                saved_path: None,
                timestamp: i * 100,
            };
            storage.save_file_transfer(&p).await.unwrap();
        }

        let transfers = storage.get_file_transfers(2).await.unwrap();
        assert_eq!(transfers.len(), 2);
        // Should be the newest (timestamps 400, 300)
        assert_eq!(transfers[0].timestamp, 400);
        assert_eq!(transfers[1].timestamp, 300);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_file_transfer_upsert() {
        let storage = create_test_storage();
        let p = FileTransferParams {
            id: "ft1",
            name: "original.bin",
            size: 100,
            mime: "application/octet-stream",
            from_device: "d1",
            to_device: "d2",
            status: "pending",
            chunks_received: 0,
            total_chunks: 2,
            saved_path: None,
            timestamp: 1000,
        };
        storage.save_file_transfer(&p).await.unwrap();

        let p2 = FileTransferParams {
            id: "ft1",
            name: "updated.bin",
            size: 200,
            mime: "application/pdf",
            from_device: "d3",
            to_device: "d4",
            status: "complete",
            chunks_received: 2,
            total_chunks: 2,
            saved_path: Some("/path/file.pdf"),
            timestamp: 2000,
        };
        storage.save_file_transfer(&p2).await.unwrap();

        let transfers = storage.get_file_transfers(10).await.unwrap();
        assert_eq!(transfers.len(), 1, "should be upserted");
        assert_eq!(transfers[0].name, "updated.bin");
        assert_eq!(transfers[0].size, 200);
        assert_eq!(transfers[0].status, "complete");
    }

    // ── Automation rules tests ────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_add_rule() {
        let storage = create_test_storage();
        let rule = make_rule("r1");
        storage.save_automation_rule(&rule).await.unwrap();

        let rules = storage.get_all_automation_rules().await.unwrap();
        assert_eq!(rules.len(), 1);

        let r = &rules[0];
        assert_eq!(r.id, "r1");
        assert_eq!(r.name, "Rule r1");
        assert!(r.enabled);
        assert_eq!(
            r.trigger,
            crate::automation::TriggerType::DeviceConnect {
                device_id: "*".to_string()
            }
        );
        assert_eq!(
            r.action,
            crate::automation::ActionType::SendNotification {
                title: "Alert".to_string(),
                body: "Device connected".to_string(),
            }
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_update_rule() {
        let storage = create_test_storage();
        let mut rule = make_rule("r1");
        storage.save_automation_rule(&rule).await.unwrap();

        // Modify all mutable fields
        rule.name = "Updated Rule".to_string();
        rule.enabled = false;
        rule.trigger = crate::automation::TriggerType::Time {
            time: "08:00".to_string(),
        };
        rule.action = crate::automation::ActionType::ToggleWiFi { enabled: true };
        rule.trusted_source_only = true;
        storage.save_automation_rule(&rule).await.unwrap();

        let rules = storage.get_all_automation_rules().await.unwrap();
        assert_eq!(rules.len(), 1, "should still be exactly one rule");

        let r = &rules[0];
        assert_eq!(r.name, "Updated Rule");
        assert!(!r.enabled);
        assert_eq!(
            r.trigger,
            crate::automation::TriggerType::Time {
                time: "08:00".to_string()
            }
        );
        assert_eq!(
            r.action,
            crate::automation::ActionType::ToggleWiFi { enabled: true }
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_delete_rule() {
        let storage = create_test_storage();
        let rule = make_rule("r1");
        storage.save_automation_rule(&rule).await.unwrap();
        // Also add a log entry for this rule
        storage
            .log_automation_execution("r1", "device_connect", 1000, true, Some("ok"))
            .await
            .unwrap();

        storage.delete_automation_rule("r1").await.unwrap();

        let rules = storage.get_all_automation_rules().await.unwrap();
        assert!(rules.is_empty(), "rule should be deleted");
        let logs = storage.get_automation_logs(10).await.unwrap();
        assert!(
            logs.is_empty(),
            "logs for deleted rule should also be removed"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_toggle_rule() {
        let storage = create_test_storage();
        let mut rule = make_rule("r1");
        storage.save_automation_rule(&rule).await.unwrap();

        // Disable
        rule.enabled = false;
        storage.save_automation_rule(&rule).await.unwrap();
        let rules = storage.get_all_automation_rules().await.unwrap();
        assert!(!rules[0].enabled);

        // Re-enable
        rule.enabled = true;
        storage.save_automation_rule(&rule).await.unwrap();
        let rules = storage.get_all_automation_rules().await.unwrap();
        assert!(rules[0].enabled);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_multiple_rules_all_present() {
        let storage = create_test_storage();
        let mut rule1 = make_rule("r1");
        rule1.name = "First".to_string();
        let mut rule2 = make_rule("r2");
        rule2.name = "Second".to_string();
        let mut rule3 = make_rule("r3");
        rule3.name = "Third".to_string();

        storage.save_automation_rule(&rule1).await.unwrap();
        storage.save_automation_rule(&rule2).await.unwrap();
        storage.save_automation_rule(&rule3).await.unwrap();

        let rules = storage.get_all_automation_rules().await.unwrap();
        assert_eq!(rules.len(), 3);
        let ids: Vec<&str> = rules.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"r1"));
        assert!(ids.contains(&"r2"));
        assert!(ids.contains(&"r3"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_rule_with_different_trigger_action_types() {
        let storage = create_test_storage();

        let rule = crate::automation::AutomationRule {
            id: "r_time".to_string(),
            name: "Morning routine".to_string(),
            enabled: true,
            trigger: crate::automation::TriggerType::Time {
                time: "07:30".to_string(),
            },
            action: crate::automation::ActionType::RunShellCommand {
                command: "echo morning".to_string(),
            },
            trusted_source_only: false,
        };
        storage.save_automation_rule(&rule).await.unwrap();

        let rules = storage.get_all_automation_rules().await.unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].trigger,
            crate::automation::TriggerType::Time {
                time: "07:30".to_string()
            }
        );
        assert_eq!(
            rules[0].action,
            crate::automation::ActionType::RunShellCommand {
                command: "echo morning".to_string()
            }
        );
    }

    // ── Automation logs tests ─────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_log_automation_execution() {
        let storage = create_test_storage();
        storage
            .log_automation_execution("r1", "device_connect", 1000, true, Some("connected"))
            .await
            .unwrap();

        let logs = storage.get_automation_logs(10).await.unwrap();
        assert_eq!(logs.len(), 1);

        let l = &logs[0];
        assert_eq!(l.id, "r1");
        assert_eq!(l.trigger_type, "device_connect");
        assert_eq!(l.timestamp, 1000);
        assert!(l.success);
        assert_eq!(l.message, Some("connected".to_string()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_automation_logs_order() {
        let storage = create_test_storage();
        storage
            .log_automation_execution("r1", "trigger", 100, true, None)
            .await
            .unwrap();
        storage
            .log_automation_execution("r1", "trigger", 300, true, None)
            .await
            .unwrap();
        storage
            .log_automation_execution("r1", "trigger", 200, false, None)
            .await
            .unwrap();

        let logs = storage.get_automation_logs(10).await.unwrap();
        assert_eq!(logs.len(), 3);
        // Newest first
        assert_eq!(logs[0].timestamp, 300);
        assert_eq!(logs[1].timestamp, 200);
        assert_eq!(logs[2].timestamp, 100);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_automation_logs_limit() {
        let storage = create_test_storage();
        for i in 0..5 {
            storage
                .log_automation_execution("r1", "trigger", i, true, None)
                .await
                .unwrap();
        }

        let logs = storage.get_automation_logs(2).await.unwrap();
        assert_eq!(logs.len(), 2);
        // Should be the newest (timestamps 4, 3)
        assert_eq!(logs[0].timestamp, 4);
        assert_eq!(logs[1].timestamp, 3);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_automation_log_failure_message() {
        let storage = create_test_storage();
        storage
            .log_automation_execution(
                "r1",
                "run_shell_command",
                1000,
                false,
                Some("command not found"),
            )
            .await
            .unwrap();

        let log = &storage.get_automation_logs(10).await.unwrap()[0];
        assert!(!log.success);
        assert_eq!(log.message, Some("command not found".to_string()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_automation_log_none_message() {
        let storage = create_test_storage();
        storage
            .log_automation_execution("r1", "toggle_wifi", 1000, true, None)
            .await
            .unwrap();

        let log = &storage.get_automation_logs(10).await.unwrap()[0];
        assert!(log.success);
        assert!(log.message.is_none());
    }

    // ── Settings tests ────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_save_and_get_setting() {
        let storage = create_test_storage();
        storage.save_setting("theme", "dark").await.unwrap();
        assert_eq!(storage.get_setting("theme").await, Some("dark".to_string()));
        assert_eq!(storage.get_setting("nonexistent").await, None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_save_setting_upsert() {
        let storage = create_test_storage();
        storage.save_setting("theme", "dark").await.unwrap();
        storage.save_setting("theme", "light").await.unwrap();
        assert_eq!(
            storage.get_setting("theme").await,
            Some("light".to_string())
        );

        // Should still be only one row
        let conn = storage.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_get_settings_defaults() {
        let storage = create_test_storage();
        let settings = storage.get_settings().await.unwrap();
        // Verify defaults from get_settings when no rows exist
        assert_eq!(settings.max_devices, 5);
        assert!(settings.sync_notifications);
        assert!(settings.sync_clipboard);
        assert!(settings.sync_files);
        assert_eq!(settings.theme, "dark");
        assert_eq!(settings.accent_color, "#00f0ff");
        assert!(settings.minimize_to_tray);
        assert!(settings.auto_accept_files);
        assert!(settings.notifications_enabled);
        assert_eq!(settings.relay_url, "ws://127.0.0.1:9531");
        assert!(
            settings.relay_enabled,
            "an empty database must read as relay_enabled = true"
        );
        assert_eq!(settings.relay_port, crate::relay::DEFAULT_RELAY_PORT);
        assert_eq!(
            settings.relay_health_port,
            crate::relay::DEFAULT_RELAY_HEALTH_PORT
        );
        assert_eq!(
            settings.notification_apps,
            crate::commands::default_notification_apps()
        );
        assert!(settings.default_download_folder.is_empty());
        assert!(settings.last_version.is_empty());
    }

    /// REGRESSION — `get_settings` read a missing `relay_enabled` row as the
    /// literal `"false"`, contradicting `ConduitSettings::default()`, the
    /// `#[serde(default = "default_true")]` attribute and
    /// `settingsTypes.ts`'s `DEFAULT_SETTINGS`, which all say "on".
    ///
    /// Nothing seeded the row: `migrations/001_initial.sql` is DDL with no
    /// `INSERT`s, and no writer runs before `main.rs`'s startup read. `main.rs`
    /// then returns early on a false `relay_enabled`, skipping *both*
    /// `relay_host.start()` and `spawn_relay_client` — so on a fresh install the
    /// relay never started and the desktop never joined its own relay.
    ///
    /// This reads the function `main.rs` actually calls. Asserting
    /// `ConduitSettings::default()` alone did not catch the defect, because the
    /// struct default and this read are two different definitions of one thing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_get_settings_fresh_install_starts_the_relay() {
        let storage = create_test_storage();

        let settings = storage.get_settings().await.unwrap();
        assert!(
            settings.relay_enabled,
            "a fresh install must read relay_enabled = true; main.rs skips \
             relay_host.start() and spawn_relay_client when it is false"
        );
        assert_eq!(
            storage.get_setting("relay_enabled").await,
            None,
            "no migration seeds this key — the value comes from the default, \
             which is the whole point of the test"
        );
        // The relay must actually start from the settings `get_settings` returns,
        // not merely be present in the struct.
        assert!(
            settings.relay_port == crate::relay::DEFAULT_RELAY_PORT
                && settings.relay_health_port == crate::relay::DEFAULT_RELAY_HEALTH_PORT,
            "a fresh install must read the ports the in-process relay binds"
        );
    }

    /// Structural guard for the drift above: every default `get_settings`
    /// applies to an empty database must be the one `ConduitSettings::default()`
    /// documents, for the whole struct and not one hand-picked field.
    ///
    /// `device_name` is excluded because it has *three* separate definitions
    /// (`ConduitSettings::default()` → `""`, this read → `gethostname()`,
    /// `DEFAULT_SETTINGS` → `"Desktop"`). Rather than bless any one of them, the
    /// test carries its actual value through, so it neither hides nor pins that
    /// drift — but it fails on any *other* field disagreeing, which is what
    /// catches `relay_enabled`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_get_settings_defaults_match_conduit_settings_default() {
        let storage = create_test_storage();

        let fresh = storage.get_settings().await.unwrap();
        let expected = crate::commands::ConduitSettings {
            device_name: fresh.device_name.clone(),
            ..Default::default()
        };

        assert_eq!(
            fresh, expected,
            "Storage::get_settings' empty-database defaults have drifted from \
             ConduitSettings::default(); every field must agree except device_name"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_save_and_load_settings() {
        let storage = create_test_storage();
        let settings = crate::commands::ConduitSettings {
            device_name: "My PC".to_string(),
            max_devices: 10,
            sync_notifications: false,
            sync_clipboard: true,
            sync_files: false,
            notification_apps: vec!["Telegram".to_string(), "Signal".to_string()],
            theme: "light".to_string(),
            accent_color: "#ff0000".to_string(),
            last_version: "1.0.0".to_string(),
            minimize_to_tray: false,
            default_download_folder: "/tmp/downloads".to_string(),
            auto_accept_files: false,
            notifications_enabled: false,
            relay_url: "wss://relay.example:9528".to_string(),
            relay_enabled: false,
            relay_port: crate::relay::DEFAULT_RELAY_PORT,
            relay_health_port: crate::relay::DEFAULT_RELAY_HEALTH_PORT,
            relay_hostname: String::new(),
            relay_cert_pin: String::new(),
        };
        storage.save_settings(&settings).await.unwrap();

        let loaded = storage.get_settings().await.unwrap();
        assert_eq!(loaded, settings, "every field must round-trip");
    }

    /// `save_settings` must write a row for *every* `ConduitSettings` field.
    ///
    /// This is the structural guard for the `relay_url` defect: the old chain of
    /// `conn.execute` calls simply had no `relay_url` entry, and nothing failed —
    /// the value was read back as a default. Comparing the rows actually written
    /// against the struct's serialised key set makes any future omission fail
    /// here instead of silently reverting a user choice.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_save_settings_writes_every_settings_key() {
        let storage = create_test_storage();

        // Deliberately non-default values so an upsert is observable.
        let mut settings = crate::commands::ConduitSettings {
            device_name: "Key Audit".to_string(),
            notification_apps: vec!["Signal".to_string()],
            relay_url: "wss://relay.example:9528".to_string(),
            relay_enabled: false,
            relay_port: crate::relay::DEFAULT_RELAY_PORT,
            relay_health_port: crate::relay::DEFAULT_RELAY_HEALTH_PORT,
            relay_hostname: String::new(),
            ..Default::default()
        };
        settings.max_devices = 9;
        settings.theme = "light".to_string();
        settings.accent_color = "#abcdef".to_string();
        settings.last_version = "9.9.9".to_string();
        settings.default_download_folder = "/tmp/x".to_string();
        settings.sync_notifications = false;
        settings.sync_clipboard = false;
        settings.sync_files = false;
        settings.minimize_to_tray = false;
        settings.auto_accept_files = false;
        settings.notifications_enabled = false;

        storage.save_settings(&settings).await.unwrap();

        let expected: Vec<String> = serde_json::to_value(&settings)
            .expect("settings serialise")
            .as_object()
            .expect("settings are an object")
            .keys()
            .cloned()
            .collect();

        for key in &expected {
            assert!(
                storage.get_setting(key).await.is_some(),
                "save_settings wrote no row for `{key}`"
            );
        }

        // `notification_apps` is stored as JSON, not as a bare list.
        assert_eq!(
            storage.get_setting("notification_apps").await.as_deref(),
            Some(r#"["Signal"]"#)
        );
        assert_eq!(
            storage.get_setting("relay_url").await.as_deref(),
            Some("wss://relay.example:9528")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_save_settings_overwrites_previous() {
        let storage = create_test_storage();

        let s1 = crate::commands::ConduitSettings {
            device_name: "First".to_string(),
            max_devices: 3,
            sync_notifications: true,
            sync_clipboard: true,
            sync_files: true,
            notification_apps: vec![],
            theme: "dark".to_string(),
            accent_color: "#00f0ff".to_string(),
            last_version: String::new(),
            minimize_to_tray: true,
            default_download_folder: String::new(),
            auto_accept_files: true,
            notifications_enabled: true,
            relay_url: crate::commands::DEFAULT_RELAY_URL.to_string(),
            relay_enabled: false,
            relay_port: crate::relay::DEFAULT_RELAY_PORT,
            relay_health_port: crate::relay::DEFAULT_RELAY_HEALTH_PORT,
            relay_hostname: String::new(),
            relay_cert_pin: String::new(),
        };
        storage.save_settings(&s1).await.unwrap();

        let s2 = crate::commands::ConduitSettings {
            device_name: "Second".to_string(),
            max_devices: 99,
            ..s1
        };
        storage.save_settings(&s2).await.unwrap();

        let loaded = storage.get_settings().await.unwrap();
        assert_eq!(loaded.device_name, "Second");
        assert_eq!(loaded.max_devices, 99);
    }

    /// REGRESSION (was the characterization test
    /// `test_save_settings_does_not_persist_relay_url`).
    ///
    /// `save_settings` used to write every `ConduitSettings` field *except*
    /// `relay_url` — the upsert chain simply ended at `analytics` — while
    /// `get_settings` read `relay_url` back from the `settings` table. A relay
    /// URL chosen in the UI was silently dropped and reverted to
    /// `ws://127.0.0.1:9528` on the next read — the default the standalone
    /// relay used. The test now pins the fix: the
    /// value is written and read back.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_save_and_load_settings_roundtrips_relay_url() {
        let storage = create_test_storage();

        let mut settings = crate::commands::ConduitSettings {
            device_name: "My PC".to_string(),
            ..Default::default()
        };
        settings.relay_url = "wss://relay.example:9528".to_string();
        storage.save_settings(&settings).await.unwrap();

        let loaded = storage.get_settings().await.unwrap();
        assert_eq!(
            loaded.relay_url, "wss://relay.example:9528",
            "a configured relay URL must survive save → load"
        );
        assert_eq!(
            storage.get_setting("relay_url").await.as_deref(),
            Some("wss://relay.example:9528"),
            "save_settings must write a relay_url row"
        );

        // …and a second save of the default must not resurrect a stale row.
        storage
            .save_settings(&crate::commands::ConduitSettings::default())
            .await
            .unwrap();
        assert_eq!(
            storage.get_settings().await.unwrap().relay_url,
            crate::commands::DEFAULT_RELAY_URL
        );
    }

    /// `notification_apps` is the other field whose persistence is load-bearing:
    /// it is the allowlist the WS dispatcher filters mirrored notifications with,
    /// and it is stored as a JSON array.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_save_and_load_settings_roundtrips_notification_apps() {
        let storage = create_test_storage();

        let settings = crate::commands::ConduitSettings {
            notification_apps: vec!["Signal".to_string(), "Mail".to_string()],
            ..Default::default()
        };
        storage.save_settings(&settings).await.unwrap();

        assert_eq!(
            storage.get_settings().await.unwrap().notification_apps,
            vec!["Signal".to_string(), "Mail".to_string()]
        );

        // An empty list is a valid, meaningful value ("mirror nothing") and must
        // not be confused with "unset" (which seeds the default list).
        let cleared = crate::commands::ConduitSettings {
            notification_apps: vec![],
            ..Default::default()
        };
        storage.save_settings(&cleared).await.unwrap();
        assert!(
            storage
                .get_settings()
                .await
                .unwrap()
                .notification_apps
                .is_empty(),
            "an explicitly emptied allowlist must persist as an empty list"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_multiple_individual_settings() {
        let storage = create_test_storage();
        storage.save_setting("key1", "value1").await.unwrap();
        storage.save_setting("key2", "value2").await.unwrap();
        storage.save_setting("key3", "value3").await.unwrap();

        assert_eq!(
            storage.get_setting("key1").await,
            Some("value1".to_string())
        );
        assert_eq!(
            storage.get_setting("key2").await,
            Some("value2".to_string())
        );
        assert_eq!(
            storage.get_setting("key3").await,
            Some("value3".to_string())
        );
        assert_eq!(storage.get_setting("missing").await, None);
    }

    // ── Cross-table integration tests ─────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_empty_database_returns_empty_results() {
        let storage = create_test_storage();
        assert!(storage.get_all_devices().await.unwrap().is_empty());
        assert!(storage.get_clipboard_history(100).await.unwrap().is_empty());
        assert!(storage.get_notifications(100).await.unwrap().is_empty());
        assert!(storage.get_file_transfers(100).await.unwrap().is_empty());
        assert!(storage.get_all_automation_rules().await.unwrap().is_empty());
        assert!(storage.get_automation_logs(100).await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_concurrent_writes_through_mutex() {
        let storage = create_test_storage();
        // Verify the Mutex works correctly by doing sequential operations
        // (true concurrency testing would require threads, but this validates
        // that the Mutex<Connection> pattern works without poisoning)
        for i in 0..50 {
            storage
                .save_clipboard(&format!("c{}", i), "text/plain", "d", i)
                .await
                .unwrap();
        }
        assert_eq!(storage.get_clipboard_history(100).await.unwrap().len(), 50);

        for i in 0..50 {
            storage
                .save_device(&make_device(&format!("d{}", i)))
                .await
                .unwrap();
        }
        assert_eq!(storage.get_all_devices().await.unwrap().len(), 50);
    }

    // ── Database open / recovery (the data-loss bug) ──────────────────────────

    /// Build a real SQLCipher-encrypted database at `db_path` containing one
    /// paired device, so the tests below exercise the production key path
    /// rather than a stub.
    fn write_encrypted_db_with_data(db_path: &Path, db_key: &str) {
        let conn = open_encrypted(db_path, db_key).expect("create encrypted db");
        conn.execute(
            "INSERT INTO devices (id, name, device_type, os, public_key, shared_secret, \
             paired_at, last_seen, status) VALUES ('dev_001', 'My Phone', 'phone', 'Android', \
             'pk', 'secret', 1, 2, 'paired')",
            [],
        )
        .expect("insert device");
    }

    fn count_devices(db_path: &Path, db_key: &str) -> i64 {
        let conn = open_encrypted(db_path, db_key).expect("open encrypted db");
        conn.query_row("SELECT COUNT(*) FROM devices", [], |r| r.get(0))
            .expect("count devices")
    }

    /// The `corduit.corrupt_<timestamp>.bak` files this module manages. The
    /// timestamp must be numeric — that is what `prune_corrupt_backups` parses
    /// — so a same-shaped file from anything else is deliberately not counted.
    fn corrupt_backups_in(dir: &Path) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
            .expect("read dir")
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.strip_prefix("conduit.corrupt_"))
                    .and_then(|n| n.strip_suffix(".bak"))
                    .is_some_and(|ts| ts.parse::<u64>().is_ok())
            })
            .collect();
        found.sort();
        found
    }

    /// REGRESSION — the data-loss bug.
    ///
    /// A database whose key cannot be recovered fails to open *identically* to
    /// a corrupt one, and the old recovery path could not tell them apart: it
    /// renamed `conduit.db` to `corrupt_<ts>.bak` and opened a brand-new empty
    /// database. Combined with a discarded `set_password` in the keyring path,
    /// that silently wiped the user's settings, paired devices, notifications
    /// and clipboard history on every launch.
    ///
    /// This asserts all three halves of the fix: the file is recognised as
    /// intact, it is NOT moved or replaced, and it is still readable — with its
    /// data — once the original key is available again.
    #[test]
    fn wrong_key_leaves_an_intact_database_untouched() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("conduit.db");
        let real_key = crate::encryption::generate_token_hex();
        write_encrypted_db_with_data(&db_path, &real_key);

        assert_eq!(
            classify_db_file(&db_path),
            DbFileKind::SqlCipher,
            "precondition: an encrypted database must be recognised as intact"
        );
        let bytes_before = std::fs::read(&db_path).expect("read before");

        // Simulate a lost key: `keyring` returned a different secret.
        let wrong_key = crate::encryption::generate_token_hex();
        let err = open_or_recover(&db_path, &wrong_key)
            .expect_err("a wrong key must not produce a usable connection");
        let msg = err.to_string();
        assert!(
            msg.contains("INTACT"),
            "the error must say the file is intact and this is a key problem: {msg}"
        );
        assert!(
            msg.contains("has NOT been moved, renamed or overwritten"),
            "the error must state that the data was left alone: {msg}"
        );
        assert!(db_path.exists(), "the database must still be on disk");
        assert_eq!(
            std::fs::read(&db_path).expect("read after"),
            bytes_before,
            "the database must be byte-for-byte unchanged"
        );
        assert_eq!(
            classify_db_file(&db_path),
            DbFileKind::SqlCipher,
            "the database must not have been replaced by an empty one"
        );
        assert!(
            corrupt_backups_in(dir.path()).is_empty(),
            "an intact database must never be quarantined"
        );
        // The message must name both places the old key could come from, or it
        // is not actionable.
        assert!(
            msg.contains("sqlite_key.key"),
            "the error must name the fallback key file: {msg}"
        );
        assert!(
            msg.contains(KEYRING_SERVICE),
            "the error must name the keyring entry: {msg}"
        );

        // And it is still fully recoverable with the right key.
        assert_eq!(
            count_devices(&db_path, &real_key),
            1,
            "the user's data must survive a wrong-key launch"
        );
    }

    /// A genuinely unreadable file is quarantined — and the quarantine keeps
    /// the original bytes, so it is still recoverable by hand.
    #[test]
    fn genuinely_corrupt_database_is_quarantined_and_recreated() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("conduit.db");
        let key = crate::encryption::generate_token_hex();
        let junk = b"this is not a database, it is a text file that got out of hand";
        std::fs::write(&db_path, junk).expect("write junk");

        assert_eq!(classify_db_file(&db_path), DbFileKind::Unrecognised);

        let conn = open_or_recover(&db_path, &key)
            .expect("a corrupt file is quarantined and replaced, not fatal");
        assert_eq!(
            count_devices(&db_path, &key),
            0,
            "the replacement is a working, empty database"
        );
        drop(conn);

        let baks = corrupt_backups_in(dir.path());
        assert_eq!(baks.len(), 1, "the original must be preserved exactly once");
        assert_eq!(
            std::fs::read(&baks[0]).expect("read backup"),
            junk,
            "the quarantine must hold the original bytes, not a truncation"
        );
    }

    /// The `*.bak` files used to accumulate without bound — one per launch,
    /// each a full copy of the database, forever.
    #[test]
    fn repeated_corruption_does_not_accumulate_unbounded_backups() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("conduit.db");

        // Six previous launches, each of which quarantined a corrupt file.
        for ts in [
            1_000_000u64,
            1_000_001,
            1_000_002,
            1_000_003,
            1_000_004,
            1_000_005,
        ] {
            std::fs::write(dir.path().join(format!("conduit.corrupt_{ts}.bak")), b"old")
                .expect("seed backup");
        }

        std::fs::write(&db_path, b"not a database header either").expect("write junk");
        drop(
            open_or_recover(&db_path, &crate::encryption::generate_token_hex())
                .expect("quarantine and recreate"),
        );

        let baks = corrupt_backups_in(dir.path());
        assert_eq!(
            baks.len(),
            MAX_CORRUPT_BACKUPS,
            "quarantines must be capped, got {} files",
            baks.len()
        );
    }

    #[test]
    fn pruning_keeps_the_newest_backups() {
        let dir = tempfile::tempdir().expect("tempdir");
        for ts in 1..=7u64 {
            std::fs::write(dir.path().join(format!("conduit.corrupt_{ts}.bak")), b"x")
                .expect("seed");
        }
        // Unrelated files must be left alone.
        std::fs::write(dir.path().join("conduit.db"), b"live").expect("write live db");
        std::fs::write(dir.path().join("conduit.plaintext_bak"), b"x").expect("write other");
        std::fs::write(dir.path().join("conduit.corrupt_notanumber.bak"), b"x")
            .expect("write other");

        prune_corrupt_backups(&dir.path().join("conduit.db"), 3);

        // `corrupt_backups_in` sorts by file name, and the timestamp is zero
        // padded to the same width here, so name order is time order.
        let remaining: Vec<u64> = corrupt_backups_in(dir.path())
            .iter()
            .map(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.strip_prefix("conduit.corrupt_"))
                    .and_then(|n| n.strip_suffix(".bak"))
                    .and_then(|n| n.parse().ok())
                    .expect("numeric timestamp")
            })
            .collect();
        assert_eq!(remaining, vec![5, 6, 7], "the newest three must survive");
        assert!(
            dir.path().join("conduit.db").exists(),
            "the live db is not a backup"
        );
        assert!(
            dir.path().join("conduit.plaintext_bak").exists(),
            "only corrupt_*.bak files are pruned"
        );
        assert!(
            dir.path().join("conduit.corrupt_notanumber.bak").exists(),
            "files that are not ours to prune are left alone"
        );
    }

    /// A zero-length file has no data, so recreating it loses nothing.
    #[test]
    fn empty_database_file_is_recreated_without_data_loss() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("conduit.db");
        std::fs::write(&db_path, b"").expect("write empty");

        assert_eq!(classify_db_file(&db_path), DbFileKind::Empty);
        drop(
            open_or_recover(&db_path, &crate::encryption::generate_token_hex())
                .expect("an empty file is not a recovery case"),
        );
        assert!(corrupt_backups_in(dir.path()).is_empty());
        assert_eq!(classify_db_file(&db_path), DbFileKind::SqlCipher);
    }

    #[test]
    fn classify_recognises_a_plain_sqlite_file_as_intact() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("plain.db");
        let mut conn = Connection::open(&path).expect("open");
        run_migrations(&mut conn).expect("migrate");
        drop(conn);
        assert_eq!(classify_db_file(&path), DbFileKind::PlainSqlite);
    }

    /// The classifier is biased towards "intact", because only a wrong answer in
    /// that direction destroys data. These are the shapes that must land on the
    /// *other* side of the line.
    #[test]
    fn classify_rejects_files_that_cannot_be_a_database() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Missing file: nothing to lose, and nothing to justify a rename.
        assert_eq!(
            classify_db_file(&dir.path().join("absent.db")),
            DbFileKind::Empty
        );

        // Empty file: SQLite's own "new database" state.
        let empty = dir.path().join("empty.db");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(classify_db_file(&empty), DbFileKind::Empty);

        // Text file: not a whole number of pages.
        let text = dir.path().join("notes.db");
        std::fs::write(&text, b"shopping list\nmilk\neggs").unwrap();
        assert_eq!(classify_db_file(&text), DbFileKind::Unrecognised);

        // Zero-filled: a multiple of the page size, but the "salt" is not random
        // and a valid file can never look like this.
        let zeroes = dir.path().join("zeroes.db");
        std::fs::write(&zeroes, vec![0u8; 4096]).unwrap();
        assert_eq!(classify_db_file(&zeroes), DbFileKind::Unrecognised);

        // Truncated below a page: a partial write.
        let partial = dir.path().join("partial.db");
        std::fs::write(&partial, vec![0x41u8; 100]).unwrap();
        assert_eq!(classify_db_file(&partial), DbFileKind::Unrecognised);
    }

    // ── sqlcipher_export: the ATTACH path is now a bound parameter ─────────────

    /// REGRESSION — the destination path used to be interpolated into the
    /// `ATTACH DATABASE '<path>'` statement with only `\` → `/` substituted, so
    /// a `'` in the path closed the SQL literal. A single quote is a legal
    /// character in a filename on every platform Conduit ships on.
    #[test]
    fn sqlcipher_export_survives_a_path_containing_a_single_quote() {
        let dir = tempfile::tempdir().expect("tempdir");
        let quoted = dir.path().join("con'duit o'brien's data");
        std::fs::create_dir_all(&quoted).expect("create quoted dir");
        let plain = quoted.join("conduit.db");
        {
            let mut conn = Connection::open(&plain).expect("open plain");
            run_migrations(&mut conn).expect("migrate plain");
            conn.execute(
                "INSERT INTO devices (id, name, device_type, os, public_key, shared_secret, \
                 paired_at, last_seen, status) \
                 VALUES ('d1', 'Phone', 'phone', 'Android', 'pk', 's', 1, 2, 'paired')",
                [],
            )
            .expect("insert");
        }

        let key = crate::encryption::generate_token_hex();
        let encrypted = quoted.join("encrypted_tmp");
        export_plaintext_to_sqlcipher(&plain, &encrypted, &key)
            .expect("a quote in the path must not break the statement");

        assert!(encrypted.exists(), "the export must have produced a file");
        assert_eq!(classify_db_file(&encrypted), DbFileKind::SqlCipher);
        let conn = Connection::open(&encrypted).expect("open encrypted");
        conn.execute_batch(&format!("PRAGMA key = \"x'{key}'\";"))
            .expect("key");
        let devices: i64 = conn
            .query_row("SELECT COUNT(*) FROM devices", [], |r| r.get(0))
            .expect("count");
        assert_eq!(devices, 1, "the exported data must be readable");
    }

    /// The whole plaintext → encrypted upgrade, on a normal path this time.
    #[test]
    fn plaintext_database_is_encrypted_in_place_and_stays_readable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("conduit.db");
        {
            let mut conn = Connection::open(&db_path).expect("open plain");
            run_migrations(&mut conn).expect("migrate plain");
            conn.execute(
                "INSERT INTO devices (id, name, device_type, os, public_key, shared_secret, \
                 paired_at, last_seen, status) \
                 VALUES ('d1', 'Phone', 'phone', 'Android', 'pk', 's', 1, 2, 'paired')",
                [],
            )
            .expect("insert");
        }

        let key = crate::encryption::generate_token_hex();
        migrate_plaintext_db_to_encrypted(&db_path, &key).expect("migrate");

        assert_eq!(classify_db_file(&db_path), DbFileKind::SqlCipher);
        assert_eq!(
            count_devices(&db_path, &key),
            1,
            "data must survive the upgrade"
        );
        assert!(
            dir.path().join("conduit.plaintext_bak").exists(),
            "a plaintext copy is kept as the rollback for the swap"
        );

        // Second run must be a no-op, not a re-encrypt.
        migrate_plaintext_db_to_encrypted(&db_path, &key).expect("idempotent");
        assert_eq!(count_devices(&db_path, &key), 1);
    }

    #[test]
    fn a_non_hex_key_is_rejected_before_it_can_reach_a_pragma() {
        // The `KEY "x'…'"` / `PRAGMA key = "x'…'"` literals are only safe
        // because this check runs first.
        for bad in [
            "",
            "not hex at all",
            &"a".repeat(63),                                         // too short
            &format!("{}'; DROP TABLE devices; --", "a".repeat(40)), // injection attempt
            &"a".repeat(65),                                         // too long
        ] {
            assert!(
                validate_db_key(bad).is_err(),
                "{bad:?} must be rejected as a database key"
            );
        }
        assert!(validate_db_key(&crate::encryption::generate_token_hex()).is_ok());
    }

    // ── Settings: "unset" is not the same as "the read failed" ─────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_settings_propagates_a_failed_read_instead_of_returning_defaults() {
        // BUG (silent data loss). The old accessor collapsed a failed query into
        // "never set", so a transient SQL failure produced a struct full of
        // defaults that the next save wrote over the user's real values.
        let storage = create_test_storage();
        storage.save_setting("theme", "light").await.unwrap();
        {
            let conn = storage.conn.lock().unwrap();
            conn.execute("DROP TABLE settings", []).unwrap();
        }

        let err = storage
            .get_settings()
            .await
            .expect_err("a failed read must not be reported as 'unset'");
        assert!(
            matches!(err, ConduitError::Database(_)),
            "expected a Database error, got {err:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_missing_key_is_not_an_error() {
        let storage = create_test_storage();
        assert_eq!(
            storage.try_get_setting("never_written").await.unwrap(),
            None,
            "'never set' must still be reported as absent, not as a failure"
        );
        assert_eq!(
            storage.get_setting("never_written").await,
            None,
            "the infallible variant must agree"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn try_get_setting_reports_a_failed_read() {
        let storage = create_test_storage();
        storage.save_setting("theme", "light").await.unwrap();
        {
            let conn = storage.conn.lock().unwrap();
            conn.execute("DROP TABLE settings", []).unwrap();
        }
        let err = storage
            .try_get_setting("theme")
            .await
            .expect_err("a dropped table is a failure, not an absent key");
        assert!(matches!(err, ConduitError::Database(_)), "got {err:?}");
    }

    /// The `Setting_or_default` contract the settings page depends on: a missing
    /// row yields the default, a broken read yields an error. Both paths.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn setting_or_default_distinguishes_absent_from_failed() {
        let storage = create_test_storage();
        {
            let conn = storage.conn.lock().unwrap();
            assert_eq!(
                Storage::setting_or_default(&conn, "absent", "fallback").unwrap(),
                "fallback"
            );
        }
        storage.save_setting("present", "real").await.unwrap();
        {
            let conn = storage.conn.lock().unwrap();
            assert_eq!(
                Storage::setting_or_default(&conn, "present", "fallback").unwrap(),
                "real"
            );
            conn.execute("DROP TABLE settings", []).unwrap();
            assert!(
                Storage::setting_or_default(&conn, "present", "fallback").is_err(),
                "a failed read must not fall back, or the default gets written over the value"
            );
        }
    }

    // ── LIMIT clamping ─────────────────────────────────────────────────────────

    /// SQLite reads a negative `LIMIT` as "no limit at all", so an unclamped
    /// value from a command turned a bounded query into a full-table dump.
    #[test]
    fn clamp_limit_keeps_every_value_inside_a_range_sqlite_honours() {
        assert_eq!(clamp_limit(-1), 1, "LIMIT -1 means UNLIMITED in SQLite");
        assert_eq!(clamp_limit(i64::MIN), 1);
        assert_eq!(clamp_limit(0), 1, "LIMIT 0 returns nothing at all");
        assert_eq!(clamp_limit(1), 1);
        assert_eq!(clamp_limit(50), 50);
        assert_eq!(clamp_limit(MAX_QUERY_LIMIT), MAX_QUERY_LIMIT);
        assert_eq!(clamp_limit(i64::MAX), MAX_QUERY_LIMIT);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn negative_limits_are_clamped_at_the_storage_layer() {
        let storage = create_test_storage();
        for i in 0..5 {
            storage
                .save_clipboard(&format!("c{i}"), "text/plain", "d", i)
                .await
                .unwrap();
            let p = NotificationParams {
                id: &format!("n{i}"),
                device_id: "d",
                app: "app",
                title: "t",
                body: "b",
                timestamp: i,
                actions: None,
            };
            storage.save_notification(&p).await.unwrap();
            let f = FileTransferParams {
                id: &format!("f{i}"),
                name: "f",
                size: 0,
                mime: "text/plain",
                from_device: "d",
                to_device: "d",
                status: "pending",
                chunks_received: 0,
                total_chunks: 0,
                saved_path: None,
                timestamp: i,
            };
            storage.save_file_transfer(&f).await.unwrap();
            storage
                .log_automation_execution("r", "t", i, true, None)
                .await
                .unwrap();
        }

        for limit in [-1, 0, i64::MIN] {
            assert_eq!(
                storage.get_clipboard_history(limit).await.unwrap().len(),
                1,
                "clipboard limit {limit} must clamp, not dump the table"
            );
            assert_eq!(
                storage.get_notifications(limit).await.unwrap().len(),
                1,
                "notification limit {limit} must clamp, not dump the table"
            );
            assert_eq!(
                storage.get_file_transfers(limit).await.unwrap().len(),
                1,
                "file transfer limit {limit} must clamp, not dump the table"
            );
            assert_eq!(
                storage.get_automation_logs(limit).await.unwrap().len(),
                1,
                "automation log limit {limit} must clamp, not dump the table"
            );
        }

        assert_eq!(
            storage.get_clipboard_history(i64::MAX).await.unwrap().len(),
            5,
            "an oversized limit is capped, not treated as unlimited"
        );
    }

    // ── Migration system (schema_version / run_migrations) ────────────────────

    #[test]
    fn run_migrations_fresh_db_bootstraps_and_is_idempotent() {
        let mut db = Connection::open_in_memory().expect("open");
        db.execute_batch("PRAGMA journal_mode=WAL;").expect("wal");

        run_migrations(&mut db).expect("fresh bootstrap must succeed");

        let version: i64 = db
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| {
                r.get(0)
            })
            .expect("schema_version row");
        assert_eq!(version, 1, "fresh DB pins the embedded baseline");

        let tables: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='devices'",
                [],
                |r| r.get(0),
            )
            .expect("devices lookup");
        assert_eq!(tables, 1, "devices table must exist after bootstrap");

        let uv: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("user_version");
        assert_eq!(uv, 1, "PRAGMA user_version must be synced");

        // Second call must be a no-op, not an error.
        run_migrations(&mut db).expect("idempotent re-run");
    }

    #[test]
    fn run_migrations_legacy_db_without_schema_version_pins_baseline() {
        let mut db = Connection::open_in_memory().expect("open");
        // Legacy state: schema created by the old rusqlite_migration-only flow.
        let legacy = Migrations::new(vec![M::up(include_str!("migrations/001_initial.sql"))]);
        legacy.to_latest(&mut db).expect("legacy migrations");

        let has_table: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='schema_version'",
                [],
                |r| r.get(0),
            )
            .expect("schema_version lookup");
        assert_eq!(
            has_table, 0,
            "precondition: legacy DB has no schema_version"
        );

        run_migrations(&mut db).expect("legacy upgrade must succeed");

        let version: i64 = db
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| {
                r.get(0)
            })
            .expect("schema_version row");
        assert_eq!(version, 1, "legacy DB records the baseline");

        run_migrations(&mut db).expect("idempotent re-run");
    }

    #[test]
    fn apply_pending_migrations_applies_only_newer_scripts_in_order() {
        let mut db = Connection::open_in_memory().expect("open");
        ensure_schema_version_table(&db).expect("ensure table");
        record_schema_version(&db, 1).expect("record baseline");

        let scripts = vec![
            (
                1i64,
                "001_skipped.sql".to_string(),
                "CREATE TABLE IF NOT EXISTS skipped_t (x INTEGER)".to_string(),
            ),
            (
                2i64,
                "002_applied.sql".to_string(),
                "CREATE TABLE IF NOT EXISTS applied_t (y INTEGER)".to_string(),
            ),
        ];
        apply_pending_migrations(&mut db, &scripts).expect("apply pending");

        let applied: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='applied_t'",
                [],
                |r| r.get(0),
            )
            .expect("applied_t lookup");
        assert_eq!(applied, 1, "only the version-2 script must be applied");

        let skipped: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='skipped_t'",
                [],
                |r| r.get(0),
            )
            .expect("skipped_t lookup");
        assert_eq!(
            skipped, 0,
            "version-1 must be skipped when already recorded"
        );

        let version: i64 = db
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| {
                r.get(0)
            })
            .expect("schema_version row");
        assert_eq!(
            version, 2,
            "recorded version advances to the applied script"
        );

        // Idempotent re-run applies nothing and does not error.
        apply_pending_migrations(&mut db, &scripts).expect("re-run must be a no-op");
    }

    #[test]
    fn apply_pending_migrations_failed_script_rolls_back() {
        let mut db = Connection::open_in_memory().expect("open");
        ensure_schema_version_table(&db).expect("ensure table");
        record_schema_version(&db, 0).expect("baseline 0");

        let scripts = vec![(
            1i64,
            "001_broken.sql".to_string(),
            "CREATE TABLE IF NOT EXISTS ok_before_fail (x INTEGER); THIS IS NOT SQL;".to_string(),
        )];
        let err = apply_pending_migrations(&mut db, &scripts).expect_err("broken script must fail");
        assert!(
            err.to_string().contains("Migration 001_broken.sql failed"),
            "unexpected error: {err}"
        );

        let ok: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='ok_before_fail'",
                [],
                |r| r.get(0),
            )
            .expect("ok_before_fail lookup");
        assert_eq!(ok, 0, "partially executed script must roll back");

        let version: i64 = db
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| {
                r.get(0)
            })
            .expect("schema_version row");
        assert_eq!(version, 0, "version must stay at baseline after rollback");
    }

    #[test]
    fn current_schema_version_corrupt_value_errors() {
        let db = Connection::open_in_memory().expect("open");
        ensure_schema_version_table(&db).expect("ensure table");
        // Recreate the row store with a TEXT version so a corrupt value can be
        // persisted (an INTEGER PRIMARY KEY would reject it at INSERT time).
        db.execute("DROP TABLE schema_version", []).expect("drop");
        db.execute_batch(
            "CREATE TABLE schema_version (
                version TEXT NOT NULL PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .expect("recreate as TEXT");
        db.execute(
            "INSERT INTO schema_version (version) VALUES ('not-a-number')",
            [],
        )
        .expect("insert corrupt version");

        let err = current_schema_version(&db).expect_err("corrupt version must error");
        assert!(
            matches!(err, ConduitError::Database(_)),
            "expected Database error, got: {err}"
        );
    }
}
