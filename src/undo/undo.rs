//! Undo system for rename operations.

use crate::core::{RenameBatch, RenameRecord, RenamerError, RenamerResult};
use crate::engine::{
    RecoveryJournal, RenamePlan, RenamePlanItem, STRANDED_MARKER, file_identity,
    is_journal_name, unique_temp_path,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Maximum number of undo batches to keep in memory.
const MAX_UNDO_HISTORY: usize = 100;

/// Sub-directory of the data directory holding undone batches (the redo stack).
const REDO_SUBDIR: &str = "redo";

/// Advisory lock file guarding the data directory against concurrent windows.
const LOCK_FILE_NAME: &str = ".lock";

/// Identity of a renamed file, captured right after the rename succeeded.
///
/// `fs::rename` neither checks that the source is still the file we renamed nor
/// that the destination is free, so undo has to prove both itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileFingerprint {
    size: u64,
    /// Modification time as (seconds, nanoseconds) since the Unix epoch.
    mtime: Option<(u64, u32)>,
    /// Inode and device, when the platform exposes them.
    inode: Option<u64>,
    dev: Option<u64>,
}

impl FileFingerprint {
    fn capture(path: &Path) -> Option<Self> {
        // symlink_metadata so a symlink is identified by itself, not its target.
        fs::symlink_metadata(path).ok().map(|meta| Self::from_metadata(&meta))
    }

    fn from_metadata(meta: &fs::Metadata) -> Self {
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| (d.as_secs(), d.subsec_nanos()));

        #[cfg(unix)]
        let (inode, dev) = {
            use std::os::unix::fs::MetadataExt;
            (Some(meta.ino()), Some(meta.dev()))
        };
        #[cfg(not(unix))]
        let (inode, dev) = (None, None);

        Self {
            size: meta.len(),
            mtime,
            inode,
            dev,
        }
    }

    /// Whether two fingerprints denote the same unchanged file.
    fn matches(&self, other: &Self) -> bool {
        // The inode is an exact identity check. Size and mtime are only a
        // fallback because a directory's size and mtime change whenever its
        // contents do, which has nothing to do with the rename we are undoing.
        if let (Some(a), Some(b)) = (self.inode, other.inode) {
            return a == b && self.dev == other.dev;
        }
        self.size == other.size && self.mtime == other.mtime
    }

    /// Whether this fingerprint refers to the very same filesystem object.
    fn is_same_object(&self, other: &Self) -> bool {
        match (self.inode, other.inode) {
            (Some(a), Some(b)) => a == b && self.dev == other.dev,
            _ => false,
        }
    }
}

/// A recorded batch plus the fingerprint of every renamed file.
///
/// Serialised with a flattened `RenameBatch`, so files written by older
/// versions (a bare `RenameBatch`) still deserialise, just without
/// fingerprints.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredBatch {
    #[serde(flatten)]
    batch: RenameBatch,
    /// Fingerprint per record id, keyed so the record order can change.
    #[serde(default)]
    fingerprints: HashMap<Uuid, FileFingerprint>,
}

impl StoredBatch {
    /// Fingerprint every record at `path_of`, which is where the file currently
    /// lives for the direction this batch will be applied in.
    fn capture(batch: RenameBatch, path_of: fn(&RenameRecord) -> &Path) -> Self {
        let fingerprints = batch
            .records
            .iter()
            .filter_map(|r| FileFingerprint::capture(path_of(r)).map(|f| (r.id, f)))
            .collect();
        Self {
            batch,
            fingerprints,
        }
    }

    /// Rebuild a stored batch for `records`, carrying fingerprints forward from
    /// an earlier capture instead of re-reading the disk. A record refused
    /// because "the file changed" must keep the fingerprint of the file it
    /// recorded — re-capturing here would adopt the impostor's fingerprint and
    /// let the next retry move the wrong file onto the original path.
    fn with_fingerprints_from(
        batch: RenameBatch,
        fingerprints: &HashMap<Uuid, FileFingerprint>,
    ) -> Self {
        let fingerprints = batch
            .records
            .iter()
            .filter_map(|r| fingerprints.get(&r.id).map(|f| (r.id, f.clone())))
            .collect();
        Self {
            batch,
            fingerprints,
        }
    }
}

/// One reversal to apply: move the file now at `source` back to `dest`.
struct PlannedMove<'a> {
    record: &'a RenameRecord,
    source: &'a Path,
    dest: &'a Path,
}

