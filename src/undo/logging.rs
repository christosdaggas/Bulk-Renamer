//! Structured logging for rename operations.

use crate::core::{RenameBatch, RenameStatus, RenamerError, RenamerResult};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use tracing::{debug, error, info};
use uuid::Uuid;

/// Log entry for a single rename operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameLogEntry {
    /// Unique identifier.
    pub id: Uuid,
    /// Timestamp of the operation.
    pub timestamp: DateTime<Local>,
    /// Batch this operation belongs to.
    pub batch_id: Uuid,
    /// Original path.
    pub original_path: PathBuf,
    /// New path.
    pub new_path: PathBuf,
    /// Whether it was a directory.
    pub is_directory: bool,
    /// Operation status.
    pub status: RenameStatus,
    /// Error message if failed.
    pub error: Option<String>,
}

/// Log manager for rename operations.
pub struct RenameLogger {
    /// Log file path.
    log_file: PathBuf,
    /// Whether logging is enabled.
    enabled: bool,
    /// Maximum log file size in bytes.
    max_size: u64,
    /// Number of rotated files to keep.
    max_rotations: u32,
}

impl RenameLogger {
    /// Create a new logger.
    pub fn new(log_file: PathBuf) -> Self {
        // Ensure log directory exists
        if let Some(parent) = log_file.parent() {
            let _ = fs::create_dir_all(parent);
        }

        Self {
            log_file,
            enabled: true,
            max_size: 10 * 1024 * 1024, // 10 MB
            max_rotations: 5,
        }
    }

    /// Enable or disable logging.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Set maximum log file size.
    pub fn set_max_size(&mut self, size: u64) {
        self.max_size = size;
    }

    /// Log a batch of rename operations.
    pub fn log_batch(&self, batch: &RenameBatch, statuses: &[(Uuid, RenameStatus, Option<String>)]) -> RenamerResult<()> {
        if !self.enabled {
            return Ok(());
        }

        self.rotate_if_needed()?;

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_file)
            .map_err(|e| RenamerError::Io(e))?;

        let mut writer = BufWriter::new(file);

        // Write batch header
        writeln!(
            writer,
            "\n=== Rename Batch: {} ===",
            batch.timestamp.format("%Y-%m-%d %H:%M:%S")
        )
        .map_err(|e| RenamerError::Io(e))?;

        writeln!(writer, "Batch ID: {}", batch.id).map_err(|e| RenamerError::Io(e))?;

        if let Some(ref desc) = batch.description {
            writeln!(writer, "Description: {}", desc).map_err(|e| RenamerError::Io(e))?;
        }

        writeln!(writer, "Total operations: {}", batch.records.len())
            .map_err(|e| RenamerError::Io(e))?;
        writeln!(writer, "---").map_err(|e| RenamerError::Io(e))?;

        // Write each record
        for record in &batch.records {
            let (status, error_msg) = statuses
                .iter()
                .find(|(id, _, _)| *id == record.id)
                .map(|(_, s, e)| (*s, e.clone()))
                .unwrap_or((RenameStatus::Completed, None));

            let status_str = match status {
                RenameStatus::Completed => "OK",
                RenameStatus::Failed => "FAILED",
                RenameStatus::Skipped => "SKIPPED",
                _ => "UNKNOWN",
            };

            writeln!(
                writer,
                "[{}] {} -> {}",
                status_str,
                record.original_path.display(),
                record.new_path.display()
            )
            .map_err(|e| RenamerError::Io(e))?;

            if let Some(ref err) = error_msg {
                writeln!(writer, "    Error: {}", err).map_err(|e| RenamerError::Io(e))?;
            }

            // Also log using tracing
            match status {
                RenameStatus::Completed => {
                    info!(
                        original = %record.original_path.display(),
                        new = %record.new_path.display(),
                        "File renamed"
                    );
                }
                RenameStatus::Failed => {
                    error!(
                        original = %record.original_path.display(),
                        new = %record.new_path.display(),
                        error = ?error_msg,
                        "Rename failed"
                    );
                }
                _ => {
                    debug!(
                        original = %record.original_path.display(),
                        status = ?status,
                        "Rename skipped"
                    );
                }
            }
        }

        writeln!(writer, "=== End Batch ===\n").map_err(|e| RenamerError::Io(e))?;
        writer.flush().map_err(|e| RenamerError::Io(e))?;

