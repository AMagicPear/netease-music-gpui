use crate::models::AudioSourceInfo;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use tempfile::{NamedTempFile, TempDir};

pub(super) const DEFAULT_CAPACITY: u64 = 2 * 1024 * 1024 * 1024;
pub(super) const DEFAULT_MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
const INDEX_NAME: &str = "audio-cache-index.json";
const INDEX_TEMP_PREFIX: &str = ".audio-cache-index-";
const LOCK_NAME: &str = ".audio-cache.lock";

#[derive(Clone, Deserialize, Serialize)]
struct Entry {
    // Both this filename stem and the map key are validated SHA-256 hex strings.
    file: String,
    size: u64,
    access: u64,
}

type Index = BTreeMap<String, Entry>;

struct Allocation {
    bytes: u64,
    path: PathBuf,
}

#[derive(Default)]
struct State {
    index: Index,
    active: HashMap<String, Weak<CachedFile>>,
    pinned: HashMap<String, usize>,
    // Incomplete reservations, invalidated live files, and failed deletions.
    unindexed: HashMap<String, Allocation>,
    clock: u64,
}

/// One store owns a cache directory. Disk operations belong on a blocking thread.
pub(super) struct AudioCacheStore {
    root: PathBuf,
    capacity: u64,
    max_file_bytes: u64,
    state: Mutex<State>,
    // Drop the locked handle before TempDir removes the directory on Windows.
    _directory_lock: fs::File,
    // Leases own the store, so a temporary directory outlives every open reader.
    _temporary: Option<TempDir>,
}

pub(super) struct CachedFile {
    store: Arc<AudioCacheStore>,
    key: String,
    file: String,
    part_path: PathBuf,
    audio_path: PathBuf,
    limit: u64,
    renamed: AtomicBool,
    completed: AtomicBool,
    size: AtomicU64,
    invalidated: AtomicBool,
}

impl AudioCacheStore {
    pub fn new(root: PathBuf, capacity: u64, max_file_bytes: u64) -> Result<Arc<Self>, String> {
        Self::open(root, capacity, max_file_bytes, None)
    }

    #[cfg(test)]
    pub fn temporary() -> Result<Arc<Self>, String> {
        let temporary = tempfile::tempdir().map_err(disk_error)?;
        Self::open(
            temporary.path().to_owned(),
            DEFAULT_CAPACITY,
            DEFAULT_MAX_FILE_BYTES,
            Some(temporary),
        )
    }

