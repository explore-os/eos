//! 9P Filesystem overlay for exposing System internals
//!
//! This module implements a 9P2000.L filesystem that exposes the internal state
//! of the actor system as a virtual filesystem. This allows inspection, monitoring,
//! and modification of the system through standard filesystem operations.
//!
//! # Filesystem Structure
//!
//! The filesystem is organized as follows:
//!
//! ```text
//! /
//! ├── spawn_queue       # Pending actor spawn requests (read-only)
//! └── actors/           # Directory of all actors
//!     └── {actor_id}/   # Directory for each actor
//!         ├── mailbox   # Actor's incoming message queue (writable)
//!         ├── script    # Path to actor's script (read-only)
//!         ├── state     # Actor's current state in JSON (writable)
//!         └── paused    # Actor's paused state as boolean (writable)
//! ```
//!
//! # Usage
//!
//! Mount the filesystem using a 9P client:
//!
//! ```bash
//! # Using 9pfuse (if available)
//! 9pfuse 'unix!/tmp/eos:0' /mnt/eos
//!
//! # Read actor state
//! ls /mnt/eos/actors
//! cat /mnt/eos/actors/{id}/state
//!
//! # Modify actor state (write JSON)
//! echo '{"counter": 42}' > /mnt/eos/actors/{id}/state
//!
//! # Pause/unpause an actor
//! echo 'true' > /mnt/eos/actors/{id}/paused
//! echo 'false' > /mnt/eos/actors/{id}/paused
//!
//! # Send a message to an actor (write JSON message)
//! echo '{"from":"sender","to":"receiver","payload":{"data":"value"}}' > /mnt/eos/actors/{id}/mailbox
//! ```
//!
//! # File Format
//!
//! - Directory listings are tab-separated: `name\ttype\tsize\n`
//! - Actor state is formatted as pretty-printed JSON
//! - Message queues show detailed message information
//! - Paused state is a boolean string ("true" or "false")
//!
//! # Write Buffering
//!
//! The filesystem implements write buffering to support editors like `vi` that perform
//! multiple write operations:
//!
//! - Writes are accumulated in a per-file buffer during editing
//! - The buffer is flushed on `fsync()` (save) or `close()` (exit) operations
//! - This ensures compatibility with standard text editors
//!
//! # Writable Files
//!
//! The following files support write operations:
//!
//! - **`/actors/{id}/state`**: Write JSON to update the actor's internal state.
//!   The entire state is replaced with the written JSON object.
//!
//! - **`/actors/{id}/mailbox`**: Write a JSON message to add it to the actor's
//!   incoming message queue. Message format: `{"from":"sender_id","to":"actor_id","payload":{...}}`
//!
//! - **`/actors/{id}/paused`**: Write "true" or "false" to pause or unpause the actor.

#![allow(unused)]

use std::cell::RefCell;
use std::sync::Arc;

use async_trait::async_trait;
use rs9p::{
    Data, DirEntry, DirEntryData, FCall, GetAttrMask, QId, QIdType, Result, Stat, Time,
    error::errno::*,
    srv::{FId, Filesystem},
};

use crate::common::Message;
use stringlit::s;
use tokio::sync::RwLock;

use crate::system::System;

// Constants for dirent d_type field (matching Unix dirent.h)
/// Directory type constant (DT_DIR from dirent.h)
const DT_DIR: u8 = 4;
/// Regular file type constant (DT_REG from dirent.h)
const DT_REG: u8 = 8;

// Constants for file mode (matching Unix stat.h)
/// Directory file type bit (S_IFDIR)
const S_IFDIR: u32 = 0o040000;
/// Regular file type bit (S_IFREG)
const S_IFREG: u32 = 0o100000;

/// Default permissions for directories (rwxr-xr-x)
const DIR_MODE: u32 = 0o755;
/// Default permissions for files (rw-rw-r--)
const FILE_MODE: u32 = 0o664;