/// Apply a set of reversals using the same two-phase staging the rename engine
/// uses: every source is moved aside first, then on to its destination.
///
/// A batch may have swapped or rotated names, in which case every destination
/// is occupied by another member of the same batch and no ordering of plain
/// renames can take it back. Staging also settles every refusal before anything
/// moves, so a batch is never left half reverted by a conflict we could see.
///
/// Between the phases every accepted file sits under a hidden staging name, so the
/// plan goes to a crash-recovery journal in `journal_dir` first, exactly as a rename
/// batch does: recovery then puts each file back where this undo or redo found it.
///
/// Returns one result per entry of `moves`, in order.
fn apply_moves(
    moves: &[PlannedMove<'_>],
    fingerprints: &HashMap<Uuid, FileFingerprint>,
    journal_dir: Option<&Path>,
) -> Vec<UndoRecordResult> {
    let mut refused: HashMap<Uuid, String> = HashMap::new();
    let mut current: HashMap<Uuid, FileFingerprint> = HashMap::new();

    // `fs::rename` neither checks that the source is still the file we renamed
    // nor that the destination is free, so undo has to prove both itself.
    for planned in moves {
        let Some(now) = FileFingerprint::capture(planned.source) else {
            refused.insert(
                planned.record.id,
                format!("Source no longer exists: {}", planned.source.display()),
            );
            continue;
        };
        if fingerprints
            .get(&planned.record.id)
            .is_some_and(|expected| !expected.matches(&now))
        {
            refused.insert(
                planned.record.id,
                format!(
                    "File changed since the rename, refusing to move: {}",
                    planned.source.display()
                ),
            );
            continue;
        }
        current.insert(planned.record.id, now);
    }

    // A destination is free if nothing is there, if it is the source itself, or
    // if another accepted reversal is about to vacate it. Refusing a reversal
    // leaves its source parked where it is, which can in turn block someone
    // else's destination, so settle to a fixed point before moving anything.
    loop {
        let vacating: HashSet<&Path> = moves
            .iter()
            .filter(|planned| !refused.contains_key(&planned.record.id))
            .map(|planned| planned.source)
            .collect();

        let mut newly_refused = Vec::new();
        for planned in moves {
            if refused.contains_key(&planned.record.id) || vacating.contains(planned.dest) {
                continue;
            }
            let Ok(dest_meta) = fs::symlink_metadata(planned.dest) else {
                continue;
            };
            // A case-only rename on a case-insensitive filesystem resolves both
            // paths to the same object; that is the file itself, not a victim.
            let occupant = FileFingerprint::from_metadata(&dest_meta);
            if current
                .get(&planned.record.id)
                .is_some_and(|source| occupant.is_same_object(source))
            {
                continue;
            }
            newly_refused.push((
                planned.record.id,
                format!("Destination already exists: {}", planned.dest.display()),
            ));
        }

        if newly_refused.is_empty() {
            break;
        }
        refused.extend(newly_refused);
    }

    let accepted: Vec<&PlannedMove<'_>> = moves
        .iter()
        .filter(|planned| !refused.contains_key(&planned.record.id))
        .collect();

    // Staging names are chosen up front so the journal can name them before anything
    // moves.
    let temps: Vec<PathBuf> = accepted
        .iter()
        .map(|planned| unique_temp_path(planned.source))
        .collect();
    let journal = journal_dir
        .filter(|_| !accepted.is_empty())
        .and_then(|dir| {
            let plan = RenamePlan {
                items: accepted
                    .iter()
                    .zip(&temps)
                    .map(|(planned, temp)| RenamePlanItem {
                        file_id: planned.record.id,
                        original_path: planned.source.to_path_buf(),
                        temp_path: temp.clone(),
                        new_path: planned.dest.to_path_buf(),
                        was_directory: planned.record.was_directory,
                        source_identity: file_identity(planned.source),
                    })
                    .collect(),
                skipped: Vec::new(),
            };
            match RecoveryJournal::write(dir, &plan) {
                Ok(journal) => Some(journal),
                Err(err) => {
                    tracing::warn!("Could not write the crash-recovery journal: {}", err);
                    None
                }
            }
        });

    let mut staged: Vec<(&PlannedMove<'_>, PathBuf)> = Vec::new();
    let mut aborted = None;
    for (planned, temp) in accepted.iter().copied().zip(temps) {
        match fs::rename(planned.source, &temp) {
            Ok(()) => staged.push((planned, temp)),
            Err(e) => {
                aborted = Some(e.to_string());
                break;
            }
        }
    }

    if let Some(reason) = aborted {
        // A source that failed to stage still sits on a path that may be another
        // reversal's destination, so finishing would rename straight over it.
        for (planned, temp) in staged {
            let note = restore_staged(&temp, planned.source);
            refused.insert(planned.record.id, format!("{}{}", reason, note));
        }
        for planned in accepted {
            refused
                .entry(planned.record.id)
                .or_insert_with(|| reason.clone());
        }
    } else {
        for (planned, temp) in staged {
            if let Err(e) = fs::rename(&temp, planned.dest) {
                let note = restore_staged(&temp, planned.source);
                refused.insert(planned.record.id, format!("{}{}", e, note));
            }
        }
    }

    if let Some(journal) = journal {
        journal.finish_with(
            refused
                .values()
                .any(|reason| reason.contains(STRANDED_MARKER)),
        );
    }

    moves
        .iter()
        .map(|planned| {
            let error = refused.remove(&planned.record.id);
            UndoRecordResult {
                record_id: planned.record.id,
                success: error.is_none(),
                error,
            }
        })
        .collect()
}

/// Put a staged file back where it came from, and describe what went wrong when
/// it could not be. Returns an empty string once the file is home again.
fn restore_staged(temp: &Path, source: &Path) -> String {
    // A reversal earlier in the same batch may already have restored a file onto
    // this path, and renaming over it is the very loss the rollback exists to
    // prevent. Leaving the file staged keeps both, and says where the other is.
    if fs::symlink_metadata(source).is_ok() {
        return format!(
            "; rollback skipped because '{}' is occupied, file left at '{}'",
            source.display(),
            temp.display()
        );
    }

    fs::rename(temp, source)
        .err()
        .map(|err| format!("; rollback failed, file left at '{}': {}", temp.display(), err))
        .unwrap_or_default()
}

/// Split `records` into the ones that moved and the ones that did not,
/// preserving batch order in both.
fn partition_by_outcome(
    records: &[RenameRecord],
    results: &[UndoRecordResult],
) -> (Vec<RenameRecord>, Vec<RenameRecord>) {
    let moved: HashSet<Uuid> = results
        .iter()
        .filter(|result| result.success)
        .map(|result| result.record_id)
        .collect();
    records
        .iter()
        .cloned()
        .partition(|record| moved.contains(&record.id))
}

/// Manager for undo operations.
pub struct UndoManager {
    /// Stack of completed batches that can be undone.
    undo_stack: VecDeque<StoredBatch>,
    /// Stack of undone batches that can be redone.
    redo_stack: VecDeque<StoredBatch>,
    /// Directory for storing undo data files.
    data_dir: PathBuf,
    /// Whether to persist undo data to disk.
    persist_to_disk: bool,
}

impl UndoManager {
    /// Create a new undo manager.
    pub fn new(data_dir: PathBuf, persist_to_disk: bool) -> Self {
        // Ensure data directory exists
        if persist_to_disk {
            let _ = fs::create_dir_all(&data_dir);
            let _ = fs::create_dir_all(data_dir.join(REDO_SUBDIR));
        }

        Self {
            undo_stack: VecDeque::new(),
            redo_stack: VecDeque::new(),
            data_dir,
            persist_to_disk,
        }
    }

    /// Record a completed rename batch.
    pub fn record_batch(&mut self, batch: RenameBatch) -> RenamerResult<()> {
        // A failed write must not abort recording: the in-memory stack is the
        // session's safety net and losing it over a full disk would turn one problem
        // into two.
        if let Some(err) = self.record(batch) {
            tracing::warn!("Could not persist the undo record: {}", err);
        }
        Ok(())
    }

    /// `record_batch` for a caller whose process ends right after, such as the CLI,
    /// where the in-memory stack undoes nothing: a history write that failed comes
    /// back as the error instead of only being logged. The batch is recorded in memory
    /// either way.
    pub fn record_batch_checked(&mut self, batch: RenameBatch) -> RenamerResult<()> {
        match self.record(batch) {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    /// Record `batch`, returning the error of a history write that failed.
    fn record(&mut self, batch: RenameBatch) -> Option<RenamerError> {
        let _lock = self.lock_data_dir();

        // Clear redo stack when new action is performed
        self.redo_stack.clear();
        if self.persist_to_disk {
            let _ = clear_json_files(&self.redo_dir());
        }

        let stored = StoredBatch::capture(batch, |r| &r.new_path);

        let persist_error = if self.persist_to_disk {
            self.save_batch_to_disk(&self.data_dir, &stored).err()
        } else {
            None
        };

        // Add to undo stack, dropping the persisted file of anything evicted so
        // the data directory stays bounded.
        while self.undo_stack.len() >= MAX_UNDO_HISTORY {
            let Some(evicted) = self.undo_stack.pop_front() else {
                break;
            };
            if self.persist_to_disk {
                let _ = delete_batch_from_disk(&self.data_dir, &evicted.batch.id);
            }
        }

        self.undo_stack.push_back(stored);
        persist_error
    }

    /// Enable or disable writing history to disk without touching the
    /// in-memory stacks: in-session undo keeps working either way.
    pub fn set_persistence(&mut self, persist: bool) {
        if persist && !self.persist_to_disk {
            let _ = fs::create_dir_all(&self.data_dir);
            let _ = fs::create_dir_all(self.data_dir.join(REDO_SUBDIR));
        }
        self.persist_to_disk = persist;
    }

    /// Check if undo is available.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Check if redo is available.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Get the description of the next undo operation.
    pub fn peek_undo(&self) -> Option<&RenameBatch> {
        self.undo_stack.back().map(|s| &s.batch)
    }

    /// Get the description of the next redo operation.
    pub fn peek_redo(&self) -> Option<&RenameBatch> {
        self.redo_stack.back().map(|s| &s.batch)
    }

    /// Undo the last rename batch.
    pub fn undo(&mut self) -> RenamerResult<UndoResult> {
        let index = self
            .undo_stack
            .len()
            .checked_sub(1)
            .ok_or(RenamerError::UndoNotAvailable {
                reason: "No operations to undo".to_string(),
            })?;
        self.undo_stored_at(index)
    }

    /// Every batch still available to undo, oldest first.
    pub fn history(&self) -> Vec<&RenameBatch> {
        self.undo_stack.iter().map(|stored| &stored.batch).collect()
    }

    /// Undo a specific batch wherever it sits in the stack.
    ///
    /// Later batches may have renamed the same files again; the fingerprint
    /// guard refuses exactly those records instead of moving the wrong file.
    pub fn undo_batch(&mut self, batch_id: Uuid) -> RenamerResult<UndoResult> {
        let index = self
            .undo_stack
            .iter()
            .position(|stored| stored.batch.id == batch_id)
            .ok_or(RenamerError::UndoNotAvailable {
                reason: "That batch is no longer in the undo history".to_string(),
            })?;
        self.undo_stored_at(index)
    }

    fn undo_stored_at(&mut self, index: usize) -> RenamerResult<UndoResult> {
        let _lock = self.lock_data_dir();

        let stored = self
            .undo_stack
            .remove(index)
            .ok_or(RenamerError::UndoNotAvailable {
                reason: "No operations to undo".to_string(),
            })?;
        let batch = &stored.batch;

        let moves: Vec<PlannedMove<'_>> = batch
            .records
            .iter()
            .map(|record| PlannedMove {
                record,
                source: &record.new_path,
                dest: &record.original_path,
            })
            .collect();
        let results = apply_moves(&moves, &stored.fingerprints, Some(&self.data_dir));
        let (undone_records, failed_records) = partition_by_outcome(&batch.records, &results);
        let success_count = undone_records.len();

        // The redo batch keeps the same ids and the same direction: redo replays
        // original_path -> new_path, exactly what this batch originally did.
        let redo_batch = RenameBatch {
            id: batch.id,
            timestamp: batch.timestamp,
            records: undone_records,
            description: batch.description.clone(),
        };
        let fully_undone = failed_records.is_empty();
        let redo_stored = StoredBatch::capture(redo_batch, |r| &r.original_path);

        if self.persist_to_disk && fully_undone {
            // Write the redo record before dropping the undo record so a crash
            // in between leaves a duplicate rather than no trail at all. The files
            // have moved by now, so a write that fails is logged rather than
            // returned: returning would drop this batch from both stacks below. The
            // undo record goes either way, since it no longer describes the disk.
            if !redo_stored.batch.records.is_empty() {
                if let Err(err) = self.save_batch_to_disk(&self.redo_dir(), &redo_stored) {
                    tracing::warn!("Could not persist the redo record: {}", err);
                }
            }
            let _ = delete_batch_from_disk(&self.data_dir, &batch.id);
        }
        // A partial undo keeps its record on disk: it still holds the mapping
        // for every record that was not reverted.

        let result = UndoResult {
            batch_id: batch.id,
            total_records: batch.records.len(),
            success_count,
            results,
        };

        // A record we refused is still the only note of where that file came
        // from, so it stays undoable: the user clears the obstruction and
        // retries. Dropping it here would lose the mapping outright whenever
        // undo persistence is off.
        let remainder = RenameBatch {
            id: batch.id,
            timestamp: batch.timestamp,
            records: failed_records,
            description: batch.description.clone(),
        };
        if !remainder.records.is_empty() {
            let at = index.min(self.undo_stack.len());
            self.undo_stack.insert(
                at,
                StoredBatch::with_fingerprints_from(remainder, &stored.fingerprints),
            );
        }

        if !redo_stored.batch.records.is_empty() {
            self.redo_stack.push_back(redo_stored);
        }

        Ok(result)
    }

    /// Redo the last undone batch.
    pub fn redo(&mut self) -> RenamerResult<UndoResult> {
        let _lock = self.lock_data_dir();

        let stored = self.redo_stack.pop_back().ok_or(RenamerError::UndoNotAvailable {
            reason: "No operations to redo".to_string(),
        })?;
        let batch = &stored.batch;

        let moves: Vec<PlannedMove<'_>> = batch
            .records
            .iter()
            .map(|record| PlannedMove {
                record,
                source: &record.original_path,
                dest: &record.new_path,
            })
            .collect();
        let results = apply_moves(&moves, &stored.fingerprints, Some(&self.data_dir));
        let (success_records, failed_records) = partition_by_outcome(&batch.records, &results);
        let success_count = success_records.len();
        let failed = !failed_records.is_empty();

        // Push successful operations back to undo stack
        let undo_batch = RenameBatch {
            id: batch.id,
            timestamp: batch.timestamp,
            records: success_records,
            description: batch.description.clone(),
        };
        let undo_stored = StoredBatch::capture(undo_batch, |r| &r.new_path);

        if self.persist_to_disk && !failed {
            // As in undo: the files have moved, so a failed write is logged and the
            // stacks below are still updated.
            if !undo_stored.batch.records.is_empty() {
                if let Err(err) = self.save_batch_to_disk(&self.data_dir, &undo_stored) {
                    tracing::warn!("Could not persist the undo record: {}", err);
                }
            }
            let _ = delete_batch_from_disk(&self.redo_dir(), &batch.id);
        }
        // As with undo, a partial redo keeps its record where it is: that file
        // still maps both directions for the records that did not move.

        let result = UndoResult {
            batch_id: batch.id,
            total_records: batch.records.len(),
            success_count,
            results,
        };

        // As on the undo side, a refused record stays redoable.
        let remainder = RenameBatch {
            id: batch.id,
            timestamp: batch.timestamp,
            records: failed_records,
            description: batch.description.clone(),
        };
        if !remainder.records.is_empty() {
            self.redo_stack
                .push_back(StoredBatch::with_fingerprints_from(
                    remainder,
                    &stored.fingerprints,
                ));
        }

        if !undo_stored.batch.records.is_empty() {
            self.undo_stack.push_back(undo_stored);
        }

        Ok(result)
    }

    /// Generate an undo shell script for a batch.
    ///
    /// A batch can be a swap, a rotation or a chain, and its records are in no
    /// particular order, so no sequence of plain `mv NEW ORIG` lines undoes it safely.
    /// The script stages the way the engine does: every renamed file first moves to a
    /// temporary name in its own directory, and only then does each go to its original
    /// name. Nothing moves until every check passes, no move ever replaces an existing
    /// file, and a script that stops part way lists the files left under temporary
    /// names.
    pub fn generate_undo_script(&self, batch: &RenameBatch) -> String {
        let tag: String = batch.id.simple().to_string().chars().take(8).collect();
        let mut renamed = String::new();
        let mut temporary = String::new();
        let mut original = String::new();
        for (index, record) in batch.records.iter().enumerate() {
            let temp = record
                .new_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default()
                .join(format!(".bulk-renamer-undo-{}-{}", tag, index));
            renamed.push_str(&format!("    {}\n", shell_escape(&record.new_path)));
            temporary.push_str(&format!("    {}\n", shell_escape(&temp)));
            original.push_str(&format!("    {}\n", shell_escape(&record.original_path)));
        }

        let mut script = String::new();
        script.push_str("#!/bin/bash\n");
        script.push_str("# Bulk Renamer Undo Script\n");
        script.push_str(&format!("# Generated: {}\n", batch.timestamp.format("%Y-%m-%d %H:%M:%S")));
        script.push_str(&format!("# Batch ID: {}\n", batch.id));
        script.push_str(UNDO_SCRIPT_INTRO);
        script.push_str(&format!(
            "renamed=(\n{})\ntemporary=(\n{})\noriginal=(\n{})\n",
            renamed, temporary, original
        ));
        script.push_str(UNDO_SCRIPT_BODY);
        script
    }

    /// Save undo script to a file.
    pub fn save_undo_script(&self, batch: &RenameBatch, path: &Path) -> RenamerResult<()> {
        let script = self.generate_undo_script(batch);
        let mut file = File::create(path).map_err(|e| RenamerError::Io(e))?;
        file.write_all(script.as_bytes())
            .map_err(|e| RenamerError::Io(e))?;

        // Make executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = file.metadata().map_err(|e| RenamerError::Io(e))?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).map_err(|e| RenamerError::Io(e))?;
        }

        Ok(())
    }

    /// Directory holding undone batches.
    fn redo_dir(&self) -> PathBuf {
        self.data_dir.join(REDO_SUBDIR)
    }

    /// Take an advisory lock on the data directory for the duration of a
    /// mutating operation, so two windows cannot interleave their writes.
    /// Dropping the returned file closes the descriptor and releases the lock.
    #[cfg(unix)]
    fn lock_data_dir(&self) -> Option<File> {
        use nix::fcntl::{FlockArg, flock};
        use std::os::unix::io::AsRawFd;

        if !self.persist_to_disk {
            return None;
        }
        let file = File::create(self.data_dir.join(LOCK_FILE_NAME)).ok()?;
        flock(file.as_raw_fd(), FlockArg::LockExclusive).ok()?;
        Some(file)
    }

    #[cfg(not(unix))]
    fn lock_data_dir(&self) -> Option<File> {
        None
    }

    /// Save a batch to disk for persistence.
    ///
    /// Written to a temporary file that is fsynced and then renamed, so a crash
    /// or a full disk can never leave a half-written record behind.
    fn save_batch_to_disk(&self, dir: &Path, stored: &StoredBatch) -> RenamerResult<()> {
        fs::create_dir_all(dir).map_err(|e| RenamerError::Io(e))?;
        let path = dir.join(format!("{}.json", stored.batch.id));
        let temp_path = dir.join(format!("{}.json.tmp", stored.batch.id));

        let file = File::create(&temp_path).map_err(|e| RenamerError::Io(e))?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, stored).map_err(|e| RenamerError::JsonError(e))?;
        // BufWriter's Drop flushes but discards the error; do it explicitly so a
        // failed write is reported instead of yielding a truncated record.
        writer.flush().map_err(|e| RenamerError::Io(e))?;
        let file = writer
            .into_inner()
            .map_err(|e| RenamerError::Io(e.into_error()))?;
        file.sync_all().map_err(|e| RenamerError::Io(e))?;
        drop(file);

        fs::rename(&temp_path, &path).map_err(|e| RenamerError::Io(e))?;
        Ok(())
    }

    /// Load undo history from disk.
    pub fn load_from_disk(&mut self) -> RenamerResult<()> {
        if !self.data_dir.exists() {
            return Ok(());
        }

        self.redo_stack = load_batches(&self.redo_dir());
        let redo_ids: Vec<Uuid> = self.redo_stack.iter().map(|s| s.batch.id).collect();

        // A crash between writing the redo record and deleting the undo record
        // leaves the same batch in both places; the redo copy wins.
        self.undo_stack = load_batches(&self.data_dir)
            .into_iter()
            .filter(|s| !redo_ids.contains(&s.batch.id))
            .collect();

        Ok(())
    }

    /// Clear all undo history.
    pub fn clear(&mut self) -> RenamerResult<()> {
        let _lock = self.lock_data_dir();

        self.undo_stack.clear();
        self.redo_stack.clear();

        // Clear persisted files
        if self.persist_to_disk && self.data_dir.exists() {
            let _ = clear_json_files(&self.redo_dir());
            clear_json_files(&self.data_dir)?;
        }

        Ok(())
    }

    /// The directory this manager keeps its history in, which is also where the
    /// crash-recovery journals of the app's batches live.
    pub(crate) fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Get the number of undo operations available.
    pub fn undo_count(&self) -> usize {
        self.undo_stack.len()
    }

    /// Get the number of redo operations available.
    pub fn redo_count(&self) -> usize {
        self.redo_stack.len()
    }
}

/// Delete a batch file from disk.
fn delete_batch_from_disk(dir: &Path, batch_id: &Uuid) -> RenamerResult<()> {
    let path = dir.join(format!("{}.json", batch_id));
    if path.exists() {
        fs::remove_file(&path).map_err(|e| RenamerError::Io(e))?;
    }
    Ok(())
}

/// Whether `path` is a crash-recovery journal, which shares the data directory with
/// the batch files but is not one.
fn is_journal(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(is_journal_name)
}

/// Remove every batch file in a directory. Recovery journals stay: they are the
/// only record of files an interrupted batch left under staging names.
fn clear_json_files(dir: &Path) -> RenamerResult<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir).map_err(|e| RenamerError::Io(e))? {
        let entry = entry.map_err(|e| RenamerError::Io(e))?;
        let path = entry.path();
        if path.extension().map(|e| e == "json").unwrap_or(false) && !is_journal(&path) {
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
}

/// Load the most recent batches from a directory, oldest first.
fn load_batches(dir: &Path) -> VecDeque<StoredBatch> {
    let Ok(entries) = fs::read_dir(dir) else {
        return VecDeque::new();
    };

    let mut candidates: Vec<(SystemTime, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.extension().map(|e| e == "json").unwrap_or(false) || is_journal(&path) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        candidates.push((meta.modified().unwrap_or(UNIX_EPOCH), path));
    }

    // Only the newest files can survive the history cap, so deserialise just
    // those instead of every file an older build may have left behind.
    candidates.sort_by_key(|candidate| candidate.0);
    if candidates.len() > MAX_UNDO_HISTORY {
        candidates.drain(..candidates.len() - MAX_UNDO_HISTORY);
    }

    let mut batches: Vec<StoredBatch> = Vec::new();
    for (_, path) in candidates {
        match File::open(&path)
            .map_err(|e| e.to_string())
            .and_then(|f| {
                serde_json::from_reader::<_, StoredBatch>(BufReader::new(f))
                    .map_err(|e| e.to_string())
            }) {
            Ok(batch) => batches.push(batch),
            Err(e) => tracing::warn!("Skipping unreadable undo record {}: {}", path.display(), e),
        }
    }

    batches.sort_by(|a, b| a.batch.timestamp.cmp(&b.batch.timestamp));
    batches.into_iter().collect()
}

/// Result of an undo/redo operation.
#[derive(Debug, Clone)]
pub struct UndoResult {
    pub batch_id: Uuid,
    pub total_records: usize,
    pub success_count: usize,
    pub results: Vec<UndoRecordResult>,
}

/// Result of undoing a single record.
#[derive(Debug, Clone)]
pub struct UndoRecordResult {
    pub record_id: Uuid,
    pub success: bool,
    pub error: Option<String>,
}

impl UndoResult {
    pub fn all_successful(&self) -> bool {
        self.success_count == self.total_records
    }
}

/// Opening comment of an undo script, after its header lines.
const UNDO_SCRIPT_INTRO: &str = r#"#
# Puts every file of this batch back under its original name. Each file first
# moves to a temporary name next to it and then to its original name, so swapped
# names come back too. Nothing moves until every check passes, and no move ever
# replaces an existing file.

set -u

# The renamed path, temporary path and original path of each file, by position.
"#;

/// The checks and the two phases of an undo script, after its path arrays.
const UNDO_SCRIPT_BODY: &str = r#"
count=${#renamed[@]}
staged=()

exists() { [ -e "$1" ] || [ -L "$1" ]; }

refuse() {
    printf 'Undo refused, nothing was changed: %s\n' "$1" >&2
    exit 1
}

stop() {
    printf 'Undo stopped: %s\n' "$1" >&2
    local i
    for i in "${!staged[@]}"; do
        printf 'Still under a temporary name: %s (originally %s)\n' \
            "${temporary[i]}" "${original[i]}" >&2
    done
    exit 1
}

# Move $1 to $2 only while $2 is free, and count it done only once $1 is gone.
move() {
    exists "$2" && return 1
    mv -n -T -- "$1" "$2" || return 1
    ! exists "$1"
}

for ((i = 0; i < count; i++)); do
    exists "${renamed[i]}" || refuse "missing: ${renamed[i]}"
    exists "${temporary[i]}" && refuse "temporary name already in use: ${temporary[i]}"
done

for ((i = 0; i < count; i++)); do
    exists "${original[i]}" || continue
    freed=no
    for ((j = 0; j < count; j++)); do
        if [ "${original[i]}" = "${renamed[j]}" ]; then
            freed=yes
            break
        fi
    done
    [ "$freed" = yes ] && continue
    # The file itself under other casing, on a case-insensitive filesystem.
    [ "${original[i]}" -ef "${renamed[i]}" ] && continue
    refuse "something else is already at ${original[i]}"
done

for ((i = 0; i < count; i++)); do
    move "${renamed[i]}" "${temporary[i]}" || stop "could not move ${renamed[i]} aside"
    staged[i]=1
done

for ((i = 0; i < count; i++)); do
    move "${temporary[i]}" "${original[i]}" \
        || stop "could not move ${temporary[i]} to ${original[i]}"
    unset 'staged[i]'
done

echo "Undo completed successfully."
"#;

/// Quote a path for bash so it reaches `mv` byte for byte. UTF-8 text without
/// control characters goes in single quotes. Anything else (bytes that are not UTF-8,
/// newlines, tabs) becomes a bash `$'...'` string with `\xHH` escapes, which keeps
/// the script itself printable text.
fn shell_escape(path: &Path) -> String {
    let bytes = path_bytes(path);
    match std::str::from_utf8(&bytes) {
        Ok(text) if !text.chars().any(char::is_control) => {
            format!("'{}'", text.replace('\'', "'\\''"))
        }
        _ => {
            let mut quoted = String::from("$'");
            for &byte in bytes.iter() {
                match byte {
                    b'\\' => quoted.push_str("\\\\"),
                    b'\'' => quoted.push_str("\\'"),
                    0x20..=0x7e => quoted.push(char::from(byte)),
                    _ => quoted.push_str(&format!("\\x{:02x}", byte)),
                }
            }
            quoted.push('\'');
            quoted
        }
    }
}

/// The raw bytes of a path.
#[cfg(unix)]
fn path_bytes(path: &Path) -> std::borrow::Cow<'_, [u8]> {
    use std::os::unix::ffi::OsStrExt;
    std::borrow::Cow::Borrowed(path.as_os_str().as_bytes())
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> std::borrow::Cow<'_, [u8]> {
    std::borrow::Cow::Owned(path.to_string_lossy().into_owned().into_bytes())
}

impl Default for UndoManager {
    fn default() -> Self {
        Self::new(default_data_dir(), true)
    }
}

/// The application's undo/journal data directory.
pub fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bulk-renamer")
        .join("undo")
}

#[cfg(test)]
mod undo_safety_tests {
    use super::*;
    use crate::core::{
        FileEntry, RenameConfig, RenameRule, RenameStatus, ReplaceRule, RuleType,
    };
    use crate::engine::{RenameEngine, execute_renames};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bulk-renamer-undo-{}-{}", name, Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn replace_config(find: &str, replace: &str) -> RenameConfig {
        replace_configs(&[(find, replace)])
    }

    fn replace_configs(pairs: &[(&str, &str)]) -> RenameConfig {
        RenameConfig {
            rules: pairs
                .iter()
                .map(|(find, replace)| {
                    RenameRule::new(RuleType::Replace(ReplaceRule {
                        find: find.to_string(),
                        replace: replace.to_string(),
                        ..Default::default()
                    }))
                })
                .collect(),
            ..Default::default()
        }
    }

    /// Run the real pipeline: generate_previews -> validate/plan -> execute.
    fn run_rename(files: &[PathBuf], config: RenameConfig) -> RenameBatch {
        let entries: Vec<FileEntry> = files
            .iter()
            .map(|p| FileEntry::from_path(p.clone(), 0).expect("file entry"))
            .collect();
        let mut engine = RenameEngine::new(config);
        let previews = engine.generate_previews(&entries);
        assert!(
            previews
                .iter()
                .all(|p| p.status == RenameStatus::WillRename),
            "previews are not all renameable: {:?}",
            previews
        );
        let map: HashMap<Uuid, FileEntry> = entries.into_iter().map(|e| (e.id, e)).collect();
        let result = execute_renames(&previews, &map).expect("execute renames");
        assert!(result.all_successful(), "rename failed: {:?}", result.failures);
        result.batch.expect("batch")
    }

    fn json_count(dir: &Path) -> usize {
        fs::read_dir(dir)
            .expect("read dir")
            .flatten()
            .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
            .count()
    }

    /// Defect 1: redo was built from already-swapped paths, so it re-applied the
    /// inverse and never restored the renamed name.
    #[test]
    fn redo_reapplies_the_rename() {
        let dir = temp_dir("redo");
        let data = temp_dir("redo-data");
        let a = dir.join("a.txt");
        fs::write(&a, "content").expect("write a");

        let batch = run_rename(std::slice::from_ref(&a), replace_config("a", "z"));
        let z = dir.join("z.txt");
        assert!(z.exists());

        let mut manager = UndoManager::new(data.clone(), true);
        manager.record_batch(batch).expect("record");

        let undo = manager.undo().expect("undo");
        assert_eq!(undo.success_count, 1);
        assert!(a.exists() && !z.exists());

        let redo = manager.redo().expect("redo");
        assert_eq!(redo.success_count, 1);
        assert!(z.exists() && !a.exists());
        assert_eq!(fs::read_to_string(&z).expect("read z"), "content");

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// Defect 2: undo deleted the durable record and kept the redo state only in
    /// RAM, so a process that undid and exited lost both.
    #[test]
    fn undo_state_survives_a_restart() {
        let dir = temp_dir("durable");
        let data = temp_dir("durable-data");
        let a = dir.join("a.txt");
        fs::write(&a, "content").expect("write a");

        let batch = run_rename(std::slice::from_ref(&a), replace_config("a", "z"));
        let mut manager = UndoManager::new(data.clone(), true);
        manager.record_batch(batch).expect("record");
        assert_eq!(manager.undo().expect("undo").success_count, 1);
        drop(manager);

        let mut reloaded = UndoManager::new(data.clone(), true);
        reloaded.load_from_disk().expect("load");
        assert_eq!(reloaded.undo_count(), 0);
        assert_eq!(reloaded.redo_count(), 1);

        let redo = reloaded.redo().expect("redo");
        assert_eq!(redo.success_count, 1);
        assert!(dir.join("z.txt").exists());

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// Defect 3: a partial undo deleted the record anyway, losing the mapping
    /// for everything that was not reverted.
    #[test]
    fn partial_undo_keeps_its_record_on_disk() {
        let dir = temp_dir("partial");
        let data = temp_dir("partial-data");
        let a = dir.join("a1.txt");
        let b = dir.join("a2.txt");
        fs::write(&a, "one").expect("write a");
        fs::write(&b, "two").expect("write b");

        let batch = run_rename(&[a.clone(), b.clone()], replace_config("a", "z"));
        let mut manager = UndoManager::new(data.clone(), true);
        manager.record_batch(batch).expect("record");

        // Something else now occupies one of the original paths.
        fs::write(&a, "squatter").expect("write squatter");

        let undo = manager.undo().expect("undo");
        assert_eq!(undo.success_count, 1);
        assert_eq!(json_count(&data), 1, "partial undo must keep its record");

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// The data-loss guard: fs::rename clobbers its destination on POSIX, so
    /// undo must refuse when the original path is occupied.
    #[test]
    fn undo_refuses_to_clobber_the_destination() {
        let dir = temp_dir("clobber");
        let data = temp_dir("clobber-data");
        let a = dir.join("a.txt");
        fs::write(&a, "original").expect("write a");

        let batch = run_rename(std::slice::from_ref(&a), replace_config("a", "z"));
        let z = dir.join("z.txt");
        let mut manager = UndoManager::new(data.clone(), true);
        manager.record_batch(batch).expect("record");

        fs::write(&a, "precious").expect("write precious");

        let undo = manager.undo().expect("undo");
        assert_eq!(undo.success_count, 0);
        assert_eq!(fs::read_to_string(&a).expect("read a"), "precious");
        assert!(z.exists(), "the renamed file must be left alone");

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// The identity guard: the file at the recorded path may no longer be the
    /// file that was renamed.
    #[test]
    fn undo_refuses_when_the_file_was_replaced() {
        let dir = temp_dir("identity");
        let data = temp_dir("identity-data");
        let a = dir.join("a.txt");
        fs::write(&a, "original").expect("write a");

        let batch = run_rename(std::slice::from_ref(&a), replace_config("a", "z"));
        let z = dir.join("z.txt");
        let mut manager = UndoManager::new(data.clone(), true);
        manager.record_batch(batch).expect("record");

        // A different file now sits at the renamed path.
        fs::remove_file(&z).expect("remove z");
        fs::write(&z, "impostor").expect("write impostor");

        let undo = manager.undo().expect("undo");
        assert_eq!(undo.success_count, 0);
        assert_eq!(fs::read_to_string(&z).expect("read z"), "impostor");
        assert!(!a.exists(), "the impostor must not be moved back");

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// A swap is a rename the engine explicitly supports, so undo has to be
    /// able to take it back: every destination is occupied by the other file.
    #[test]
    fn undo_restores_a_two_name_swap() {
        let dir = temp_dir("swap");
        let data = temp_dir("swap-data");
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        fs::write(&a, "content-a").expect("write a");
        fs::write(&b, "content-b").expect("write b");

        // a -> b and b -> a, routed through a sentinel neither name contains.
        let batch = run_rename(
            &[a.clone(), b.clone()],
            replace_configs(&[("a", "X"), ("b", "a"), ("X", "b")]),
        );
        assert_eq!(fs::read_to_string(&a).expect("read a"), "content-b");
        assert_eq!(fs::read_to_string(&b).expect("read b"), "content-a");

        let mut manager = UndoManager::new(data.clone(), true);
        manager.record_batch(batch).expect("record");

        let undo = manager.undo().expect("undo");
        assert_eq!(
            undo.success_count, 2,
            "a swap must be undoable: {:?}",
            undo.results
        );
        assert_eq!(fs::read_to_string(&a).expect("read a"), "content-a");
        assert_eq!(fs::read_to_string(&b).expect("read b"), "content-b");

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// A record that was refused is still the only note of where that file came
    /// from, so it has to stay undoable once the obstruction is cleared.
    #[test]
    fn a_refused_undo_can_be_retried() {
        let dir = temp_dir("retry");
        let data = temp_dir("retry-data");
        let a = dir.join("a.txt");
        fs::write(&a, "original").expect("write a");

        let batch = run_rename(std::slice::from_ref(&a), replace_config("a", "z"));
        let mut manager = UndoManager::new(data.clone(), true);
        manager.record_batch(batch).expect("record");

        // Something else occupies the original path, so undo must refuse.
        fs::write(&a, "squatter").expect("write squatter");
        assert_eq!(manager.undo().expect("undo").success_count, 0);

        // The user clears the obstruction and tries again.
        fs::remove_file(&a).expect("remove squatter");
        assert!(manager.can_undo(), "a refused undo must remain undoable");
        assert_eq!(manager.undo().expect("retry").success_count, 1);
        assert_eq!(fs::read_to_string(&a).expect("read a"), "original");

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// Undo stages every source aside before filling any destination, so a
    /// reversal that cannot land must not put its file back over a path another
    /// reversal has already restored.
    #[test]
    #[cfg(unix)]
    fn a_failed_reversal_does_not_overwrite_a_restored_file() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("rollback-clobber");
        let data = temp_dir("rollback-clobber-data");
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).expect("create sub");
        fs::write(dir.join("n.txt"), "n-content").expect("write n");
        fs::write(sub.join("m.txt"), "m-content").expect("write m");

        // n.txt -> v.txt vacates the name that sub/m.txt then moves up into.
        // The batch is built directly rather than through run_rename: the engine
        // now refuses a generated name containing a separator, so a cross-directory
        // move is no longer reachable through the rules API. Historical batches on
        // disk can still contain one, which is exactly what undo has to survive.
        fs::rename(dir.join("n.txt"), dir.join("v.txt")).expect("stage n -> v");
        fs::rename(sub.join("m.txt"), dir.join("n.txt")).expect("stage m -> n");
        let batch = RenameBatch::new(vec![
            RenameRecord {
                id: Uuid::new_v4(),
                timestamp: chrono::Local::now(),
                original_path: dir.join("n.txt"),
                new_path: dir.join("v.txt"),
                was_directory: false,
            },
            RenameRecord {
                id: Uuid::new_v4(),
                timestamp: chrono::Local::now(),
                original_path: sub.join("m.txt"),
                new_path: dir.join("n.txt"),
                was_directory: false,
            },
        ]);
        assert_eq!(
            fs::read_to_string(dir.join("v.txt")).expect("read v"),
            "n-content"
        );
        assert_eq!(
            fs::read_to_string(dir.join("n.txt")).expect("read n"),
            "m-content"
        );

        let mut manager = UndoManager::new(data.clone(), true);
        manager.record_batch(batch).expect("record");

        // sub stays traversable but is no longer writable, so m.txt can be
        // staged out of the way and then not be put back.
        fs::set_permissions(&sub, fs::Permissions::from_mode(0o555)).expect("chmod sub");
        if fs::write(sub.join("probe"), "x").is_err() {
            let undo = manager.undo().expect("undo");
            assert_eq!(
                fs::read_to_string(dir.join("n.txt")).expect("read n"),
                "n-content",
                "a reversal that failed overwrote the file another reversal restored: {:?}",
                undo.results
            );
            // The file that could not be put back still has to exist somewhere.
            let found = fs::read_dir(&dir)
                .expect("read dir")
                .flatten()
                .any(|e| fs::read_to_string(e.path()).map(|c| c == "m-content").unwrap_or(false));
            assert!(found, "m.txt's contents were lost: {:?}", undo.results);
        }

        fs::set_permissions(&sub, fs::Permissions::from_mode(0o755)).ok();
        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// A rotation is a cycle with no two-element shortcut, so every destination
    /// is held by a different member of the batch.
    #[test]
    fn undo_restores_a_three_name_rotation() {
        let dir = temp_dir("rotation");
        let data = temp_dir("rotation-data");
        for (name, body) in [("a.txt", "A"), ("b.txt", "B"), ("c.txt", "C")] {
            fs::write(dir.join(name), body).expect("write");
        }

        // a -> c, b -> a, c -> b.
        let batch = run_rename(
            &[dir.join("a.txt"), dir.join("b.txt"), dir.join("c.txt")],
            replace_configs(&[("a", "X"), ("b", "a"), ("c", "b"), ("X", "c")]),
        );
        assert_eq!(fs::read_to_string(dir.join("c.txt")).expect("read c"), "A");

        let mut manager = UndoManager::new(data.clone(), true);
        manager.record_batch(batch).expect("record");

        let undo = manager.undo().expect("undo");
        assert_eq!(
            undo.success_count, 3,
            "a rotation must be undoable: {:?}",
            undo.results
        );
        for (name, body) in [("a.txt", "A"), ("b.txt", "B"), ("c.txt", "C")] {
            assert_eq!(
                fs::read_to_string(dir.join(name)).expect("read"),
                body,
                "{} was not restored",
                name
            );
        }

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// Redo replays the batch in its original direction, so it hits the same
    /// occupied destinations undo does and needs the same staging.
    #[test]
    fn undo_then_redo_then_undo_round_trips_a_swap() {
        let dir = temp_dir("round-trip");
        let data = temp_dir("round-trip-data");
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        fs::write(&a, "content-a").expect("write a");
        fs::write(&b, "content-b").expect("write b");

        let batch = run_rename(
            &[a.clone(), b.clone()],
            replace_configs(&[("a", "X"), ("b", "a"), ("X", "b")]),
        );
        let mut manager = UndoManager::new(data.clone(), true);
        manager.record_batch(batch).expect("record");

        assert_eq!(manager.undo().expect("undo").success_count, 2);
        assert_eq!(fs::read_to_string(&a).expect("read a"), "content-a");

        let redo = manager.redo().expect("redo");
        assert_eq!(
            redo.success_count, 2,
            "a swap must be redoable: {:?}",
            redo.results
        );
        assert_eq!(fs::read_to_string(&a).expect("read a"), "content-b");
        assert_eq!(fs::read_to_string(&b).expect("read b"), "content-a");

        assert_eq!(manager.undo().expect("second undo").success_count, 2);
        assert_eq!(fs::read_to_string(&a).expect("read a"), "content-a");
        assert_eq!(fs::read_to_string(&b).expect("read b"), "content-b");

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// Refusing one member of a swap parks its file on the other member's
    /// destination, so the refusal has to spread rather than let that one
    /// proceed and overwrite it.
    #[test]
    fn a_swap_with_one_member_replaced_moves_nothing() {
        let dir = temp_dir("swap-replaced");
        let data = temp_dir("swap-replaced-data");
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        fs::write(&a, "content-a").expect("write a");
        fs::write(&b, "content-b").expect("write b");

        let batch = run_rename(
            &[a.clone(), b.clone()],
            replace_configs(&[("a", "X"), ("b", "a"), ("X", "b")]),
        );
        let mut manager = UndoManager::new(data.clone(), true);
        manager.record_batch(batch).expect("record");

        // A different file now sits on one of the two renamed paths.
        fs::remove_file(&a).expect("remove a");
        fs::write(&a, "impostor").expect("write impostor");

        let undo = manager.undo().expect("undo");
        assert_eq!(
            undo.success_count, 0,
            "neither half of the swap can move: {:?}",
            undo.results
        );
        assert_eq!(fs::read_to_string(&a).expect("read a"), "impostor");
        assert_eq!(fs::read_to_string(&b).expect("read b"), "content-a");

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// Refusing one reversal leaves its file parked on its source, which is the
    /// destination of the reversal behind it in the chain. That one has to be
    /// refused too, or it renames straight over a file nothing else will move.
    #[test]
    fn a_refusal_spreads_back_along_the_chain() {
        let dir = temp_dir("chain");
        let data = temp_dir("chain-data");
        let p = dir.join("p.txt");
        let q = dir.join("q.txt");
        fs::write(&p, "P").expect("write p");
        fs::write(&q, "Q").expect("write q");

        // p -> q and q -> r: q is only free because p's move waits for it.
        let batch = run_rename(
            &[p.clone(), q.clone()],
            replace_configs(&[("q", "r"), ("p", "q")]),
        );
        assert_eq!(fs::read_to_string(&q).expect("read q"), "P");
        assert_eq!(fs::read_to_string(dir.join("r.txt")).expect("read r"), "Q");

        let mut manager = UndoManager::new(data.clone(), true);
        manager.record_batch(batch).expect("record");

        // Something else takes p.txt, so p's file can never go home and q.txt
        // stays occupied for good.
        fs::write(&p, "squatter").expect("write squatter");

        let undo = manager.undo().expect("undo");
        assert_eq!(
            undo.success_count, 0,
            "nothing can move once the head of the chain is blocked: {:?}",
            undo.results
        );
        assert_eq!(fs::read_to_string(&p).expect("read p"), "squatter");
        assert_eq!(
            fs::read_to_string(&q).expect("read q"),
            "P",
            "a blocked reversal was overwritten by the one behind it"
        );
        assert_eq!(fs::read_to_string(dir.join("r.txt")).expect("read r"), "Q");

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// A record truncated by a crash or a full disk must not cost us the
    /// records that are still intact.
    #[test]
    fn a_corrupt_record_does_not_take_the_others_down() {
        let dir = temp_dir("corrupt");
        let data = temp_dir("corrupt-data");
        let a = dir.join("a.txt");
        fs::write(&a, "content").expect("write a");

        let batch = run_rename(std::slice::from_ref(&a), replace_config("a", "z"));
        let mut manager = UndoManager::new(data.clone(), true);
        manager.record_batch(batch).expect("record");
        drop(manager);

        // A half-written record, a well-formed file of the wrong shape, and an
        // empty one, all next to the good record.
        let good = fs::read_to_string(
            fs::read_dir(&data)
                .expect("read data")
                .flatten()
                .map(|e| e.path())
                .find(|p| p.extension().map(|x| x == "json").unwrap_or(false))
                .expect("good record"),
        )
        .expect("read good record");
        fs::write(data.join("truncated.json"), &good[..good.len() / 2]).expect("write truncated");
        fs::write(data.join("wrong-shape.json"), r#"{"hello":"world"}"#).expect("write wrong");
        fs::write(data.join("empty.json"), "").expect("write empty");

        let mut reloaded = UndoManager::new(data.clone(), true);
        reloaded.load_from_disk().expect("load");
        assert_eq!(reloaded.undo_count(), 1, "the intact record must survive");
        assert_eq!(reloaded.undo().expect("undo").success_count, 1);
        assert_eq!(fs::read_to_string(&a).expect("read a"), "content");

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// Two windows can hold the same batch. Once one has undone it, the other
    /// must not move anything on top of the result.
    #[test]
    fn a_second_process_undoing_the_same_batch_changes_nothing() {
        let dir = temp_dir("two-windows");
        let data = temp_dir("two-windows-data");
        let a = dir.join("a.txt");
        fs::write(&a, "content").expect("write a");

        let batch = run_rename(std::slice::from_ref(&a), replace_config("a", "z"));
        let mut writer = UndoManager::new(data.clone(), true);
        writer.record_batch(batch).expect("record");
        drop(writer);

        let mut first = UndoManager::new(data.clone(), true);
        first.load_from_disk().expect("load first");
        let mut second = UndoManager::new(data.clone(), true);
        second.load_from_disk().expect("load second");

        assert_eq!(first.undo().expect("first undo").success_count, 1);
        assert_eq!(fs::read_to_string(&a).expect("read a"), "content");

        let late = second.undo().expect("second undo");
        assert_eq!(
            late.success_count, 0,
            "the batch was already undone: {:?}",
            late.results
        );
        assert_eq!(
            fs::read_to_string(&a).expect("read a"),
            "content",
            "the restored file was disturbed by the second undo"
        );
        assert!(!dir.join("z.txt").exists());

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// Defect 4: eviction dropped the batch from memory but left its file on
    /// disk forever.
    #[test]
    fn eviction_deletes_the_evicted_file() {
        let dir = temp_dir("evict");
        let data = temp_dir("evict-data");
        let mut manager = UndoManager::new(data.clone(), true);

        for i in 0..(MAX_UNDO_HISTORY + 5) {
            let path = dir.join(format!("a{}.txt", i));
            fs::write(&path, "x").expect("write");
            let batch = run_rename(&[path], replace_config("a", "z"));
            manager.record_batch(batch).expect("record");
        }

        assert_eq!(manager.undo_count(), MAX_UNDO_HISTORY);
        assert_eq!(json_count(&data), MAX_UNDO_HISTORY);

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// Defect 5: records are written through a temp file, so a reader never
    /// sees a partially written record and no temp residue is left behind.
    #[test]
    fn records_are_written_atomically() {
        let dir = temp_dir("atomic");
        let data = temp_dir("atomic-data");
        let a = dir.join("a.txt");
        fs::write(&a, "content").expect("write a");

        let batch = run_rename(&[a], replace_config("a", "z"));
        let mut manager = UndoManager::new(data.clone(), true);
        manager.record_batch(batch).expect("record");

        let leftovers: Vec<PathBuf> = fs::read_dir(&data)
            .expect("read data dir")
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "tmp").unwrap_or(false))
            .collect();
        assert!(leftovers.is_empty(), "temp residue: {:?}", leftovers);
        assert_eq!(json_count(&data), 1);

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// Records written by an older version have no fingerprints; they must
    /// still load and still undo.
    #[test]
    fn legacy_records_without_fingerprints_still_load() {
        let dir = temp_dir("legacy");
        let data = temp_dir("legacy-data");
        let a = dir.join("a.txt");
        fs::write(&a, "content").expect("write a");

        let batch = run_rename(std::slice::from_ref(&a), replace_config("a", "z"));
        let legacy_path = data.join(format!("{}.json", batch.id));
        let file = File::create(&legacy_path).expect("create legacy");
        serde_json::to_writer_pretty(file, &batch).expect("write legacy");

        let mut manager = UndoManager::new(data.clone(), true);
        manager.load_from_disk().expect("load");
        assert_eq!(manager.undo_count(), 1);
        assert_eq!(manager.undo().expect("undo").success_count, 1);
        assert!(a.exists());

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// Defect 6: startup must not deserialise an unbounded backlog, and must
    /// still keep the most recent history.
    #[test]
    fn load_keeps_only_the_most_recent_history() {
        let dir = temp_dir("load-cap");
        let data = temp_dir("load-cap-data");
        let manager = UndoManager::new(data.clone(), true);

        for i in 0..(MAX_UNDO_HISTORY + 5) {
            let path = dir.join(format!("a{}.txt", i));
            fs::write(&path, "x").expect("write");
            let batch = run_rename(&[path], replace_config("a", "z"));
            // Bypass eviction to simulate the backlog an older build left.
            let stored = StoredBatch::capture(batch, |r| &r.new_path);
            manager
                .save_batch_to_disk(&data, &stored)
                .expect("save");
        }

        let mut reloaded = UndoManager::new(data.clone(), true);
        reloaded.load_from_disk().expect("load");
        assert_eq!(reloaded.undo_count(), MAX_UNDO_HISTORY);

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// Defect 7: concurrent managers sharing a data directory must not corrupt
    /// each other's records.
    #[test]
    fn concurrent_managers_do_not_corrupt_the_store() {
        let dir = temp_dir("concurrent");
        let data = temp_dir("concurrent-data");

        let handles: Vec<_> = (0..2)
            .map(|worker| {
                let dir = dir.clone();
                let data = data.clone();
                std::thread::spawn(move || {
                    let mut manager = UndoManager::new(data, true);
                    for i in 0..10 {
                        let path = dir.join(format!("a{}_{}.txt", worker, i));
                        fs::write(&path, "x").expect("write");
                        let batch = run_rename(&[path], replace_config("a", "z"));
                        manager.record_batch(batch).expect("record");
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("join");
        }

        let mut reloaded = UndoManager::new(data.clone(), true);
        reloaded.load_from_disk().expect("load");
        assert_eq!(reloaded.undo_count(), 20);

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// A record refused because the file at its path changed must keep
    /// refusing on retry. Re-capturing fingerprints on push-back used to adopt
    /// the impostor's identity, and the second Ctrl+Z then moved the wrong
    /// file onto the original path.
    #[test]
    fn a_refused_undo_keeps_refusing_the_impostor_on_retry() {
        let dir = temp_dir("impostor-retry");
        let data = dir.join("data");
        let a = dir.join("a.txt");
        fs::write(&a, "original").expect("write a");

        let batch = run_rename(&[a.clone()], replace_config("a", "z"));
        let z = dir.join("z.txt");
        assert!(z.exists());

        let mut manager = UndoManager::new(data, false);
        manager.record_batch(batch).expect("record");

        // Replace the renamed file with an impostor under a fresh inode.
        fs::remove_file(&z).expect("remove renamed file");
        fs::write(&z, "impostor").expect("write impostor");

        let first = manager.undo().expect("first undo runs");
        assert_eq!(first.success_count, 0, "the impostor must be refused");
        assert!(manager.can_undo(), "the refused record stays undoable");

        let second = manager.undo().expect("second undo runs");
        assert_eq!(
            second.success_count, 0,
            "the retry must keep refusing: adopting the impostor's fingerprint \
             on push-back is exactly the bug this test guards against"
        );
        assert!(!a.exists(), "the impostor must never land on the original path");
        assert_eq!(
            fs::read_to_string(&z).expect("read impostor"),
            "impostor",
            "the impostor stays where it is"
        );

        fs::remove_dir_all(dir).ok();
    }

    /// In-session undo must work with persistence off: the setting controls
    /// the disk, not the safety net.
    #[test]
    fn undo_works_in_session_without_persistence() {
        let dir = temp_dir("no-persist");
        let data = dir.join("data");
        let a = dir.join("a.txt");
        fs::write(&a, "content").expect("write a");

        let batch = run_rename(&[a.clone()], replace_config("a", "b"));
        let mut manager = UndoManager::new(data.clone(), false);
        manager.record_batch(batch).expect("record");

        assert!(
            !data.exists()
                || fs::read_dir(&data)
                    .map(|mut entries| entries.next().is_none())
                    .unwrap_or(true),
            "nothing may be written to disk with persistence off"
        );

        let result = manager.undo().expect("undo");
        assert_eq!(result.success_count, 1);
        assert!(a.exists(), "the rename was reverted purely from memory");

        fs::remove_dir_all(dir).ok();
    }

    /// Toggling persistence on starts writing records without touching the
    /// in-memory history.
    #[test]
    fn set_persistence_starts_writing_without_dropping_history() {
        let dir = temp_dir("toggle-persist");
        let data = dir.join("data");
        let a = dir.join("a.txt");
        let b = dir.join("m.txt");
        fs::write(&a, "one").expect("write a");
        fs::write(&b, "two").expect("write b");

        let mut manager = UndoManager::new(data.clone(), false);
        let first = run_rename(&[a.clone()], replace_config("a", "x"));
        manager.record_batch(first).expect("record first");

        manager.set_persistence(true);
        let second = run_rename(&[b.clone()], replace_config("m", "n"));
        manager.record_batch(second).expect("record second");

        let on_disk = fs::read_dir(&data)
            .expect("data dir exists")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .count();
        assert_eq!(on_disk, 1, "only the batch recorded after the toggle is persisted");
        assert_eq!(manager.undo_count(), 2, "both batches stay undoable in memory");

        fs::remove_dir_all(dir).ok();
    }

    /// Per-batch undo: an older batch can be reverted while newer batches
    /// stay in the history.
    #[test]
    fn a_specific_batch_can_be_undone_out_of_order() {
        let dir = temp_dir("undo-batch");
        let data = dir.join("data");
        let a = dir.join("a.txt");
        let b = dir.join("m.txt");
        fs::write(&a, "one").expect("write a");
        fs::write(&b, "two").expect("write b");

        let mut manager = UndoManager::new(data, false);
        let first = run_rename(&[a.clone()], replace_config("a", "x"));
        let first_id = first.id;
        manager.record_batch(first).expect("record first");
        let second = run_rename(&[b.clone()], replace_config("m", "n"));
        manager.record_batch(second).expect("record second");

        let result = manager.undo_batch(first_id).expect("undo the older batch");
        assert_eq!(result.success_count, 1);
        assert!(a.exists(), "the older batch is reverted");
        assert!(dir.join("n.txt").exists(), "the newer batch is untouched");
        assert_eq!(manager.undo_count(), 1, "only the newer batch remains");
        assert!(manager.history().iter().all(|batch| batch.id != first_id));

        fs::remove_dir_all(dir).ok();
    }

    /// Run an exported undo script with bash, the way a user would.
    fn run_undo_script(script_dir: &Path, script: &str) -> std::process::Output {
        let path = script_dir.join("undo-rename.sh");
        fs::write(&path, script).expect("write script");
        std::process::Command::new("bash")
            .arg(&path)
            .output()
            .expect("run bash")
    }

    /// Names in `dir`, sorted, so a test can see stray staging files.
    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("read dir")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// The exported script replayed `mv NEW ORIG` in reverse record order, and the
    /// records of a two-phase batch are in no particular order: the first `mv` of a
    /// swap landed on the other member and destroyed it.
    #[test]
    fn the_undo_script_reverts_a_swap() {
        let dir = temp_dir("script-swap");
        let scripts = temp_dir("script-swap-scripts");
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        fs::write(&a, "content-a").expect("write a");
        fs::write(&b, "content-b").expect("write b");

        let batch = run_rename(
            &[a.clone(), b.clone()],
            replace_configs(&[("a", "X"), ("b", "a"), ("X", "b")]),
        );
        let manager = UndoManager::new(scripts.join("data"), false);
        let output = run_undo_script(&scripts, &manager.generate_undo_script(&batch));

        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(fs::read_to_string(&a).expect("read a"), "content-a");
        assert_eq!(fs::read_to_string(&b).expect("read b"), "content-b");
        assert_eq!(names_in(&dir), ["a.txt", "b.txt"]);

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(scripts).ok();
    }

    #[test]
    fn the_undo_script_reverts_a_three_name_rotation() {
        let dir = temp_dir("script-rotation");
        let scripts = temp_dir("script-rotation-scripts");
        for (name, body) in [("a.txt", "A"), ("b.txt", "B"), ("c.txt", "C")] {
            fs::write(dir.join(name), body).expect("write");
        }

        // a -> c, b -> a, c -> b.
        let batch = run_rename(
            &[dir.join("a.txt"), dir.join("b.txt"), dir.join("c.txt")],
            replace_configs(&[("a", "X"), ("b", "a"), ("c", "b"), ("X", "c")]),
        );
        let manager = UndoManager::new(scripts.join("data"), false);
        let output = run_undo_script(&scripts, &manager.generate_undo_script(&batch));

        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        for (name, body) in [("a.txt", "A"), ("b.txt", "B"), ("c.txt", "C")] {
            assert_eq!(fs::read_to_string(dir.join(name)).expect("read"), body, "{}", name);
        }
        assert_eq!(names_in(&dir), ["a.txt", "b.txt", "c.txt"]);

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(scripts).ok();
    }

    /// `mv` overwrites by default, so the script replaced whatever had arrived on an
    /// original path since the rename.
    #[test]
    fn the_undo_script_refuses_to_overwrite_and_moves_nothing() {
        let dir = temp_dir("script-refuse");
        let scripts = temp_dir("script-refuse-scripts");
        let a1 = dir.join("a1.txt");
        let a2 = dir.join("a2.txt");
        fs::write(&a1, "one").expect("write a1");
        fs::write(&a2, "two").expect("write a2");

        let batch = run_rename(&[a1.clone(), a2.clone()], replace_config("a", "z"));
        // Something new sits on one of the original paths.
        fs::write(&a2, "outsider").expect("write outsider");

        let manager = UndoManager::new(scripts.join("data"), false);
        let output = run_undo_script(&scripts, &manager.generate_undo_script(&batch));

        assert!(!output.status.success(), "the script must refuse");
        assert_eq!(fs::read_to_string(&a2).expect("read a2"), "outsider");
        assert!(!a1.exists(), "nothing may move once one check fails");
        assert_eq!(fs::read_to_string(dir.join("z1.txt")).expect("read z1"), "one");
        assert_eq!(fs::read_to_string(dir.join("z2.txt")).expect("read z2"), "two");
        assert_eq!(names_in(&dir), ["a2.txt", "z1.txt", "z2.txt"]);

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(scripts).ok();
    }

    /// The script quoted paths through `to_string_lossy`, so a name that is not UTF-8
    /// was written down with U+FFFD and could never be found again.
    #[test]
    #[cfg(unix)]
    fn the_undo_script_keeps_every_byte_of_a_name() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let dir = temp_dir("script-bytes");
        let scripts = temp_dir("script-bytes-scripts");
        let original = dir.join(OsStr::from_bytes(b"caf\xe9 'quoted' \\ $HOME\n.txt"));
        let renamed = dir.join("plain.txt");
        fs::write(&original, "latin-1").expect("write");
        fs::rename(&original, &renamed).expect("rename");
        let batch = RenameBatch::new(vec![RenameRecord {
            id: Uuid::new_v4(),
            timestamp: chrono::Local::now(),
            original_path: original.clone(),
            new_path: renamed.clone(),
            was_directory: false,
        }]);

        let manager = UndoManager::new(scripts.join("data"), false);
        let output = run_undo_script(&scripts, &manager.generate_undo_script(&batch));

        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(fs::read_to_string(&original).expect("read original"), "latin-1");
        assert!(!renamed.exists());

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(scripts).ok();
    }

    /// Undo stages files under hidden names just as a rename batch does, and a file
    /// it cannot put back has to be on record for the startup recovery to find.
    #[test]
    #[cfg(unix)]
    fn an_undo_that_strands_a_file_leaves_a_recovery_journal() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("undo-journal");
        let data = temp_dir("undo-journal-data");
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).expect("create sub");
        // The same two moves as a_failed_reversal_does_not_overwrite_a_restored_file.
        fs::write(dir.join("v.txt"), "n-content").expect("write v");
        fs::write(dir.join("n.txt"), "m-content").expect("write n");
        let batch = RenameBatch::new(vec![
            RenameRecord {
                id: Uuid::new_v4(),
                timestamp: chrono::Local::now(),
                original_path: dir.join("n.txt"),
                new_path: dir.join("v.txt"),
                was_directory: false,
            },
            RenameRecord {
                id: Uuid::new_v4(),
                timestamp: chrono::Local::now(),
                original_path: sub.join("m.txt"),
                new_path: dir.join("n.txt"),
                was_directory: false,
            },
        ]);
        let mut manager = UndoManager::new(data.clone(), true);
        manager.record_batch(batch).expect("record");
        assert_eq!(crate::engine::pending_recovery_count(&data), 0);

        fs::set_permissions(&sub, fs::Permissions::from_mode(0o555)).expect("chmod sub");
        if fs::write(sub.join("probe"), "x").is_err() {
            let undo = manager.undo().expect("undo");
            assert_eq!(
                crate::engine::pending_recovery_count(&data),
                1,
                "the stranded file is not on record: {:?}",
                undo.results
            );
        }

        fs::set_permissions(&sub, fs::Permissions::from_mode(0o755)).ok();
        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// A clean undo and redo leave no journal behind.
    #[test]
    fn a_clean_undo_and_redo_leave_no_journal() {
        let dir = temp_dir("undo-journal-clean");
        let data = temp_dir("undo-journal-clean-data");
        let a = dir.join("a.txt");
        fs::write(&a, "content").expect("write a");

        let batch = run_rename(std::slice::from_ref(&a), replace_config("a", "z"));
        let mut manager = UndoManager::new(data.clone(), true);
        manager.record_batch(batch).expect("record");
        assert_eq!(manager.undo().expect("undo").success_count, 1);
        assert_eq!(manager.redo().expect("redo").success_count, 1);

        let journals = fs::read_dir(&data)
            .expect("read data")
            .flatten()
            .filter(|entry| is_journal(&entry.path()))
            .count();
        assert_eq!(journals, 0);

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// The CLI ends right after recording, so a history write that failed has to
    /// reach it, not only a log the CLI never sets up.
    #[test]
    #[cfg(unix)]
    fn record_batch_checked_reports_a_failed_write() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let dir = temp_dir("checked-record");
        let data = temp_dir("checked-record-data");
        let mut manager = UndoManager::new(data.clone(), true);

        let a = dir.join("a.txt");
        fs::write(&a, "content").expect("write a");
        let batch = run_rename(std::slice::from_ref(&a), replace_config("a", "z"));
        manager.record_batch_checked(batch).expect("a writable batch persists");
        assert_eq!(json_count(&data), 1);

        // serde cannot write a path that is not UTF-8.
        let original = dir.join(OsStr::from_bytes(b"caf\xe9.txt"));
        let renamed = dir.join("cafe.txt");
        fs::write(&renamed, "latin-1").expect("write");
        let batch = RenameBatch::new(vec![RenameRecord {
            id: Uuid::new_v4(),
            timestamp: chrono::Local::now(),
            original_path: original,
            new_path: renamed,
            was_directory: false,
        }]);
        assert!(manager.record_batch_checked(batch).is_err());
        assert_eq!(manager.undo_count(), 2, "the batch is still recorded in memory");

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }

    /// Undo and redo returned an error when the history write failed, after the files
    /// had already moved, and dropped the batch from both stacks on the way out.
    #[test]
    #[cfg(unix)]
    fn a_failed_history_write_does_not_lose_the_undo() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let dir = temp_dir("unwritable-history");
        let data = temp_dir("unwritable-history-data");
        // serde cannot write a path that is not UTF-8, so every history write for this
        // batch fails, exactly as it would on a full disk.
        let original = dir.join(OsStr::from_bytes(b"caf\xe9.txt"));
        let renamed = dir.join("cafe.txt");
        fs::write(&original, "latin-1").expect("write");
        fs::rename(&original, &renamed).expect("rename");
        let batch = RenameBatch::new(vec![RenameRecord {
            id: Uuid::new_v4(),
            timestamp: chrono::Local::now(),
            original_path: original.clone(),
            new_path: renamed.clone(),
            was_directory: false,
        }]);

        let mut manager = UndoManager::new(data.clone(), true);
        manager.record_batch(batch).expect("record");

        let undo = manager.undo().expect("the files moved, so the undo happened");
        assert_eq!(undo.success_count, 1);
        assert!(original.exists() && !renamed.exists());
        assert!(manager.can_redo(), "the redo must survive a failed history write");

        let redo = manager.redo().expect("the files moved, so the redo happened");
        assert_eq!(redo.success_count, 1);
        assert!(renamed.exists() && !original.exists());
        assert!(manager.can_undo(), "the undo must survive a failed history write");

        fs::remove_dir_all(dir).ok();
        fs::remove_dir_all(data).ok();
    }
}