    fn open(
        root: PathBuf,
        capacity: u64,
        max_file_bytes: u64,
        temporary: Option<TempDir>,
    ) -> Result<Arc<Self>, String> {
        if capacity == 0 || max_file_bytes == 0 {
            return Err("音频缓存容量和单文件上限必须大于零".into());
        }
        fs::create_dir_all(&root).map_err(disk_error)?;
        let directory_lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join(LOCK_NAME))
            .map_err(disk_error)?;
        // Lock before inspecting or cleaning files: another process may own live parts.
        directory_lock
            .try_lock()
            .map_err(|error| format!("音频缓存目录无法加锁，可能正在被其他实例使用：{error}"))?;
        let mut index: Index = match fs::read(root.join(INDEX_NAME)) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Index::new(),
            Err(error) => return Err(disk_error(error)),
        };
        let mut filenames = HashSet::new();
        if index.iter().any(|(key, entry)| {
            !is_hash(key) || !is_hash(&entry.file) || !filenames.insert(entry.file.clone())
        }) {
            index.clear();
        }
        let mut valid = Index::new();
        for (key, entry) in index {
            if entry.size == 0 || entry.size > max_file_bytes || entry.size > capacity {
                continue;
            }
            match fs::symlink_metadata(root.join(format!("{}.audio", entry.file))) {
                Ok(metadata) if metadata.is_file() && metadata.len() == entry.size => {
                    valid.insert(key, entry);
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(disk_error(error)),
            }
        }
        let referenced: HashSet<_> = valid.values().map(|entry| entry.file.as_str()).collect();
        for entry in fs::read_dir(&root).map_err(disk_error)? {
            let entry = entry.map_err(disk_error)?;
            let filename = entry.file_name();
            let Some(name) = filename.to_str() else {
                continue;
            };
            let orphan = name.strip_suffix(".part").is_some_and(is_hash)
                || name
                    .strip_suffix(".audio")
                    .is_some_and(|stem| is_hash(stem) && !referenced.contains(stem))
                || name
                    .strip_prefix(INDEX_TEMP_PREFIX)
                    .and_then(|rest| rest.strip_suffix(".tmp"))
                    .is_some_and(|rest| {
                        !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_alphanumeric())
                    });
            // Never recurse or delete files outside the cache's own namespace.
            if orphan && !entry.file_type().map_err(disk_error)?.is_dir() {
                remove_file(&entry.path()).map_err(disk_error)?;
            }
        }
        // Normalize persisted counters so even an untrusted u64::MAX cannot freeze LRU.
        let mut order: Vec<_> = valid
            .iter()
            .map(|(key, entry)| (entry.access, key.clone()))
            .collect();
        order.sort();
        for (rank, (_, key)) in order.iter().enumerate() {
            valid.get_mut(key).unwrap().access = rank as u64 + 1;
        }
        let store = Arc::new(Self {
            root,
            capacity,
            max_file_bytes,
            state: Mutex::new(State {
                index: valid,
                clock: order.len() as u64,
                ..State::default()
            }),
            _directory_lock: directory_lock,
            _temporary: temporary,
        });
        {
            let mut state = store.state.lock().unwrap();
            store.make_room(&mut state, 0)?;
            store.persist(&state.index)?;
        }
        Ok(store)
    }

    pub fn acquire(
        self: &Arc<Self>,
        song_id: u64,
        source: &AudioSourceInfo,
    ) -> Result<Arc<CachedFile>, String> {
        let serialized = serde_json::to_vec(&(
            song_id,
            source.quality.map(|quality| quality.api_level()),
            source.cache_id.as_deref().unwrap_or(&source.url),
            source.byte_len,
            source.duration.map(|duration| duration.as_micros()),
        ))
        .map_err(|error| format!("音频缓存标识生成失败：{error}"))?;
        let key = format!("{:x}", Sha256::digest(serialized));
        let mut state = self.state.lock().unwrap();
        // Do not drop an upgraded last Arc while holding the store mutex: Drop locks it.
        if let Some(lease) = state.active.get(&key).and_then(Weak::upgrade) {
            if state.index.contains_key(&key) {
                state.clock = state.clock.saturating_add(1);
                state.index.get_mut(&key).unwrap().access = state.clock;
                let result = self.persist(&state.index);
                drop(state);
                result?;
            }
            return Ok(lease);
        }
        if let Some(entry) = state.index.get(&key).cloned() {
            let audio_path = self.root.join(format!("{}.audio", entry.file));
            match fs::symlink_metadata(&audio_path) {
                Ok(metadata) if metadata.is_file() && metadata.len() == entry.size => {
                    state.clock = state.clock.saturating_add(1);
                    state.index.get_mut(&key).unwrap().access = state.clock;
                    self.persist(&state.index)?;
                    let lease = Arc::new(CachedFile {
                        store: self.clone(),
                        key: key.clone(),
                        part_path: self.root.join(format!("{}.part", entry.file)),
                        audio_path,
                        file: entry.file.clone(),
                        limit: entry.size,
                        renamed: AtomicBool::new(true),
                        completed: AtomicBool::new(true),
                        size: AtomicU64::new(entry.size),
                        invalidated: AtomicBool::new(false),
                    });
                    *state.pinned.entry(entry.file).or_default() += 1;
                    state.active.insert(key, Arc::downgrade(&lease));
                    return Ok(lease);
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(disk_error(error)),
            }
            remove_file(&audio_path).map_err(disk_error)?;
            state.index.remove(&key);
            self.persist(&state.index)?;
        }
        let limit = source
            .byte_len
            .unwrap_or(self.max_file_bytes)
            .min(self.max_file_bytes);
        if limit == 0 {
            return Err("音频资源为空".into());
        }
        self.make_room(&mut state, limit)?;
        // Reserve before create_new. The caller must enforce lease.limit() while writing.
        loop {
            let nonce = rand::random::<u128>();
            let file = format!(
                "{:x}",
                Sha256::digest(format!("{key}:{nonce:032x}").as_bytes())
            );
            let part_path = self.root.join(format!("{file}.part"));
            let audio_path = self.root.join(format!("{file}.audio"));
            if state.pinned.contains_key(&file)
                || state.unindexed.contains_key(&file)
                || audio_path.try_exists().map_err(disk_error)?
            {
                continue;
            }
            state.unindexed.insert(
                file.clone(),
                Allocation {
                    bytes: limit,
                    path: part_path.clone(),
                },
            );
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&part_path)
            {
                Ok(handle) => drop(handle),
                Err(error) => {
                    state.unindexed.remove(&file);
                    if error.kind() == io::ErrorKind::AlreadyExists {
                        continue;
                    }
                    return Err(disk_error(error));
                }
            }
            let lease = Arc::new(CachedFile {
                store: self.clone(),
                key: key.clone(),
                file: file.clone(),
                part_path,
                audio_path,
                limit,
                renamed: AtomicBool::new(false),
                completed: AtomicBool::new(false),
                size: AtomicU64::new(0),
                invalidated: AtomicBool::new(false),
            });
            *state.pinned.entry(file).or_default() += 1;
            state.active.insert(key, Arc::downgrade(&lease));
            return Ok(lease);
        }
    }

    fn persist(&self, index: &Index) -> Result<(), String> {
        let mut temporary: NamedTempFile = tempfile::Builder::new()
            .prefix(INDEX_TEMP_PREFIX)
            .suffix(".tmp")
            .tempfile_in(&self.root)
            .map_err(disk_error)?;
        serde_json::to_writer(&mut temporary, index)
            .map_err(|error| format!("音频缓存索引写入失败：{error}"))?;
        temporary.flush().map_err(disk_error)?;
        temporary.as_file().sync_all().map_err(disk_error)?;
        temporary
            .persist(self.root.join(INDEX_NAME))
            .map_err(|error| disk_error(error.error))?;
        self.sync_directory()
    }

    fn sync_directory(&self) -> Result<(), String> {
        // Windows does not support opening a directory with std::fs::File::open.
        #[cfg(unix)]
        fs::File::open(&self.root)
            .and_then(|directory| directory.sync_all())
            .map_err(disk_error)?;
        Ok(())
    }

    fn make_room(&self, state: &mut State, needed: u64) -> Result<(), String> {
        if needed > self.capacity {
            return Err("音频缓存预留空间超过总容量".into());
        }
        // A failed Windows deletion remains charged until it can actually be removed.
        state.unindexed.retain(|file, allocation| {
            state.pinned.contains_key(file) || remove_file(&allocation.path).is_err()
        });
        let used = |state: &State| -> u128 {
            state
                .index
                .values()
                .map(|entry| u128::from(entry.size))
                .sum::<u128>()
                + state
                    .unindexed
                    .values()
                    .map(|entry| u128::from(entry.bytes))
                    .sum::<u128>()
        };
        let mut changed = false;
        while used(state) + u128::from(needed) > u128::from(self.capacity) {
            let victim = state
                .index
                .iter()
                .filter(|(_, entry)| !state.pinned.contains_key(&entry.file))
                .min_by_key(|(_, entry)| entry.access)
                .map(|(key, entry)| (key.clone(), entry.file.clone()));
            let Some((key, file)) = victim else {
                if changed {
                    self.persist(&state.index)?;
                }
                return Err("音频缓存已满，正在使用或下载的文件无法清理".into());
            };
            if let Err(error) = remove_file(&self.root.join(format!("{file}.audio"))) {
                if changed {
                    self.persist(&state.index)?;
                }
                return Err(disk_error(error));
            }
            state.index.remove(&key);
            changed = true;
        }
        if changed {
            self.persist(&state.index)?;
        }
        Ok(())
    }
}