/// 9P filesystem overlay that exposes System internals
///
/// This structure wraps the actor system and implements the 9P filesystem
/// protocol to provide read and write access to system state.
///
/// Writable files include actor state, mailbox, and paused status.
/// See module-level documentation for details on write operations.
#[derive(Clone)]
pub struct FsOverlay {
    /// The actor system being exposed, wrapped in Arc<RwLock> for thread-safe access
    pub sys: Arc<RwLock<System>>,
}

impl FsOverlay {
    /// Create a new filesystem overlay for the given System
    ///
    /// # Arguments
    ///
    /// * `sys` - The actor system to expose through the filesystem
    pub fn new(sys: Arc<RwLock<System>>) -> Self {
        Self { sys }
    }
}

/// File ID auxiliary data for tracking file paths and types
///
/// Each 9P file identifier (fid) is associated with this data structure
/// which tracks the virtual filesystem path and whether it's a directory.
#[derive(Debug, Default)]
pub struct MyFId {
    /// Virtual filesystem path (e.g., "/actors/abc123/state")
    pub path: RwLock<String>,
    /// Whether this fid represents a directory
    pub is_dir: RwLock<bool>,
    /// Write buffer for accumulating writes before commit
    pub write_buffer: RwLock<Option<Vec<u8>>>,
}

impl MyFId {
    /// Create a new FId auxiliary data
    fn new(path: String, is_dir: bool) -> Self {
        let path = if path.is_empty() {
            "/".to_string()
        } else {
            path
        };
        Self {
            path: RwLock::new(path),
            is_dir: RwLock::new(is_dir),
            write_buffer: RwLock::new(None),
        }
    }

    /// Create FId data for the root directory
    fn root() -> Self {
        Self::new("/".to_string(), true)
    }
}

#[async_trait]
impl Filesystem for FsOverlay {
    type FId = MyFId;

    async fn rattach(
        &self,
        fid: &FId<Self::FId>,
        afid: Option<&FId<Self::FId>>,
        uname: &str,
        aname: &str,
        n_uname: u32,
    ) -> Result<FCall> {
        log::warn!("rattach: {fid:?} {afid:?} {uname} {aname} {n_uname}");

        // Initialize the fid with root directory information
        *fid.aux.path.write().await = "/".to_string();
        *fid.aux.is_dir.write().await = true;

        Ok(FCall::RAttach {
            qid: QId {
                typ: QIdType::DIR,
                version: 1,
                path: 1,
            },
        })
    }

