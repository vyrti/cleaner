//! Persistent macOS TUI index.
//!
//! FSEvents are deliberately treated as hints.  The database only receives a
//! new active generation after authoritative directory scans have completed.

use super::tree::{DirEntry, DirTree, ScanProgress};
use crate::config::Config;
use crate::patterns::PatternMatcher;
use crossbeam_channel::{unbounded, Receiver, Sender};
use foldhash::{HashMap, HashMapExt};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

const FORMAT_VERSION: u32 = 1;
const ROOTS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("roots-v1");
const META: TableDefinition<&[u8], &[u8]> = TableDefinition::new("root-meta-v1");
const NODES: TableDefinition<&[u8], &[u8]> = TableDefinition::new("nodes-v1");
const GLOBAL: TableDefinition<&[u8], &[u8]> = TableDefinition::new("global-v1");

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexState {
    Disabled,
    Persisting,
    CatchingUp,
    Trusted,
    Rebuilding,
    Degraded,
}

impl IndexState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Disabled => "off",
            Self::Persisting => "persisting",
            Self::CatchingUp => "syncing; browsing only",
            Self::Trusted => "ready",
            Self::Rebuilding => "rebuilding",
            Self::Degraded => "degraded",
        }
    }

    pub fn actions_enabled(self) -> bool {
        matches!(
            self,
            Self::Disabled | Self::Persisting | Self::Trusted | Self::Degraded
        )
    }
}

pub enum IndexStartup {
    Cached { tree: DirTree, last_event_id: u64 },
    Exact { reason: String },
}

pub enum IndexEvent {
    State(IndexState),
    Tree(DirTree),
    Error(String),
}

enum Command {
    Persist {
        tree: DirTree,
        event_id: u64,
        catch_up: bool,
    },
    CatchUp {
        tree: DirTree,
        event_id: u64,
    },
    Delete(PathBuf, bool),
    Shutdown,
}

#[derive(Clone, Copy)]
struct RootMeta {
    device: u64,
    active_generation: u64,
    pending_generation: u64,
    last_event_id: u64,
    config_hash: u64,
    complete: bool,
}

pub struct IndexService {
    commands: Sender<Command>,
    events: Receiver<IndexEvent>,
    handle: Option<JoinHandle<()>>,
    cancelled: Arc<AtomicBool>,
}