impl CachedFile {
    /// 与 commit 共用目录状态锁，避免取得 .part 路径后正好被重命名。
    pub fn open_reader(&self) -> Result<(fs::File, Option<u64>), String> {
        let _state = self.store.state.lock().unwrap();
        if self.invalidated.load(Ordering::Acquire) {
            return Err("音频缓存已失效，请重新加载".into());
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.path())
            .map_err(disk_error)?;
        Ok((file, self.size()))
    }

    /// The path switches to .audio after rename; previously borrowed paths stay valid
    /// references, but callers should open the file before commit or fetch path again.
    pub fn path(&self) -> &Path {
        if self.renamed.load(Ordering::Acquire) {
            &self.audio_path
        } else {
            &self.part_path
        }
    }

    pub fn complete(&self) -> bool {
        self.completed.load(Ordering::Acquire) && !self.invalidated.load(Ordering::Acquire)
    }

    pub fn size(&self) -> Option<u64> {
        self.complete().then(|| self.size.load(Ordering::Relaxed))
    }

    pub fn limit(&self) -> u64 {
        self.limit
    }

    pub fn commit(&self, size: u64) -> Result<(), String> {
        let mut state = self.store.state.lock().unwrap();
        if self.invalidated.load(Ordering::Acquire) {
            return Err("音频缓存租约已失效".into());
        }
        if self.completed.load(Ordering::Acquire) {
            return if self.size.load(Ordering::Relaxed) == size {
                Ok(())
            } else {
                Err("音频缓存提交长度与已完成文件不一致".into())
            };
        }
        if size == 0 || size > self.limit || size > self.store.max_file_bytes {
            return Err("音频缓存提交长度超过预留空间或文件为空".into());
        }
        let handle = OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.path())
            .map_err(disk_error)?;
        if handle.metadata().map_err(disk_error)?.len() != size {
            return Err("音频缓存提交长度与实际文件长度不一致".into());
        }
        handle.sync_all().map_err(disk_error)?;
        drop(handle);
        if !self.renamed.load(Ordering::Acquire) {
            fs::rename(&self.part_path, &self.audio_path).map_err(disk_error)?;
            state.unindexed.get_mut(&self.file).unwrap().path = self.audio_path.clone();
            self.renamed.store(true, Ordering::Release);
        }
        state.clock = state.clock.saturating_add(1);
        let mut index = state.index.clone();
        index.insert(
            self.key.clone(),
            Entry {
                file: self.file.clone(),
                size,
                access: state.clock,
            },
        );
        // A failed index write keeps a reserved, unindexed lease. Commit can retry;
        // Drop deletes its renamed file, and startup also recognizes it as an orphan.
        self.store.persist(&index)?;
        state.index = index;
        state.unindexed.remove(&self.file);
        self.size.store(size, Ordering::Relaxed);
        self.completed.store(true, Ordering::Release);
        Ok(())
    }

    pub fn invalidate(&self) {
        let mut state = self.store.state.lock().unwrap();
        if self.invalidated.swap(true, Ordering::AcqRel) {
            return;
        }
        if state
            .active
            .get(&self.key)
            .is_some_and(|lease| std::ptr::eq(lease.as_ptr(), self))
        {
            state.active.remove(&self.key);
        }
        if state
            .index
            .get(&self.key)
            .is_some_and(|entry| entry.file == self.file)
        {
            let entry = state.index.remove(&self.key).unwrap();
            state.unindexed.insert(
                self.file.clone(),
                Allocation {
                    bytes: entry.size,
                    path: self.audio_path.clone(),
                },
            );
            // The void API cannot report I/O failure. Removing the index is the safe
            // fallback: a restart resets only cache data instead of reusing bad audio.
            if self.store.persist(&state.index).is_err() {
                let _ = remove_file(&self.store.root.join(INDEX_NAME));
                let _ = self.store.sync_directory();
            }
        }
    }
}