    /// Get file attributes (9P2000.L getattr operation)
    ///
    /// Returns detailed file or directory attributes including permissions, ownership,
    /// size, timestamps, and link counts. This is the 9P equivalent of the Unix `stat()` system call.
    ///
    /// # Implementation Details
    ///
    /// ## File Modes
    /// - Directories: `S_IFDIR | 0o755` (drwxr-xr-x)
    /// - Files: `S_IFREG | 0o444` (r--r--r--)
    ///
    /// Files are read-only since the filesystem provides inspection capabilities only.
    ///
    /// ## Timestamps
    /// All timestamps (atime, mtime, ctime) are set to the current system time,
    /// reflecting that the content is dynamically generated.
    ///
    /// ## Link Counts
    /// - Root directory: 3 (., .., and actors/)
    /// - /actors directory: 2 + number of actor subdirectories
    /// - Actor directories: 2 (. and ..)
    /// - Files: 1
    ///
    /// ## Ownership
    /// All files are owned by uid/gid 1000 (typical first user on Linux systems).
    ///
    /// # Arguments
    ///
    /// * `fid` - File identifier for the file/directory
    /// * `req_mask` - Bitmask indicating which attributes the client wants (currently ignored,
    ///   all available attributes are always returned)
    ///
    /// # Returns
    ///
    /// `RGetAttr` with:
    /// - `valid`: Mask of which stat fields are valid
    /// - `qid`: Unique file identifier
    /// - `stat`: Structure containing all file attributes
    ///
    /// # Errors
    ///
    /// Returns `ENOENT` if the path does not exist in the virtual filesystem.
    async fn rgetattr(&self, fid: &FId<Self::FId>, req_mask: GetAttrMask) -> Result<FCall> {
        let path = fid.aux.path.read().await;
        let path_str = if path.is_empty() { "/" } else { path.as_str() };

        let sys = self.sys.read().await;

        // Determine if this is a valid path and get its attributes
        let (exists, is_directory, size) = self.get_path_info(&sys, path_str).await?;

        if !exists {
            return Err(rs9p::Error::No(ENOENT));
        }

        // Get current time for timestamps
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let time = Time {
            sec: now.as_secs(),
            nsec: now.subsec_nanos() as u64,
        };

        // Determine QID type and mode
        let (qid_type, mode_type, base_perms) = if is_directory {
            (QIdType::DIR, S_IFDIR, DIR_MODE)
        } else {
            (QIdType::FILE, S_IFREG, FILE_MODE)
        };

        let mode = mode_type | base_perms;

        // Calculate number of links
        // Directories have at least 2 links (. and parent's entry)
        // Files have 1 link
        let nlink = if is_directory {
            // Count subdirectories to calculate proper link count
            let subdir_count = if path_str == "/" {
                // Root has "actors" subdirectory
                1u64
            } else if path_str == "/actors" {
                // Count actor directories
                sys.actors.len() as u64
            } else {
                // Actor subdirectories have no subdirectories
                0u64
            };
            2 + subdir_count
        } else {
            1
        };

        // Build the stat structure
        let stat = Stat {
            mode,
            uid: 1000,
            gid: 1000,
            nlink,
            rdev: 0,
            size,
            blksize: 4096,
            blocks: (size + 511) / 512, // 512-byte blocks as per stat(2)
            atime: time,
            mtime: time,
            ctime: time,
        };

        // Determine which fields are valid based on what we can provide
        let valid = GetAttrMask::MODE
            | GetAttrMask::NLINK
            | GetAttrMask::UID
            | GetAttrMask::GID
            | GetAttrMask::RDEV
            | GetAttrMask::ATIME
            | GetAttrMask::MTIME
            | GetAttrMask::CTIME
            | GetAttrMask::SIZE
            | GetAttrMask::BLOCKS;

        Ok(FCall::RGetAttr {
            valid,
            qid: QId {
                typ: qid_type,
                version: 1,
                path: self.path_to_qid(path_str),
            },
            stat,
        })
    }

    async fn rclunk(&self, fid: &FId<Self::FId>) -> Result<FCall> {
        // Flush any buffered writes before closing the file
        let mut write_buffer = fid.aux.write_buffer.write().await;
        if let Some(buffer) = write_buffer.take() {
            let path = fid.aux.path.read().await.clone();
            log::debug!("rclunk: flushing {} bytes for path {}", buffer.len(), path);

            // Get write lock on system and write the buffered data
            let mut sys = self.sys.write().await;
            let _ = self.write_file(&mut sys, &path, &buffer).await;
        }

        Ok(FCall::RClunk)
    }