        Ok(())
    }

    /// Log a single rename entry.
    pub fn log_entry(&self, entry: &RenameLogEntry) -> RenamerResult<()> {
        if !self.enabled {
            return Ok(());
        }

        self.rotate_if_needed()?;

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_file)
            .map_err(|e| RenamerError::Io(e))?;

        let mut writer = BufWriter::new(file);

        let json = serde_json::to_string(entry).map_err(|e| RenamerError::JsonError(e))?;
        writeln!(writer, "{}", json).map_err(|e| RenamerError::Io(e))?;

        Ok(())
    }

    /// Write a log entry in JSON Lines format.
    pub fn log_jsonl(&self, entry: &RenameLogEntry) -> RenamerResult<()> {
        if !self.enabled {
            return Ok(());
        }

        let jsonl_path = self.log_file.with_extension("jsonl");
        self.rotate_if_needed_path(&jsonl_path)?;

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&jsonl_path)
            .map_err(|e| RenamerError::Io(e))?;

        let mut writer = BufWriter::new(file);
        let json = serde_json::to_string(entry).map_err(|e| RenamerError::JsonError(e))?;
        writeln!(writer, "{}", json).map_err(|e| RenamerError::Io(e))?;

        Ok(())
    }

    /// Read log entries from the log file.
    pub fn read_entries(&self) -> RenamerResult<Vec<RenameLogEntry>> {
        let jsonl_path = self.log_file.with_extension("jsonl");
        
        if !jsonl_path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&jsonl_path).map_err(|e| RenamerError::Io(e))?;
        let reader = BufReader::new(file);
        let mut entries = Vec::new();

        for line in reader.lines() {
            let line = line.map_err(|e| RenamerError::Io(e))?;
            if let Ok(entry) = serde_json::from_str::<RenameLogEntry>(&line) {
                entries.push(entry);
            }
        }

        Ok(entries)
    }

    /// Rotate log file if it exceeds maximum size.
    fn rotate_if_needed(&self) -> RenamerResult<()> {
        self.rotate_if_needed_path(&self.log_file)
    }

    fn rotate_if_needed_path(&self, path: &Path) -> RenamerResult<()> {
        if !path.exists() {
            return Ok(());
        }

        let metadata = fs::metadata(path).map_err(|e| RenamerError::Io(e))?;

        if metadata.len() < self.max_size {
            return Ok(());
        }

        // Rotate existing log files
        for i in (1..self.max_rotations).rev() {
            let old_path = Self::rotated_path(path, i);
            let new_path = Self::rotated_path(path, i + 1);

            if old_path.exists() {
                if i + 1 >= self.max_rotations {
                    fs::remove_file(&old_path).ok();
                } else {
                    fs::rename(&old_path, &new_path).ok();
                }
            }
        }

        // Rotate current file to .1
        let rotated = Self::rotated_path(path, 1);
        fs::rename(path, &rotated).map_err(|e| RenamerError::Io(e))?;

        info!(path = %path.display(), "Log file rotated");

        Ok(())
    }

    /// Get the path for a rotated log file.
    fn rotated_path(base: &Path, n: u32) -> PathBuf {
        let name = base.file_name().unwrap_or_default().to_string_lossy();
        base.with_file_name(format!("{}.{}", name, n))
    }

    /// Get log file path.
    pub fn log_file_path(&self) -> &Path {
        &self.log_file
    }

    /// Clear all log files.
    pub fn clear(&self) -> RenamerResult<()> {
        if self.log_file.exists() {
            fs::remove_file(&self.log_file).map_err(|e| RenamerError::Io(e))?;
        }

        let jsonl_path = self.log_file.with_extension("jsonl");
        if jsonl_path.exists() {
            fs::remove_file(&jsonl_path).map_err(|e| RenamerError::Io(e))?;
        }

        // Remove rotated files
        for i in 1..=self.max_rotations {
            let rotated = Self::rotated_path(&self.log_file, i);
            if rotated.exists() {
                fs::remove_file(&rotated).ok();
            }

            let rotated_jsonl = Self::rotated_path(&jsonl_path, i);
            if rotated_jsonl.exists() {
                fs::remove_file(&rotated_jsonl).ok();
            }
        }

        Ok(())
    }

    /// Export log to CSV.
    pub fn export_csv(&self, output_path: &Path) -> RenamerResult<()> {
        let entries = self.read_entries()?;

        let file = File::create(output_path).map_err(|e| RenamerError::Io(e))?;
        let mut writer = csv::Writer::from_writer(file);

        // Write header
        writer
            .write_record(&[
                "timestamp",
                "batch_id",
                "original_path",
                "new_path",
                "is_directory",
                "status",
                "error",
            ])
            .map_err(|e| RenamerError::CsvError(e))?;

        // Write entries
        for entry in entries {
            writer
                .write_record(&[
                    entry.timestamp.to_rfc3339(),
                    entry.batch_id.to_string(),
                    entry.original_path.to_string_lossy().to_string(),
                    entry.new_path.to_string_lossy().to_string(),
                    entry.is_directory.to_string(),
                    format!("{:?}", entry.status),
                    entry.error.unwrap_or_default(),
                ])
                .map_err(|e| RenamerError::CsvError(e))?;
        }

        writer.flush().map_err(|e| RenamerError::Io(e))?;

        Ok(())
    }
}

impl Default for RenameLogger {
    fn default() -> Self {
        let log_file = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("bulk-renamer")
            .join("logs")
            .join("rename.log");

        Self::new(log_file)
    }
}

/// Initialize the tracing subscriber for application-wide logging.
pub fn init_tracing(log_level: &str) {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(log_level));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .init();
}