impl Drop for CachedFile {
    fn drop(&mut self) {
        let mut state = self.store.state.lock().unwrap();
        let pins = state.pinned.get_mut(&self.file).unwrap();
        *pins -= 1;
        if *pins == 0 {
            state.pinned.remove(&self.file);
        }
        if state
            .active
            .get(&self.key)
            .is_some_and(|lease| std::ptr::eq(lease.as_ptr(), self))
        {
            state.active.remove(&self.key);
        }
        if !state.pinned.contains_key(&self.file)
            && state.unindexed.contains_key(&self.file)
            && remove_file(self.path()).is_ok()
        {
            state.unindexed.remove(&self.file);
        }
    }
}

fn is_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn remove_file(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn disk_error(error: io::Error) -> String {
    format!("音频缓存磁盘操作失败：{error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AudioQualityLevel;
    use std::{io::Read, time::Duration};

    fn source(bytes: Option<u64>) -> AudioSourceInfo {
        AudioSourceInfo {
            url: "https://audio.example/song?signature=original".into(),
            cache_id: Some("platform-md5".into()),
            byte_len: bytes,
            duration: Some(Duration::from_secs(180)),
            quality: Some(AudioQualityLevel::Standard),
        }
    }

    fn finish(lease: &CachedFile, bytes: &[u8]) {
        // Keep an actual reader open across rename, just like playback does.
        let mut reader = fs::File::open(lease.path()).unwrap();
        fs::write(lease.path(), bytes).unwrap();
        lease.commit(bytes.len() as u64).unwrap();
        let mut actual = Vec::new();
        reader.read_to_end(&mut actual).unwrap();
        assert_eq!(actual, bytes);
        assert!(lease.complete());
        assert_eq!(lease.size(), Some(bytes.len() as u64));
        assert_eq!(lease.path().extension().unwrap(), "audio");
    }

    #[test]
    fn durable_hit_with_renewed_url_and_private_index() {
        let directory = tempfile::tempdir().unwrap();
        let store = AudioCacheStore::new(directory.path().into(), 32, 16).unwrap();
        let mut source = source(Some(4));
        let lease = store.acquire(42, &source).unwrap();
        let same = store.acquire(42, &source).unwrap();
        assert!(Arc::ptr_eq(&lease, &same));
        finish(&lease, b"song");
        let path = lease.path().to_owned();
        drop(same);
        drop(lease);
        drop(store);

        source.url = "https://audio.example/song?signature=renewed".into();
        let store = AudioCacheStore::new(directory.path().into(), 32, 16).unwrap();
        let hit = store.acquire(42, &source).unwrap();
        assert!(hit.complete());
        assert_eq!(hit.path(), path);
        assert_eq!(fs::read(hit.path()).unwrap(), b"song");
        let index = fs::read_to_string(directory.path().join(INDEX_NAME)).unwrap();
        assert!(!index.contains("https://"));
        assert!(!index.contains("platform-md5"));
        assert!(!index.contains("signature"));
    }

    #[test]
    fn quality_preview_length_song_and_unsigned_url_are_distinct() {
        let directory = tempfile::tempdir().unwrap();
        let store = AudioCacheStore::new(directory.path().into(), 64, 8).unwrap();
        let original = source(Some(4));
        let full = store.acquire(1, &original).unwrap();
        finish(&full, b"full");
        let mut different = source(Some(4));
        different.quality = Some(AudioQualityLevel::Lossless);
        assert!(!store.acquire(1, &different).unwrap().complete());
        different = source(Some(4));
        different.duration = Some(Duration::from_micros(30_000_001));
        assert!(!store.acquire(1, &different).unwrap().complete());
        different = source(Some(3));
        assert!(!store.acquire(1, &different).unwrap().complete());
        assert!(!store.acquire(2, &original).unwrap().complete());
        different = source(Some(4));
        different.cache_id = None;
        let url_only = store.acquire(1, &different).unwrap();
        finish(&url_only, b"url!");
        different.url.push_str("&renewed=true");
        assert!(!store.acquire(1, &different).unwrap().complete());
    }

    #[test]
    fn incomplete_drop_cleans_without_indexing_and_temporary_outlives_store() {
        let store = AudioCacheStore::temporary().unwrap();
        let root = store.root.clone();
        let lease = store.acquire(1, &source(None)).unwrap();
        let path = lease.path().to_owned();
        fs::write(&path, b"partial").unwrap();
        assert!(!lease.complete());
        assert_eq!(lease.size(), None);
        let index: Index =
            serde_json::from_slice(&fs::read(root.join(INDEX_NAME)).unwrap()).unwrap();
        assert!(index.is_empty());
        let same = store.acquire(1, &source(None)).unwrap();
        drop(lease);
        assert!(path.exists());
        drop(same);
        assert!(!path.exists());
        let lease = store.acquire(2, &source(Some(4))).unwrap();
        drop(store);
        assert!(root.exists());
        finish(&lease, b"song");
        drop(lease);
        assert!(!root.exists());
    }

    #[test]
    fn startup_reconciles_missing_wrong_sized_and_orphan_files() {
        let directory = tempfile::tempdir().unwrap();
        let store = AudioCacheStore::new(directory.path().into(), 32, 8).unwrap();
        let valid = store.acquire(1, &source(Some(4))).unwrap();
        finish(&valid, b"good");
        let bad = store.acquire(2, &source(Some(4))).unwrap();
        finish(&bad, b"bad!");
        let missing = store.acquire(3, &source(Some(4))).unwrap();
        finish(&missing, b"gone");
        fs::write(bad.path(), b"x").unwrap();
        fs::remove_file(missing.path()).unwrap();
        let valid_path = valid.path().to_owned();
        let bad_path = bad.path().to_owned();
        drop((valid, bad, missing, store));
        let part = directory.path().join(format!("{}.part", "a".repeat(64)));
        let audio = directory.path().join(format!("{}.audio", "b".repeat(64)));
        fs::write(&part, b"partial").unwrap();
        fs::write(&audio, b"orphan").unwrap();
        let unrelated = directory.path().join("personal.audio");
        fs::write(&unrelated, b"keep").unwrap();
        let store = AudioCacheStore::new(directory.path().into(), 32, 8).unwrap();
        assert!(valid_path.exists());
        assert!(!bad_path.exists());
        assert!(!part.exists());
        assert!(!audio.exists());
        assert!(unrelated.exists());
        assert!(store.acquire(1, &source(Some(4))).unwrap().complete());
        assert!(!store.acquire(2, &source(Some(4))).unwrap().complete());
        assert!(!store.acquire(3, &source(Some(4))).unwrap().complete());
    }

    #[test]
    fn corrupt_and_path_traversal_indexes_reset_only_owned_cache_files() {
        let directory = tempfile::tempdir().unwrap();
        let outside = directory.path().join("personal.txt");
        fs::write(&outside, b"keep").unwrap();
        for bad_index in [
            "not json".to_owned(),
            format!(
                "{{\"{}\":{{\"file\":\"../personal.txt\",\"size\":4,\"access\":1}}}}",
                "a".repeat(64)
            ),
        ] {
            let orphan = directory.path().join(format!("{}.audio", "b".repeat(64)));
            fs::write(&orphan, b"cache").unwrap();
            fs::write(directory.path().join(INDEX_NAME), bad_index).unwrap();
            let store = AudioCacheStore::new(directory.path().into(), 8, 8).unwrap();
            assert!(store.state.lock().unwrap().index.is_empty());
            assert!(!orphan.exists());
            assert_eq!(fs::read(&outside).unwrap(), b"keep");
        }
    }

    #[test]
    fn lru_preserves_hits_and_pins_and_enforces_reservations() {
        let directory = tempfile::tempdir().unwrap();
        let store = AudioCacheStore::new(directory.path().into(), 12, 8).unwrap();
        let one = store.acquire(1, &source(Some(4))).unwrap();
        finish(&one, b"1111");
        let one_path = one.path().to_owned();
        drop(one);
        let two = store.acquire(2, &source(Some(4))).unwrap();
        finish(&two, b"2222");
        let two_path = two.path().to_owned();
        drop(two);
        let pinned = store.acquire(1, &source(Some(4))).unwrap();
        let download = store.acquire(3, &source(None)).unwrap();
        assert_eq!(download.limit(), 8);
        assert!(one_path.exists());
        assert!(!two_path.exists());
        assert!(store.acquire(4, &source(Some(1))).is_err());
        let same = store.acquire(3, &source(None)).unwrap();
        assert!(Arc::ptr_eq(&download, &same));
        finish(&download, b"3333");
        let four = store.acquire(4, &source(Some(4))).unwrap();
        assert_eq!(four.limit(), 4);
        assert!(store.acquire(5, &source(Some(1))).is_err());
        drop(four);
        assert!(store.acquire(5, &source(Some(4))).is_ok());
        assert!(pinned.complete());
    }

    #[test]
    fn unpinned_lru_hit_changes_eviction_order_and_startup_shrink_evicts() {
        let directory = tempfile::tempdir().unwrap();
        let store = AudioCacheStore::new(directory.path().into(), 8, 4).unwrap();
        let one = store.acquire(1, &source(Some(4))).unwrap();
        finish(&one, b"1111");
        let one_path = one.path().to_owned();
        drop(one);
        let two = store.acquire(2, &source(Some(4))).unwrap();
        finish(&two, b"2222");
        let two_path = two.path().to_owned();
        drop(two);
        drop(store.acquire(1, &source(Some(4))).unwrap());
        let three = store.acquire(3, &source(Some(4))).unwrap();
        assert!(one_path.exists());
        assert!(!two_path.exists());
        finish(&three, b"3333");
        let three_path = three.path().to_owned();
        drop((three, store));
        let store = AudioCacheStore::new(directory.path().into(), 4, 4).unwrap();
        assert!(!one_path.exists());
        assert!(three_path.exists());
        assert!(store.acquire(3, &source(Some(4))).unwrap().complete());
    }

    #[test]
    fn commit_validates_cap_length_and_is_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let store = AudioCacheStore::new(directory.path().into(), 8, 4).unwrap();
        let oversized = store.acquire(1, &source(Some(5))).unwrap();
        assert_eq!(oversized.limit(), 4);
        assert!(oversized.commit(5).is_err());
        drop(oversized);
        let lease = store.acquire(1, &source(Some(3))).unwrap();
        assert_eq!(lease.open_reader().unwrap().1, None);
        assert_eq!(lease.limit(), 3);
        fs::write(lease.path(), b"abc").unwrap();
        assert!(lease.commit(4).is_err());
        assert!(lease.commit(2).is_err());
        assert!(lease.commit(0).is_err());
        assert!(!lease.complete());
        lease.commit(3).unwrap();
        assert_eq!(lease.open_reader().unwrap().1, Some(3));
        lease.commit(3).unwrap();
        assert!(lease.commit(2).is_err());
        let small = AudioCacheStore::new(directory.path().join("small"), 2, 4).unwrap();
        assert!(small.acquire(1, &source(None)).is_err());
        assert!(small.acquire(1, &source(Some(2))).is_ok());
    }

    #[test]
    fn invalidation_removes_index_but_counts_and_keeps_live_files() {
        let directory = tempfile::tempdir().unwrap();
        let store = AudioCacheStore::new(directory.path().into(), 8, 4).unwrap();
        let lease = store.acquire(1, &source(Some(4))).unwrap();
        finish(&lease, b"old!");
        let old_path = lease.path().to_owned();
        lease.invalidate();
        lease.invalidate();
        assert!(!lease.complete());
        assert_eq!(lease.size(), None);
        assert!(lease.commit(4).is_err());
        assert!(old_path.exists());
        let index: Index =
            serde_json::from_slice(&fs::read(directory.path().join(INDEX_NAME)).unwrap()).unwrap();
        assert!(index.is_empty());
        let replacement = store.acquire(1, &source(Some(4))).unwrap();
        assert!(!Arc::ptr_eq(&lease, &replacement));
        assert!(store.acquire(2, &source(Some(1))).is_err());
        finish(&replacement, b"new!");
        drop(lease);
        assert!(!old_path.exists());
        let hit = store.acquire(1, &source(Some(4))).unwrap();
        assert!(Arc::ptr_eq(&replacement, &hit));
        assert!(store.acquire(2, &source(Some(4))).is_ok());
        drop((hit, replacement, store));
        let store = AudioCacheStore::new(directory.path().into(), 8, 4).unwrap();
        assert_eq!(
            fs::read(store.acquire(1, &source(Some(4))).unwrap().path()).unwrap(),
            b"new!"
        );
    }

    #[test]
    fn invalidating_an_incomplete_lease_cannot_index_it() {
        let directory = tempfile::tempdir().unwrap();
        let store = AudioCacheStore::new(directory.path().into(), 8, 4).unwrap();
        let lease = store.acquire(1, &source(Some(4))).unwrap();
        fs::write(lease.path(), b"data").unwrap();
        let path = lease.path().to_owned();
        lease.invalidate();
        assert!(lease.commit(4).is_err());
        let replacement = store.acquire(1, &source(Some(4))).unwrap();
        drop(lease);
        assert!(!path.exists());
        finish(&replacement, b"okay");
    }

    #[test]
    fn directory_lock_prevents_cleanup_until_store_and_leases_drop() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let store = AudioCacheStore::new(root.into(), 8, 4).unwrap();
        let lease = store.acquire(1, &source(Some(4))).unwrap();
        fs::write(lease.path(), b"live").unwrap();
        let part_path = lease.path().to_owned();
        assert!(AudioCacheStore::new(root.into(), 8, 4).is_err());
        assert!(part_path.exists());
        drop(store);
        assert!(AudioCacheStore::new(root.into(), 8, 4).is_err());
        assert!(part_path.exists());
        lease.commit(4).unwrap();
        drop(lease);
        let reopened = AudioCacheStore::new(root.into(), 8, 4).unwrap();
        assert!(root.join(LOCK_NAME).exists());
        assert!(reopened.acquire(1, &source(Some(4))).unwrap().complete());
        drop(reopened);
        assert!(AudioCacheStore::new(root.into(), 8, 4).is_ok());
    }

    #[test]
    fn overlapping_old_drop_and_new_lease_preserve_the_new_pin() {
        let directory = tempfile::tempdir().unwrap();
        let store = AudioCacheStore::new(directory.path().into(), 4, 4).unwrap();
        let old = store.acquire(1, &source(Some(4))).unwrap();
        finish(&old, b"data");
        // Model Weak::upgrade failing while the old lease's Drop waits on the mutex.
        store.state.lock().unwrap().active.remove(&old.key);
        let new = store.acquire(1, &source(Some(4))).unwrap();
        assert!(!Arc::ptr_eq(&old, &new));
        let path = new.path().to_owned();
        new.invalidate();
        drop(old);
        assert!(path.exists());
        assert!(store.acquire(2, &source(Some(1))).is_err());
        drop(new);
        assert!(!path.exists());
        assert!(store.acquire(2, &source(Some(4))).is_ok());
    }

    #[test]
    fn failed_index_commit_keeps_reservation_and_can_retry_or_drop() {
        let directory = tempfile::tempdir().unwrap();
        let store = AudioCacheStore::new(directory.path().into(), 4, 4).unwrap();
        let lease = store.acquire(1, &source(Some(4))).unwrap();
        fs::write(lease.path(), b"data").unwrap();
        let index_path = directory.path().join(INDEX_NAME);
        fs::remove_file(&index_path).unwrap();
        fs::create_dir(&index_path).unwrap();
        assert!(lease.commit(4).is_err());
        assert!(!lease.complete());
        assert_eq!(lease.path().extension().unwrap(), "audio");
        assert!(store.state.lock().unwrap().index.is_empty());
        assert!(store.acquire(2, &source(Some(1))).is_err());
        fs::remove_dir(&index_path).unwrap();
        lease.commit(4).unwrap();
        assert!(lease.complete());
        lease.invalidate();
        drop(lease);

        let unfinished = store.acquire(2, &source(Some(4))).unwrap();
        fs::write(unfinished.path(), b"data").unwrap();
        fs::remove_file(&index_path).unwrap();
        fs::create_dir(&index_path).unwrap();
        assert!(unfinished.commit(4).is_err());
        let renamed = unfinished.path().to_owned();
        drop(unfinished);
        assert!(!renamed.exists());
        assert!(store.state.lock().unwrap().unindexed.is_empty());
    }
}