    async fn rwalk(
        &self,
        fid: &FId<Self::FId>,
        newfid: &FId<Self::FId>,
        wnames: &[String],
    ) -> Result<FCall> {
        let current_path = fid.aux.path.read().await.clone();
        let current_path = if current_path == "" {
            s!("/")
        } else {
            current_path
        };
        let current_is_dir = *fid.aux.is_dir.read().await;
        let sys = self.sys.read().await;

        log::debug!("rwalk: current_path={}, wnames={:?}", current_path, wnames);

        // If no wnames, this is a clone operation
        if wnames.is_empty() {
            // Clone the fid to newfid
            *newfid.aux.path.write().await = current_path.clone();
            *newfid.aux.is_dir.write().await = current_is_dir;
            log::debug!("rwalk: clone operation, newfid.path={}", current_path);
            return Ok(FCall::RWalk { wqids: vec![] });
        }

        let mut wqids = Vec::new();
        let mut path = current_path.clone();
        let mut final_is_dir = current_is_dir;

        for wname in wnames {
            if wname == ".." {
                // Go up one directory
                if path != "/" {
                    path = path.rsplitn(2, '/').nth(1).unwrap_or("/").to_string();
                    if path.is_empty() {
                        path = "/".to_string();
                    }
                }
            } else {
                // Go down into a directory
                if path == "/" {
                    path = format!("/{}", wname);
                } else {
                    path = format!("{}/{}", path, wname);
                }
            }

            let (exists, is_dir, _) = self.get_path_info(&sys, &path).await?;
            if !exists {
                break;
            }

            final_is_dir = is_dir;
            let qid_type = if is_dir { QIdType::DIR } else { QIdType::FILE };

            wqids.push(QId {
                typ: qid_type,
                version: 1,
                path: self.path_to_qid(&path),
            });
        }

        // Update newfid with the final path after successful walk
        if !wqids.is_empty() {
            *newfid.aux.path.write().await = path.clone();
            *newfid.aux.is_dir.write().await = final_is_dir;
            log::debug!(
                "rwalk: success, newfid.path={}, is_dir={}, qids={}",
                path,
                final_is_dir,
                wqids.len()
            );
        } else {
            // If walk failed, newfid stays at the original position
            *newfid.aux.path.write().await = current_path.clone();
            *newfid.aux.is_dir.write().await = current_is_dir;
            log::debug!("rwalk: partial walk, newfid.path={}", current_path);
        }

        Ok(FCall::RWalk { wqids })
    }

    async fn rread(&self, fid: &FId<Self::FId>, offset: u64, count: u32) -> Result<FCall> {
        let path = fid.aux.path.read().await.clone();
        let mut is_dir = *fid.aux.is_dir.read().await;
        let path = if path.is_empty() {
            is_dir = true;
            s!("/")
        } else {
            path
        };

        log::debug!(
            "rread: path={}, is_dir={}, offset={}, count={}",
            path,
            is_dir,
            offset,
            count
        );

        let sys = self.sys.read().await;

        if is_dir {
            // Read directory entries
            let entries = self.read_directory(&sys, &path).await?;
            let data = self.encode_directory_entries(&entries, offset, count);
            Ok(FCall::RRead { data: Data(data) })
        } else {
            // Read file contents
            let content = self.read_file(&sys, &path).await?;
            let start = offset as usize;
            let end = std::cmp::min(start + count as usize, content.len());
            let data = if start < content.len() {
                content[start..end].to_vec()
            } else {
                vec![]
            };
            Ok(FCall::RRead { data: Data(data) })
        }
    }

    async fn rwrite(&self, fid: &FId<Self::FId>, offset: u64, data: &Data) -> Result<FCall> {
        let path = fid.aux.path.read().await.clone();
        let is_dir = *fid.aux.is_dir.read().await;

        log::debug!(
            "rwrite: path={}, is_dir={}, offset={}, data_len={}",
            path,
            is_dir,
            offset,
            data.0.len()
        );

        // Cannot write to directories
        if is_dir {
            return Err(rs9p::Error::No(EISDIR));
        }

        // Get or create write buffer
        let mut write_buffer = fid.aux.write_buffer.write().await;
        let buffer = write_buffer.get_or_insert_with(Vec::new);

        // Handle offset-based writes
        if offset as usize > buffer.len() {
            // Extend buffer with zeros if offset is beyond current size
            buffer.resize(offset as usize, 0);
        }

        // Write data at offset
        if offset as usize == buffer.len() {
            // Append to end
            buffer.extend_from_slice(&data.0);
        } else {
            // Overwrite at offset
            let end = (offset as usize + data.0.len()).min(buffer.len());
            if offset as usize + data.0.len() > buffer.len() {
                // Need to extend
                buffer.resize(offset as usize + data.0.len(), 0);
            }
            buffer[offset as usize..offset as usize + data.0.len()].copy_from_slice(&data.0);
        }

        let count = data.0.len() as u32;
        log::debug!(
            "rwrite: buffered {} bytes, total buffer size: {}",
            count,
            buffer.len()
        );

        Ok(FCall::RWrite { count })
    }