impl IndexService {
    pub fn open(
        root: &Path,
        config: Arc<Config>,
        rebuild: bool,
    ) -> io::Result<(Self, IndexStartup)> {
        let db_path = database_path()?;
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut db = match open_database(&db_path) {
            Ok(db) => db,
            Err(first_error) => {
                // A writer lock is expected to be transient; do not rename a database
                // which another Cleaner process may be using.
                if first_error.to_lowercase().contains("lock") {
                    return Err(io::Error::new(io::ErrorKind::WouldBlock, first_error));
                }
                let aside = db_path.with_extension(format!(
                    "corrupt-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                ));
                let _ = fs::rename(&db_path, aside);
                open_database(&db_path).map_err(io::Error::other)?
            }
        };

        if let Err(schema_error) = initialize_schema(&db) {
            drop(db);
            let aside = db_path.with_extension(format!(
                "incompatible-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            ));
            fs::rename(&db_path, aside).map_err(|error| {
                io::Error::other(format!("{schema_error}; cannot archive index: {error}"))
            })?;
            db = open_database(&db_path).map_err(io::Error::other)?;
            initialize_schema(&db).map_err(io::Error::other)?;
        }
        let root_id = ensure_root(&db, root).map_err(io::Error::other)?;
        let device = fs::metadata(root)?.dev();
        let matcher = Arc::new(PatternMatcher::new(Arc::clone(&config)));
        let hash = config_hash(&config);

        let startup = if rebuild {
            IndexStartup::Exact {
                reason: "index rebuild requested".into(),
            }
        } else {
            match load_meta(&db, root_id).and_then(|meta| {
                if meta.device != device || meta.active_generation == 0 {
                    return Err("index volume or generation is invalid".into());
                }
                let mut tree = load_tree(&db, root_id, meta.active_generation)?;
                if meta.config_hash != hash {
                    tree.reclassify(&matcher, root, config.force);
                    persist_generation(&db, root_id, device, &tree, meta.last_event_id, hash)?;
                }
                Ok((tree, meta.last_event_id))
            }) {
                Ok((tree, last_event_id)) => IndexStartup::Cached {
                    tree,
                    last_event_id,
                },
                Err(reason) => IndexStartup::Exact { reason },
            }
        };

        let (command_tx, command_rx) = unbounded();
        let (event_tx, event_rx) = unbounded();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let worker_root = root.to_path_buf();
        let handle = thread::Builder::new()
            .name("cleaner-index".into())
            .spawn(move || {
                worker_loop(
                    db,
                    root_id,
                    device,
                    worker_root,
                    matcher,
                    hash,
                    command_rx,
                    event_tx,
                    worker_cancelled,
                )
            })?;

        Ok((
            Self {
                commands: command_tx,
                events: event_rx,
                handle: Some(handle),
                cancelled,
            },
            startup,
        ))
    }

    pub fn current_event_id() -> u64 {
        unsafe { objc2_core_services::FSEventsGetCurrentEventId() }
    }

    pub fn persist_exact(&self, tree: DirTree, starting_event_id: u64) {
        let _ = self.commands.send(Command::Persist {
            tree,
            event_id: starting_event_id,
            catch_up: true,
        });
    }

    pub fn persist_refresh(&self, tree: DirTree) {
        let _ = self.commands.send(Command::Persist {
            tree,
            event_id: Self::current_event_id(),
            catch_up: false,
        });
    }

    pub fn start_catchup(&self, tree: DirTree, last_event_id: u64) {
        let _ = self.commands.send(Command::CatchUp {
            tree,
            event_id: last_event_id,
        });
    }

    pub fn record_deletion(&self, path: PathBuf, is_dir: bool) {
        let _ = self.commands.send(Command::Delete(path, is_dir));
    }

    pub fn try_event(&self) -> Option<IndexEvent> {
        self.events.try_recv().ok()
    }

    pub fn shutdown(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        let _ = self.commands.send(Command::Shutdown);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for IndexService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[allow(clippy::too_many_arguments)]
fn worker_loop(
    db: Database,
    root_id: u64,
    device: u64,
    root: PathBuf,
    matcher: Arc<PatternMatcher>,
    config_hash: u64,
    commands: Receiver<Command>,
    events: Sender<IndexEvent>,
    cancelled: Arc<AtomicBool>,
) {
    let mut current_tree: Option<DirTree> = None;
    while let Ok(command) = commands.recv() {
        match command {
            Command::Shutdown => break,
            Command::Persist {
                tree,
                event_id,
                catch_up,
            } => {
                let _ = events.send(IndexEvent::State(IndexState::Persisting));
                match persist_generation(&db, root_id, device, &tree, event_id, config_hash) {
                    Ok(()) => {
                        current_tree = Some(tree.clone());
                        if catch_up {
                            catch_up_index(
                                &db,
                                root_id,
                                device,
                                &root,
                                &matcher,
                                config_hash,
                                tree,
                                event_id,
                                &events,
                                &mut current_tree,
                                &cancelled,
                            );
                        } else {
                            let _ = events.send(IndexEvent::State(IndexState::Trusted));
                        }
                    }
                    Err(error) => {
                        let _ = events.send(IndexEvent::Error(error));
                        let _ = events.send(IndexEvent::State(IndexState::Degraded));
                    }
                }
            }
            Command::CatchUp { tree, event_id } => {
                current_tree = Some(tree.clone());
                catch_up_index(
                    &db,
                    root_id,
                    device,
                    &root,
                    &matcher,
                    config_hash,
                    tree,
                    event_id,
                    &events,
                    &mut current_tree,
                    &cancelled,
                );
            }
            Command::Delete(path, is_dir) => {
                if let Some(tree) = current_tree.as_mut() {
                    tree.delete_entry(&path, is_dir);
                    // Do not advance past unrelated external events.  App
                    // deletions are an eager cache update; FSEvents remains the
                    // authority for advancing the durable history boundary.
                    let event_id = load_meta(&db, root_id).map_or(0, |meta| meta.last_event_id);
                    if let Err(error) =
                        persist_generation(&db, root_id, device, tree, event_id, config_hash)
                    {
                        let _ = events.send(IndexEvent::Error(error));
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn catch_up_index(
    db: &Database,
    root_id: u64,
    device: u64,
    root: &Path,
    matcher: &PatternMatcher,
    config_hash: u64,
    mut tree: DirTree,
    since: u64,
    events: &Sender<IndexEvent>,
    current_tree: &mut Option<DirTree>,
    cancelled: &Arc<AtomicBool>,
) {
    let _ = events.send(IndexEvent::State(IndexState::CatchingUp));
    match fsevents::history(root, since, cancelled) {
        Ok(history) if history.invalid => {
            let _ = events.send(IndexEvent::State(IndexState::Rebuilding));
            let progress = Arc::new(ScanProgress::new());
            let rebuilt = DirTree::build_with_progress(
                root,
                matcher,
                progress,
                Arc::clone(cancelled),
                matcher.config().force,
            );
            let event_id = IndexService::current_event_id();
            if let Err(error) =
                persist_generation(db, root_id, device, &rebuilt, event_id, config_hash)
            {
                let _ = events.send(IndexEvent::Error(error));
                let _ = events.send(IndexEvent::State(IndexState::Degraded));
                return;
            }
            *current_tree = Some(rebuilt.clone());
            let _ = events.send(IndexEvent::Tree(rebuilt));
            let _ = events.send(IndexEvent::State(IndexState::Trusted));
        }
        Ok(history) => {
            if !history.dirty.is_empty() {
                tree.reconcile(&history.dirty, matcher, root, matcher.config().force);
            }
            if let Err(error) =
                persist_generation(db, root_id, device, &tree, history.highest_id, config_hash)
            {
                let _ = events.send(IndexEvent::Error(error));
                let _ = events.send(IndexEvent::State(IndexState::Degraded));
                return;
            }
            *current_tree = Some(tree.clone());
            let _ = events.send(IndexEvent::Tree(tree));
            let _ = events.send(IndexEvent::State(IndexState::Trusted));
        }
        Err(error) => {
            if cancelled.load(Ordering::Acquire) {
                return;
            }
            let _ = events.send(IndexEvent::Error(error));
            let _ = events.send(IndexEvent::State(IndexState::Rebuilding));
            let progress = Arc::new(ScanProgress::new());
            let rebuilt = DirTree::build_with_progress(
                root,
                matcher,
                progress,
                Arc::clone(cancelled),
                matcher.config().force,
            );
            let event_id = IndexService::current_event_id();
            match persist_generation(db, root_id, device, &rebuilt, event_id, config_hash) {
                Ok(()) => {
                    *current_tree = Some(rebuilt.clone());
                    let _ = events.send(IndexEvent::Tree(rebuilt));
                    let _ = events.send(IndexEvent::State(IndexState::Trusted));
                }
                Err(error) => {
                    let _ = events.send(IndexEvent::Error(error));
                    let _ = events.send(IndexEvent::State(IndexState::Degraded));
                }
            }
        }
    }
}

fn database_path() -> io::Result<PathBuf> {
    dirs::cache_dir()
        .map(|path| path.join("cleaner/index-v1.redb"))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "macOS cache directory is unavailable",
            )
        })
}

fn open_database(path: &Path) -> Result<Database, String> {
    Database::create(path).map_err(|error| error.to_string())
}

fn initialize_schema(db: &Database) -> Result<(), String> {
    let tx = db.begin_write().map_err(|e| e.to_string())?;
    {
        let mut global = tx.open_table(GLOBAL).map_err(|e| e.to_string())?;
        let existing = global
            .get(b"format".as_slice())
            .map_err(|e| e.to_string())?
            .map(|value| value.value().to_vec());
        match existing.as_deref() {
            Some(value) if value == FORMAT_VERSION.to_be_bytes() => {}
            Some(_) => return Err("incompatible index schema".into()),
            None => {
                global
                    .insert(
                        b"format".as_slice(),
                        FORMAT_VERSION.to_be_bytes().as_slice(),
                    )
                    .map_err(|e| e.to_string())?;
            }
        }
        tx.open_table(ROOTS).map_err(|e| e.to_string())?;
        tx.open_table(META).map_err(|e| e.to_string())?;
        tx.open_table(NODES).map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

fn ensure_root(db: &Database, root: &Path) -> Result<u64, String> {
    let key = root.as_os_str().as_bytes();
    let tx = db.begin_write().map_err(|e| e.to_string())?;
    let root_id;
    {
        let mut roots = tx.open_table(ROOTS).map_err(|e| e.to_string())?;
        let existing = roots
            .get(key)
            .map_err(|e| e.to_string())?
            .map(|value| value.value().to_vec());
        if let Some(value) = existing {
            root_id =
                u64::from_be_bytes(value.as_slice().try_into().map_err(|_| "invalid root id")?);
        } else {
            let next = {
                let global = tx.open_table(GLOBAL).map_err(|e| e.to_string())?;
                let value = global
                    .get(b"next-root".as_slice())
                    .map_err(|e| e.to_string())?
                    .map(|v| u64::from_be_bytes(v.value().try_into().unwrap_or([0; 8])))
                    .unwrap_or(1);
                value
            };
            root_id = next;
            roots
                .insert(key, root_id.to_be_bytes().as_slice())
                .map_err(|e| e.to_string())?;
            let mut global = tx.open_table(GLOBAL).map_err(|e| e.to_string())?;
            global
                .insert(
                    b"next-root".as_slice(),
                    next.saturating_add(1).to_be_bytes().as_slice(),
                )
                .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(root_id)
}

fn load_meta(db: &Database, root_id: u64) -> Result<RootMeta, String> {
    let tx = db.begin_read().map_err(|e| e.to_string())?;
    let table = tx.open_table(META).map_err(|e| e.to_string())?;
    let value = table
        .get(root_id.to_be_bytes().as_slice())
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "index has no completed generation".to_string())?;
    decode_meta(value.value())
}

fn persist_generation(
    db: &Database,
    root_id: u64,
    device: u64,
    tree: &DirTree,
    event_id: u64,
    hash: u64,
) -> Result<(), String> {
    let old = load_meta(db, root_id).ok();
    let generation = old.map_or(1, |meta| meta.active_generation.saturating_add(1).max(1));
    let pending = RootMeta {
        device,
        active_generation: old.map_or(0, |m| m.active_generation),
        pending_generation: generation,
        last_event_id: old.map_or(0, |m| m.last_event_id),
        config_hash: hash,
        complete: false,
    };
    write_meta(db, root_id, pending)?;

    let tx = db.begin_write().map_err(|e| e.to_string())?;
    {
        let mut nodes = tx.open_table(NODES).map_err(|e| e.to_string())?;
        for (path, entries) in tree.shared_children() {
            let key = node_key(root_id, generation, path.as_os_str().as_bytes());
            let value = encode_entries(entries);
            nodes
                .insert(key.as_slice(), value.as_slice())
                .map_err(|e| e.to_string())?;
        }
        let complete = RootMeta {
            device,
            active_generation: generation,
            pending_generation: 0,
            last_event_id: event_id,
            config_hash: hash,
            complete: true,
        };
        let mut meta = tx.open_table(META).map_err(|e| e.to_string())?;
        meta.insert(
            root_id.to_be_bytes().as_slice(),
            encode_meta(complete).as_slice(),
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    remove_old_generations(db, root_id, generation)
}

fn write_meta(db: &Database, root_id: u64, meta_value: RootMeta) -> Result<(), String> {
    let tx = db.begin_write().map_err(|e| e.to_string())?;
    {
        tx.open_table(META)
            .map_err(|e| e.to_string())?
            .insert(
                root_id.to_be_bytes().as_slice(),
                encode_meta(meta_value).as_slice(),
            )
            .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

fn remove_old_generations(db: &Database, root_id: u64, keep: u64) -> Result<(), String> {
    let tx = db.begin_write().map_err(|e| e.to_string())?;
    {
        let mut nodes = tx.open_table(NODES).map_err(|e| e.to_string())?;
        let keys: Vec<Vec<u8>> = nodes
            .iter()
            .map_err(|e| e.to_string())?
            .filter_map(|row| row.ok())
            .map(|(key, _)| key.value().to_vec())
            .filter(|key| {
                key.len() >= 16
                    && key[..8] == root_id.to_be_bytes()
                    && key[8..16] != keep.to_be_bytes()
            })
            .collect();
        for key in keys {
            nodes.remove(key.as_slice()).map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())
}

fn load_tree(db: &Database, root_id: u64, generation: u64) -> Result<DirTree, String> {
    let tx = db.begin_read().map_err(|e| e.to_string())?;
    let nodes = tx.open_table(NODES).map_err(|e| e.to_string())?;
    let prefix = node_key(root_id, generation, b"");
    let mut children = HashMap::new();
    for row in nodes.iter().map_err(|e| e.to_string())? {
        let (key, value) = row.map_err(|e| e.to_string())?;
        if !key.value().starts_with(&prefix) {
            continue;
        }
        let path = PathBuf::from(OsString::from_vec(key.value()[16..].to_vec()));
        children.insert(path, Arc::new(decode_entries(value.value())?));
    }
    if children.is_empty() {
        return Err("active index generation is empty".into());
    }
    Ok(DirTree::from_shared_children(children))
}

fn node_key(root_id: u64, generation: u64, path: &[u8]) -> Vec<u8> {
    let mut key = Vec::with_capacity(16 + path.len());
    key.extend_from_slice(&root_id.to_be_bytes());
    key.extend_from_slice(&generation.to_be_bytes());
    key.extend_from_slice(path);
    key
}

fn encode_meta(meta: RootMeta) -> Vec<u8> {
    let mut out = Vec::with_capacity(45);
    out.extend_from_slice(&FORMAT_VERSION.to_be_bytes());
    out.extend_from_slice(&meta.device.to_be_bytes());
    out.extend_from_slice(&meta.active_generation.to_be_bytes());
    out.extend_from_slice(&meta.pending_generation.to_be_bytes());
    out.extend_from_slice(&meta.last_event_id.to_be_bytes());
    out.extend_from_slice(&meta.config_hash.to_be_bytes());
    out.push(meta.complete as u8);
    out
}

fn decode_meta(bytes: &[u8]) -> Result<RootMeta, String> {
    if bytes.len() != 45 || u32::from_be_bytes(bytes[0..4].try_into().unwrap()) != FORMAT_VERSION {
        return Err("invalid root metadata".into());
    }
    Ok(RootMeta {
        device: u64::from_be_bytes(bytes[4..12].try_into().unwrap()),
        active_generation: u64::from_be_bytes(bytes[12..20].try_into().unwrap()),
        pending_generation: u64::from_be_bytes(bytes[20..28].try_into().unwrap()),
        last_event_id: u64::from_be_bytes(bytes[28..36].try_into().unwrap()),
        config_hash: u64::from_be_bytes(bytes[36..44].try_into().unwrap()),
        complete: bytes[44] == 1,
    })
}

fn encode_entries(entries: &[DirEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for entry in entries {
        let name = entry.name.as_bytes();
        out.extend_from_slice(&(name.len() as u32).to_be_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(&entry.size.to_be_bytes());
        out.push((entry.is_dir as u8) | ((entry.is_temp as u8) << 1));
    }
    out
}

fn decode_entries(mut bytes: &[u8]) -> Result<Vec<DirEntry>, String> {
    let count = take_u32(&mut bytes)? as usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let length = take_u32(&mut bytes)? as usize;
        if bytes.len() < length + 9 {
            return Err("truncated node record".into());
        }
        let name = OsString::from_vec(bytes[..length].to_vec());
        bytes = &bytes[length..];
        let size = u64::from_be_bytes(bytes[..8].try_into().unwrap());
        let flags = bytes[8];
        bytes = &bytes[9..];
        entries.push(DirEntry {
            name,
            size,
            is_dir: flags & 1 != 0,
            is_temp: flags & 2 != 0,
        });
    }
    if !bytes.is_empty() {
        return Err("invalid trailing node data".into());
    }
    Ok(entries)
}

fn take_u32(bytes: &mut &[u8]) -> Result<u32, String> {
    if bytes.len() < 4 {
        return Err("truncated index record".into());
    }
    let value = u32::from_be_bytes(bytes[..4].try_into().unwrap());
    *bytes = &bytes[4..];
    Ok(value)
}

fn config_hash(config: &Config) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in config
        .directories
        .iter()
        .chain(&config.files)
        .flat_map(|s| s.bytes().chain([0]))
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash ^= config.force as u64;
    hash ^= config.days.unwrap_or(u64::MAX);
    hash
}

mod fsevents {
    use objc2_core_foundation as cf;
    use objc2_core_services as fs;
    use std::ffi::{CStr, OsStr};
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    const INVALID: u32 = fs::kFSEventStreamEventFlagUserDropped
        | fs::kFSEventStreamEventFlagKernelDropped
        | fs::kFSEventStreamEventFlagEventIdsWrapped
        | fs::kFSEventStreamEventFlagRootChanged
        | fs::kFSEventStreamEventFlagMount
        | fs::kFSEventStreamEventFlagUnmount;

    pub struct History {
        pub dirty: Vec<PathBuf>,
        pub highest_id: u64,
        pub invalid: bool,
    }
    struct CallbackInfo {
        sender: crossbeam_channel::Sender<(Option<PathBuf>, u32, u64)>,
    }

    unsafe extern "C-unwind" fn release_context(info: *const libc::c_void) {
        unsafe {
            drop(Box::from_raw(info as *mut CallbackInfo));
        }
    }

    unsafe extern "C-unwind" fn callback(
        _stream: fs::ConstFSEventStreamRef,
        info: *mut libc::c_void,
        count: libc::size_t,
        paths: NonNull<libc::c_void>,
        flags: NonNull<fs::FSEventStreamEventFlags>,
        ids: NonNull<fs::FSEventStreamEventId>,
    ) {
        let _ = std::panic::catch_unwind(|| unsafe {
            let info = &*(info as *const CallbackInfo);
            let paths = paths.as_ptr() as *const *const libc::c_char;
            for index in 0..count {
                let flag = *flags.as_ptr().add(index);
                let id = *ids.as_ptr().add(index);
                let path = if flag & fs::kFSEventStreamEventFlagHistoryDone != 0 {
                    None
                } else {
                    let bytes = CStr::from_ptr(*paths.add(index)).to_bytes();
                    Some(PathBuf::from(OsStr::from_bytes(bytes)))
                };
                let _ = info.sender.send((path, flag, id));
            }
        });
    }

    pub fn history(
        root: &Path,
        since: u64,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<History, String> {
        let root_str = root
            .to_str()
            .ok_or_else(|| "FSEvents cannot represent the scan root".to_string())?;
        let paths = cf::CFMutableArray::<cf::CFString>::empty();
        paths.append(&cf::CFString::from_str(root_str));
        let (tx, rx) = crossbeam_channel::unbounded();
        let context_ptr = Box::into_raw(Box::new(CallbackInfo { sender: tx }));
        let context = fs::FSEventStreamContext {
            version: 0,
            info: context_ptr.cast(),
            retain: None,
            release: Some(release_context),
            copyDescription: None,
        };
        let stream = unsafe {
            fs::FSEventStreamCreate(
                cf::kCFAllocatorDefault,
                Some(callback),
                &context as *const _ as *mut _,
                paths.as_opaque(),
                since,
                0.25,
                fs::kFSEventStreamCreateFlagFileEvents
                    | fs::kFSEventStreamCreateFlagWatchRoot
                    | fs::kFSEventStreamCreateFlagNoDefer,
            )
        };
        if stream.is_null() {
            unsafe {
                drop(Box::from_raw(context_ptr));
            }
            return Err("unable to create FSEvents stream".into());
        }
        // The dispatch-queue API avoids a detached run-loop and gives shutdown
        // a simple stop/invalidate/release ordering.
        let queue = dispatch2::DispatchQueue::new(
            "com.cleaner.index.fsevents",
            dispatch2::DispatchQueueAttr::SERIAL,
        );
        unsafe {
            fs::FSEventStreamSetDispatchQueue(stream, Some(&queue));
            if !fs::FSEventStreamStart(stream) {
                fs::FSEventStreamInvalidate(stream);
                fs::FSEventStreamRelease(stream);
                return Err("unable to start FSEvents stream".into());
            }
        }

        let mut dirty = Vec::new();
        let mut highest = since;
        let mut invalid = false;
        let started = std::time::Instant::now();
        let result = loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok((path, flag, id)) => {
                    highest = highest.max(id);
                    invalid |= flag & INVALID != 0;
                    if flag & fs::kFSEventStreamEventFlagHistoryDone != 0 {
                        break Ok(());
                    }
                    if let Some(path) = path {
                        if !path.starts_with(root) {
                            invalid = true;
                        } else if flag & fs::kFSEventStreamEventFlagMustScanSubDirs != 0 {
                            dirty.push(path);
                        } else if let Some(parent) = path.parent() {
                            dirty.push(parent.to_path_buf());
                        }
                    } else {
                        invalid = true;
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    if cancelled.load(Ordering::Acquire) {
                        break Err("index shutdown".into());
                    }
                    if started.elapsed() >= Duration::from_secs(30) {
                        break Err("FSEvents history timed out".into());
                    }
                }
                Err(error) => break Err(format!("FSEvents history is unavailable: {error}")),
            }
        };
        unsafe {
            fs::FSEventStreamStop(stream);
        }
        // Ensure callbacks enqueued before Stop have drained while their
        // callback context is still alive.
        queue.exec_sync(|| {});
        unsafe {
            fs::FSEventStreamInvalidate(stream);
            fs::FSEventStreamRelease(stream);
        }
        result?;
        dirty.sort();
        dirty.dedup();
        let mut coalesced = Vec::new();
        for path in dirty {
            if !coalesced
                .iter()
                .any(|ancestor: &PathBuf| path.starts_with(ancestor))
            {
                coalesced.push(path);
            }
        }
        Ok(History {
            dirty: coalesced,
            highest_id: highest,
            invalid,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use std::os::unix::ffi::OsStringExt;

    fn tree(root: &Path) -> DirTree {
        let mut children = HashMap::new();
        children.insert(
            root.to_path_buf(),
            vec![
                DirEntry {
                    name: OsString::from_vec(b"wide".to_vec()),
                    size: 9,
                    is_dir: true,
                    is_temp: false,
                },
                DirEntry {
                    name: OsString::from_vec(vec![b'x', 0xff]),
                    size: 7,
                    is_dir: false,
                    is_temp: true,
                },
            ],
        );
        children.insert(
            root.join("wide"),
            vec![
                DirEntry {
                    name: "..".into(),
                    size: 0,
                    is_dir: true,
                    is_temp: false,
                },
                DirEntry {
                    name: "deep.pyc".into(),
                    size: 9,
                    is_dir: false,
                    is_temp: true,
                },
            ],
        );
        DirTree::from_children(children)
    }

    #[test]
    fn redb_round_trip_preserves_raw_names_sizes_and_flags() {
        let temp = TempDir::new("index-roundtrip");
        let root = temp.path();
        let db = Database::create(temp.join("index.redb")).unwrap();
        initialize_schema(&db).unwrap();
        let root_id = ensure_root(&db, root).unwrap();
        persist_generation(
            &db,
            root_id,
            fs::metadata(root).unwrap().dev(),
            &tree(root),
            42,
            7,
        )
        .unwrap();
        let loaded = load_tree(&db, root_id, 1).unwrap();
        let entries = &loaded.children[root];
        assert_eq!(entries[0].size, 9);
        assert!(entries[1].is_temp);
        assert_eq!(entries[1].name.as_bytes(), &[b'x', 0xff]);
        assert_eq!(load_meta(&db, root_id).unwrap().last_event_id, 42);
    }

    #[test]
    fn pending_generation_never_replaces_active_generation() {
        let temp = TempDir::new("index-pending");
        let root = temp.path();
        let db = Database::create(temp.join("index.redb")).unwrap();
        initialize_schema(&db).unwrap();
        let root_id = ensure_root(&db, root).unwrap();
        let device = fs::metadata(root).unwrap().dev();
        persist_generation(&db, root_id, device, &tree(root), 11, 3).unwrap();
        write_meta(
            &db,
            root_id,
            RootMeta {
                device,
                active_generation: 1,
                pending_generation: 2,
                last_event_id: 11,
                config_hash: 3,
                complete: false,
            },
        )
        .unwrap();
        let meta = load_meta(&db, root_id).unwrap();
        assert_eq!(meta.active_generation, 1);
        assert!(load_tree(&db, root_id, meta.active_generation).is_ok());
        assert!(load_tree(&db, root_id, meta.pending_generation).is_err());
    }

    #[test]
    fn configuration_change_reclassifies_loaded_records() {
        let temp = TempDir::new("index-reclassify");
        let root = temp.path();
        let mut loaded = tree(root);
        let matcher = PatternMatcher::new(Arc::new(Config {
            directories: vec!["wide".into()],
            files: vec![".log".into()],
            days: None,
            force: true,
        }));
        loaded.reclassify(&matcher, root, true);
        let entries = &loaded.children[root];
        assert!(entries[0].is_temp);
        assert!(!entries[1].is_temp);
        assert_eq!(entries[1].name.as_bytes(), &[b'x', 0xff]);
    }

    #[test]
    #[ignore = "requires native macOS FSEvents access outside the test sandbox"]
    fn native_fsevents_replays_changes_after_saved_event_id() {
        let temp = TempDir::new("index-native-fsevents");
        let root = temp.path().canonicalize().unwrap();
        let event_id = IndexService::current_event_id();
        temp.write("created-after-boundary", b"changed");
        let history =
            fsevents::history(&root, event_id, &Arc::new(AtomicBool::new(false))).unwrap();
        assert!(!history.invalid);
        assert!(history
            .dirty
            .iter()
            .any(|path| path == &root || root.starts_with(path)));
        assert!(history.highest_id >= event_id);
    }
}