    async fn rlopen(&self, fid: &FId<Self::FId>, _flags: u32) -> Result<FCall> {
        let qid_type = if *fid.aux.is_dir.read().await {
            QIdType::DIR
        } else {
            QIdType::FILE
        };

        let path = fid.aux.path.read().await.clone();
        Ok(FCall::RlOpen {
            qid: QId {
                typ: qid_type,
                version: 1,
                path: self.path_to_qid(&path),
            },
            iounit: 8192,
        })
    }

    async fn rfsync(&self, fid: &FId<Self::FId>) -> Result<FCall> {
        // Flush any buffered writes to the actual system
        log::debug!("rfsync: called");

        let mut write_buffer = fid.aux.write_buffer.write().await;
        if let Some(buffer) = write_buffer.take() {
            let path = fid.aux.path.read().await.clone();
            log::debug!("rfsync: flushing {} bytes for path {}", buffer.len(), path);

            // Get write lock on system and write the buffered data
            let mut sys = self.sys.write().await;
            self.write_file(&mut sys, &path, &buffer).await?;
        }

        Ok(FCall::RFSync)
    }

    async fn rsetattr(
        &self,
        _fid: &FId<Self::FId>,
        _valid: rs9p::SetAttrMask,
        _stat: &rs9p::SetAttr,
    ) -> Result<FCall> {
        // For our virtual filesystem, we don't allow changing file attributes
        // like timestamps, permissions, etc. through setattr
        // This is mainly used by editors to preserve timestamps
        log::debug!("rsetattr: called (no-op)");
        Ok(FCall::RSetAttr)
    }

    async fn rreaddir(&self, fid: &FId<Self::FId>, offset: u64, count: u32) -> Result<FCall> {
        let path = fid.aux.path.read().await.clone();
        let mut is_dir = *fid.aux.is_dir.read().await;
        let path_str = if path.is_empty() {
            is_dir = true;
            "/"
        } else {
            path.as_str()
        };

        log::debug!(
            "rreaddir: path={}, is_dir={}, offset={}, count={}",
            path_str,
            is_dir,
            offset,
            count
        );

        let sys = self.sys.read().await;

        // Verify this is actually a directory
        if !is_dir {
            return Err(rs9p::Error::No(ENOTDIR));
        }

        // Get directory entries
        let entries = self.read_directory(&sys, path_str).await?;

        // Convert to DirEntry format, respecting offset and count
        let mut dir_entries = Vec::new();
        let mut current_size = 0u32;
        let offset_idx = offset as usize;

        for (idx, (name, is_dir, _size)) in entries.iter().enumerate() {
            // Skip entries before the requested offset
            if idx < offset_idx {
                continue;
            }

            let entry_path = if path_str == "/" {
                format!("/{}", name)
            } else {
                format!("{}/{}", path_str, name)
            };

            // Determine the d_type value using Unix dirent constants
            let typ = if *is_dir { DT_DIR } else { DT_REG };

            let qid = QId {
                typ: if *is_dir { QIdType::DIR } else { QIdType::FILE },
                version: 1,
                path: self.path_to_qid(&entry_path),
            };

            let entry = DirEntry {
                qid,
                offset: (idx + 1) as u64, // offset is the index of the *next* entry
                typ,
                name: name.clone(),
            };

            // Check if adding this entry would exceed the count limit
            let entry_size = entry.size();
            if current_size + entry_size > count {
                // If we haven't added any entries yet, we need to return at least one
                if dir_entries.is_empty() && count > 0 {
                    dir_entries.push(entry);
                }
                break;
            }

            current_size += entry_size;
            dir_entries.push(entry);
        }

        Ok(FCall::RReadDir {
            data: DirEntryData::with(dir_entries),
        })
    }
}

impl FsOverlay {
    /// Get file attributes for a path
    ///
    /// This is a helper for the `rgetattr` implementation that validates paths
    /// and retrieves file metadata. The implementation properly handles:
    ///
    /// - Empty paths (treated as root "/")
    /// - Non-existent paths (returns error)
    /// - Dynamic content size calculation
    /// - Proper file type determination
    /// - Current timestamp generation
    /// - Correct link count calculation
    ///
    /// # File Type Modes
    ///
    /// The mode field combines the file type bits with permission bits:
    ///
    /// - **Directories**: `S_IFDIR (0o040000) | 0o755`
    ///   - Binary: `0b0100_000_111_101_101`
    ///   - Decimal: 16877
    ///   - Permissions: `drwxr-xr-x`
    ///
    /// - **Regular Files**: `S_IFREG (0o100000) | 0o444`
    ///   - Binary: `0b1000_000_100_100_100`
    ///   - Decimal: 33060
    ///   - Permissions: `-r--r--r--`
    ///
    /// Read-only permissions for files reflect that this is an inspection interface.
    ///
    /// # Block Calculation
    ///
    /// The `blocks` field reports the number of 512-byte blocks, following `stat(2)`:
    /// ```text
    /// blocks = (size + 511) / 512
    /// ```
    ///
    /// This differs from the `blksize` field which is the preferred I/O block size (4096).

    /// Read directory entries using the `readdir` operation (9P2000.L)
    ///
    /// This is the preferred method for reading directories in 9P2000.L,
    /// providing structured directory entries with proper QIDs and type information.
    ///
    /// The implementation respects the `offset` parameter for pagination and the
    /// `count` parameter to limit response size, ensuring efficient directory listing
    /// for directories with many entries.
    ///
    /// # Arguments
    ///
    /// * `fid` - File ID for the directory to read
    /// * `offset` - Entry index to start from (0-based)
    /// * `count` - Maximum number of bytes to return
    ///
    /// # Returns
    ///
    /// `RReadDir` with a `DirEntryData` containing directory entries that fit within
    /// the count limit. Each entry includes:
    /// - `qid`: Unique file identifier
    /// - `offset`: Index of the next entry (for pagination)
    /// - `typ`: File type (DT_DIR for directories, DT_REG for regular files)
    /// - `name`: Entry name
    ///
    /// # Errors
    ///
    /// Returns `ENOTDIR` if the fid does not refer to a directory.

    /// Convert a filesystem path to a QID path value
    ///
    /// QIDs uniquely identify files in 9P. We use a hash of the path string.
    fn path_to_qid(&self, path: &str) -> u64 {
        let path = if path.is_empty() { "/" } else { path };
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        hasher.finish()
    }

    /// Get information about a path in the virtual filesystem
    ///
    /// Returns (exists, is_directory, size) for the given path.
    ///
    /// # Arguments
    ///
    /// * `sys` - Reference to the System
    /// * `path` - Virtual filesystem path to query
    ///
    /// # Returns
    ///
    /// Tuple of (exists, is_directory, content_size_in_bytes)
    async fn get_path_info(&self, sys: &System, path: &str) -> Result<(bool, bool, u64)> {
        match path {
            "/" | "" => Ok((true, true, 0)),
            "/actors" => Ok((true, true, 0)),
            "/spawn_queue" => {
                let content = self.format_spawn_queue(sys);
                Ok((true, false, content.len() as u64))
            }
            _ => {
                // Check if it's an actor path
                if path.starts_with("/actors/") {
                    let parts: Vec<&str> = path.trim_start_matches("/actors/").split('/').collect();
                    if parts.is_empty() || parts[0].is_empty() {
                        return Ok((false, false, 0));
                    }

                    let actor_id = parts[0];
                    if !sys.actors.contains_key(actor_id) {
                        return Ok((false, false, 0));
                    }

                    if parts.len() == 1 {
                        // /actors/{id}
                        return Ok((true, true, 0));
                    }

                    if let Some(actor) = sys.actors.get(actor_id) {
                        match parts[1] {
                            "mailbox" => {
                                let content = self.format_mailbox(actor);
                                Ok((true, false, content.len() as u64))
                            }
                            "script" => Ok((true, false, actor.script.len() as u64)),
                            "state" => {
                                let content =
                                    serde_json::to_string_pretty(&actor.state).unwrap_or_default();
                                Ok((true, false, content.len() as u64))
                            }
                            "paused" => {
                                let content = actor.paused.to_string();
                                Ok((true, false, content.len() as u64))
                            }
                            _ => Ok((false, false, 0)),
                        }
                    } else {
                        Ok((false, false, 0))
                    }
                } else {
                    Ok((false, false, 0))
                }
            }
        }
    }

    /// Read directory entries for a given path
    ///
    /// Returns a vector of tuples containing (name, is_directory, size)
    /// for each entry in the directory.
    ///
    /// # Arguments
    ///
    /// * `sys` - Reference to the System
    /// * `path` - Virtual filesystem path of the directory
    async fn read_directory(&self, sys: &System, path: &str) -> Result<Vec<(String, bool, u64)>> {
        log::debug!("read_directory: path={}", path);
        let entries = match path {
            "/" => vec![
                ("actors".to_string(), true, 0),
                (
                    "spawn_queue".to_string(),
                    false,
                    self.format_spawn_queue(sys).len() as u64,
                ),
            ],
            "/actors" => {
                let mut entries: Vec<_> = sys
                    .actors
                    .keys()
                    .map(|id| (id.clone(), true, 0u64))
                    .collect();
                entries.sort_by(|a, b| a.0.cmp(&b.0));
                entries
            }
            _ => {
                if path.starts_with("/actors/") {
                    let parts: Vec<&str> = path.trim_start_matches("/actors/").split('/').collect();
                    if parts.len() == 1 && !parts[0].is_empty() {
                        let actor_id = parts[0];
                        if let Some(actor) = sys.actors.get(actor_id) {
                            return Ok(vec![
                                (
                                    "mailbox".to_string(),
                                    false,
                                    self.format_mailbox(actor).len() as u64,
                                ),
                                ("script".to_string(), false, actor.script.len() as u64),
                                (
                                    "state".to_string(),
                                    false,
                                    serde_json::to_string_pretty(&actor.state)
                                        .unwrap_or_default()
                                        .len() as u64,
                                ),
                                (
                                    "paused".to_string(),
                                    false,
                                    actor.paused.to_string().len() as u64,
                                ),
                            ]);
                        }
                    }
                }
                vec![]
            }
        };
        log::debug!(
            "read_directory: path={}, found {} entries",
            path,
            entries.len()
        );
        Ok(entries)
    }

    /// Encode directory entries into a byte buffer for reading
    ///
    /// Formats directory entries as tab-separated values with newlines.
    /// Handles offset and count for partial reads.
    ///
    /// # Format
    ///
    /// Each line: `name\ttype\tsize\n` where type is "dir" or "file"
    ///
    /// # Arguments
    ///
    /// * `entries` - List of (name, is_dir, size) tuples
    /// * `offset` - Byte offset to start reading from
    /// * `count` - Maximum number of bytes to return
    fn encode_directory_entries(
        &self,
        entries: &[(String, bool, u64)],
        offset: u64,
        count: u32,
    ) -> Vec<u8> {
        let mut data = Vec::new();
        let mut current_offset = 0u64;

        for (name, is_dir, size) in entries {
            let entry_type = if *is_dir { "dir" } else { "file" };
            let entry = format!("{}\t{}\t{}\n", name, entry_type, size);
            let entry_bytes = entry.as_bytes();
            let entry_len = entry_bytes.len() as u64;

            if current_offset + entry_len > offset {
                let start_in_entry = if current_offset < offset {
                    (offset - current_offset) as usize
                } else {
                    0
                };

                let remaining = count as usize - data.len();
                let to_copy = std::cmp::min(remaining, entry_bytes.len() - start_in_entry);

                data.extend_from_slice(&entry_bytes[start_in_entry..start_in_entry + to_copy]);

                if data.len() >= count as usize {
                    break;
                }
            }

            current_offset += entry_len;
        }

        data
    }

    /// Read file contents for a given path
    ///
    /// Returns the complete file contents as bytes.
    ///
    /// # Arguments
    ///
    /// * `sys` - Reference to the System
    /// * `path` - Virtual filesystem path of the file
    async fn read_file(&self, sys: &System, path: &str) -> Result<Vec<u8>> {
        match path {
            "/spawn_queue" => Ok(self.format_spawn_queue(sys).into_bytes()),
            _ => {
                if path.starts_with("/actors/") {
                    let parts: Vec<&str> = path.trim_start_matches("/actors/").split('/').collect();
                    if parts.len() >= 2 {
                        let actor_id = parts[0];
                        if let Some(actor) = sys.actors.get(actor_id) {
                            match parts[1] {
                                "mailbox" => return Ok(self.format_mailbox(actor).into_bytes()),
                                "script" => {
                                    return Ok(actor.script.clone().into_bytes());
                                }
                                "state" => {
                                    return Ok(serde_json::to_string_pretty(&actor.state)
                                        .unwrap_or_default()
                                        .into_bytes());
                                }
                                "paused" => {
                                    return Ok(actor.paused.to_string().into_bytes());
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Ok(vec![])
            }
        }
    }

    /// Write data to a file in the virtual filesystem
    ///
    /// # Arguments
    ///
    /// * `sys` - Mutable reference to the System
    /// * `path` - Virtual filesystem path of the file
    /// * `data` - Data to write to the file
    async fn write_file(&self, sys: &mut System, path: &str, data: &[u8]) -> Result<u32> {
        // Parse the data as a string
        let content = std::str::from_utf8(data).map_err(|_| rs9p::Error::No(EINVAL))?;

        match path {
            _ => {
                if path.starts_with("/actors/") {
                    let parts: Vec<&str> = path.trim_start_matches("/actors/").split('/').collect();
                    if parts.len() >= 2 {
                        let actor_id = parts[0];
                        if let Some(actor) = sys.actors.get_mut(actor_id) {
                            match parts[1] {
                                "state" => {
                                    // Parse and update actor state
                                    let new_state: serde_json::Value =
                                        serde_json::from_str(content).map_err(|e| {
                                            log::error!("Failed to parse JSON state: {}", e);
                                            rs9p::Error::No(EINVAL)
                                        })?;
                                    actor.state = new_state;
                                    log::info!("Updated state for actor {}", actor_id);
                                    return Ok(data.len() as u32);
                                }
                                "mailbox" => {
                                    // Parse and add message to mailbox
                                    if let Ok(messages) = serde_json::from_str(content) {
                                        actor.mailbox = messages;
                                        log::info!("Updated mailbox of actor {}", actor_id);
                                    }
                                    return Ok(data.len() as u32);
                                }
                                "script" => {
                                    actor.script = content.to_owned();
                                    log::info!("Updated script of actor {}", actor_id);
                                    return Ok(data.len() as u32);
                                }
                                "paused" => {
                                    // Parse and update paused state
                                    let paused: bool = content.trim().parse().map_err(|e| {
                                        log::error!("Failed to parse paused state: {}", e);
                                        rs9p::Error::No(EINVAL)
                                    })?;
                                    actor.paused = paused;
                                    log::info!(
                                        "Updated paused state for actor {} to {}",
                                        actor_id,
                                        paused
                                    );
                                    return Ok(data.len() as u32);
                                }
                                _ => {}
                            }
                        } else {
                            return Err(rs9p::Error::No(ENOENT));
                        }
                    }
                }
                // File not writable or doesn't exist
                Err(rs9p::Error::No(EROFS))
            }
        }
    }

    /// Format the spawn queue as human-readable text
    ///
    /// Returns a formatted string showing all pending actor spawn requests
    /// with their script paths, IDs, and initial state.
    fn format_spawn_queue(&self, sys: &System) -> String {
        serde_json::to_string_pretty(&sys.spawn_queue).unwrap_or_else(|_| s!("[]"))
    }

    /// Format an actor's mailbox as human-readable text
    ///
    /// Returns a formatted string showing all messages in the actor's
    /// incoming message queue with sender, recipient, and payload details.
    fn format_mailbox(&self, actor: &crate::system::Actor) -> String {
        serde_json::to_string_pretty(&actor.mailbox).unwrap_or_else(|_| s!("[]"))
    }
}
