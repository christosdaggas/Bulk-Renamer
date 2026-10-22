//! Preset management system.

use crate::core::{RenameConfig, RenamerError, RenamerResult};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Current schema version for preset files.
///
/// Bump this when the on-disk format changes in a way that requires migration.
pub const PRESET_SCHEMA_VERSION: u32 = 1;

fn default_preset_version() -> u32 {
    PRESET_SCHEMA_VERSION
}

/// A saved preset containing a rename configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preset {
    /// Schema version of the preset file (for future migrations).
    #[serde(default = "default_preset_version")]
    pub version: u32,
    /// Unique identifier.
    pub id: Uuid,
    /// Display name.
    pub name: String,
    /// Description.
    pub description: Option<String>,
    /// The rename configuration.
    pub config: RenameConfig,
    /// When the preset was created.
    pub created: DateTime<Local>,
    /// When the preset was last modified.
    pub modified: DateTime<Local>,
    /// Tags for organization.
    pub tags: Vec<String>,
    /// Whether this is a built-in preset.
    pub builtin: bool,
}

impl Default for Preset {
    fn default() -> Self {
        let now = Local::now();
        Self {
            version: PRESET_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            name: String::from("Untitled"),
            description: None,
            config: RenameConfig::default(),
            created: now,
            modified: now,
            tags: Vec::new(),
            builtin: false,
        }
    }
}

impl Preset {
    /// Create a new preset from a config.
    pub fn new(name: &str, config: RenameConfig) -> Self {
        Self {
            name: name.to_string(),
            config,
            ..Self::default()
        }
    }

    /// Create with description.
    pub fn with_description(mut self, description: &str) -> Self {
        self.description = Some(description.to_string());
        self
    }

    /// Add tags.
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }
}

/// Manager for presets.
pub struct PresetManager {
    /// Directory where presets are stored.
    presets_dir: PathBuf,
    /// Loaded presets.
    presets: HashMap<Uuid, Preset>,
}

impl PresetManager {
    /// Create a new preset manager.
    pub fn new(presets_dir: PathBuf) -> Self {
        // Ensure directory exists
        let _ = fs::create_dir_all(&presets_dir);

        let mut manager = Self {
            presets_dir,
            presets: HashMap::new(),
        };

        // Load presets and add built-in ones
        let _ = manager.load_all();
        manager.ensure_builtin_presets();

        manager
    }

    /// Load all presets from disk.
    pub fn load_all(&mut self) -> RenamerResult<()> {
        if !self.presets_dir.exists() {
            return Ok(());
        }

        for entry in fs::read_dir(&self.presets_dir).map_err(|e| RenamerError::Io(e))? {
            let entry = entry.map_err(|e| RenamerError::Io(e))?;
            let path = entry.path();

            if path.extension().map(|e| e == "json").unwrap_or(false) {
                match self.load_preset_from_file(&path) {
                    Ok(preset) => {
                        self.presets.insert(preset.id, preset);
                    }
                    Err(e) => {
                        // Never delete or overwrite an unreadable preset file:
                        // keep it on disk so the user can recover it manually.
                        tracing::warn!(
                            path = %path.display(),
                            error = %e,
                            "Failed to load preset file; skipping it but keeping it on disk"
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Load a single preset from a file.
    fn load_preset_from_file(&self, path: &Path) -> RenamerResult<Preset> {
        let file = File::open(path).map_err(|e| RenamerError::Io(e))?;
        let reader = BufReader::new(file);
        let preset: Preset =
            serde_json::from_reader(reader).map_err(|e| RenamerError::JsonError(e))?;
        Ok(preset)
    }

    /// Save a preset to disk.
    pub fn save_preset(&mut self, preset: &Preset) -> RenamerResult<()> {
        let path = self.presets_dir.join(format!("{}.json", preset.id));
        write_json_atomic(&path, preset)?;

        // Update in-memory cache
        self.presets.insert(preset.id, preset.clone());

        Ok(())
    }

    /// Add a new preset.
    pub fn add_preset(&mut self, preset: Preset) -> RenamerResult<()> {
        self.save_preset(&preset)
    }

    /// Update an existing preset.
    pub fn update_preset(&mut self, mut preset: Preset) -> RenamerResult<()> {
        preset.modified = Local::now();
        self.save_preset(&preset)
    }

    /// Delete a preset.
    pub fn delete_preset(&mut self, id: &Uuid) -> RenamerResult<()> {
        // Don't delete built-in presets
        if let Some(preset) = self.presets.get(id) {
            if preset.builtin {
                return Err(RenamerError::Internal(
                    "Cannot delete built-in preset".to_string(),
                ));
            }
        }

        let path = self.presets_dir.join(format!("{}.json", id));
        if path.exists() {
            fs::remove_file(&path).map_err(|e| RenamerError::Io(e))?;
        }

        self.presets.remove(id);
        Ok(())
    }

    /// Get a preset by ID.
    pub fn get_preset(&self, id: &Uuid) -> Option<&Preset> {
        self.presets.get(id)
    }

    /// Get a preset by name.
    ///
    /// Names are not unique, and a HashMap has no stable order, so the choice is made
    /// explicit: a user preset beats a built-in of the same name, the most recently
    /// modified of several wins, and the id breaks an exact tie.
    pub fn get_preset_by_name(&self, name: &str) -> Option<&Preset> {
        self.presets
            .values()
            .filter(|p| p.name == name)
            .max_by(|a, b| (!a.builtin, a.modified, a.id).cmp(&(!b.builtin, b.modified, b.id)))
    }

    /// The user preset (never a built-in) that `get_preset_by_name` would pick.
    pub fn get_user_preset_by_name(&self, name: &str) -> Option<&Preset> {
        self.get_preset_by_name(name).filter(|preset| !preset.builtin)
    }

    /// Get all presets.
    pub fn get_all(&self) -> Vec<&Preset> {
        let mut presets: Vec<_> = self.presets.values().collect();
        presets.sort_by(|a, b| a.name.cmp(&b.name));
        presets
    }

    /// Get presets by tag.
    pub fn get_by_tag(&self, tag: &str) -> Vec<&Preset> {
        self.presets
            .values()
            .filter(|p| p.tags.contains(&tag.to_string()))
            .collect()
    }

    /// Search presets by name or description.
    pub fn search(&self, query: &str) -> Vec<&Preset> {
        let query_lower = query.to_lowercase();
        self.presets
            .values()
            .filter(|p| {
                p.name.to_lowercase().contains(&query_lower)
                    || p.description
                        .as_ref()
                        .map(|d| d.to_lowercase().contains(&query_lower))
                        .unwrap_or(false)
            })
            .collect()
    }

    /// Export a preset to a file.
    pub fn export_preset(&self, id: &Uuid, path: &Path) -> RenamerResult<()> {
        let preset = self
            .presets
            .get(id)
            .ok_or(RenamerError::PresetNotFound { name: id.to_string() })?;

        write_json_atomic(path, preset)
    }

    /// Import a preset from a file.
    pub fn import_preset(&mut self, path: &Path) -> RenamerResult<Preset> {
        let mut preset = self.load_preset_from_file(path)?;

        // Generate new ID to avoid conflicts
        preset.id = Uuid::new_v4();
        preset.builtin = false;

        // Check for name conflicts
        let original_name = preset.name.clone();
        let mut counter = 1;
        while self.get_preset_by_name(&preset.name).is_some() {
            preset.name = format!("{} ({})", original_name, counter);
            counter += 1;
        }

        self.save_preset(&preset)?;
        Ok(preset)
    }

    /// Ensure built-in presets exist.
    fn ensure_builtin_presets(&mut self) {
        let builtin_presets = create_builtin_presets();

        for preset in builtin_presets {
            if self.get_preset_by_name(&preset.name).is_none() {
                let _ = self.save_preset(&preset);
            }
        }
    }

    /// Get all tags used across presets.
    pub fn get_all_tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = self
            .presets
            .values()
            .flat_map(|p| p.tags.clone())
            .collect();

        tags.sort();
        tags.dedup();
        tags
    }

    /// Duplicate a preset.
    pub fn duplicate_preset(&mut self, id: &Uuid) -> RenamerResult<Preset> {
        let original = self
            .presets
            .get(id)
            .ok_or(RenamerError::PresetNotFound { name: id.to_string() })?
            .clone();

        let mut new_preset = original;
        new_preset.id = Uuid::new_v4();
        new_preset.name = format!("{} (Copy)", new_preset.name);
        new_preset.builtin = false;
        new_preset.created = Local::now();
        new_preset.modified = Local::now();

        self.save_preset(&new_preset)?;
        Ok(new_preset)
    }
}

impl Default for PresetManager {
    fn default() -> Self {
        let presets_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("bulk-renamer")
            .join("presets");

        Self::new(presets_dir)
    }
}

/// Atomically write a value as pretty-printed JSON to `path`.
///
/// The data is first written to a temporary file in the same directory and
/// then renamed over the target, so a crash mid-write can never leave a
/// truncated or corrupt file at `path`. The temp file uses a non-`.json`
/// extension so preset loading never picks up leftovers.
fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> RenamerResult<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("preset.json");
    let tmp_path = dir.join(format!("{}.tmp-{}", file_name, Uuid::new_v4().simple()));

    let result = (|| {
        let file = File::create(&tmp_path).map_err(RenamerError::Io)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, value).map_err(RenamerError::JsonError)?;
        writer.flush().map_err(RenamerError::Io)?;
        // Make sure the bytes hit the disk before the rename makes them visible.
        writer.get_ref().sync_all().map_err(RenamerError::Io)?;
        fs::rename(&tmp_path, path).map_err(RenamerError::Io)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }

    result
}

/// Create built-in presets.
fn create_builtin_presets() -> Vec<Preset> {
    use crate::core::*;

    let mut presets = Vec::new();

    // 1. Lowercase all
    let mut preset = Preset::new(
        "Lowercase All",
        RenameConfig {
            id: Uuid::new_v4(),
            name: "Lowercase All".to_string(),
            rules: vec![RenameRule::new(RuleType::ChangeCase(CaseRule {
                case_type: CaseType::Lower,
                include_extension: true,
            }))],
            separate_extension: false,
            filter: None,
        },
    );
    preset.description = Some("Convert all filenames to lowercase".to_string());
    preset.tags = vec!["case".to_string(), "simple".to_string()];
    preset.builtin = true;
    presets.push(preset);

    // 2. Title Case
    let mut preset = Preset::new(
        "Title Case",
        RenameConfig {
            id: Uuid::new_v4(),
            name: "Title Case".to_string(),
            rules: vec![RenameRule::new(RuleType::ChangeCase(CaseRule {
                case_type: CaseType::Title,
                include_extension: false,
            }))],
            separate_extension: true,
            filter: None,
        },
    );
    preset.description = Some("Convert filenames to Title Case".to_string());
    preset.tags = vec!["case".to_string(), "simple".to_string()];
    preset.builtin = true;
    presets.push(preset);

    // 3. Replace Spaces with Underscores
    let mut preset = Preset::new(
        "Spaces to Underscores",
        RenameConfig {
            id: Uuid::new_v4(),
            name: "Spaces to Underscores".to_string(),
            rules: vec![RenameRule::new(RuleType::Replace(ReplaceRule {
                find: " ".to_string(),
                replace: "_".to_string(),
                use_regex: false,
                case_sensitive: true,
                replace_all: true,
                include_extension: false,
            }))],
            separate_extension: true,
            filter: None,
        },
    );
    preset.description = Some("Replace all spaces with underscores".to_string());
    preset.tags = vec!["cleanup".to_string(), "simple".to_string()];
    preset.builtin = true;
    presets.push(preset);

    // 4. Add Sequential Numbers
    let mut preset = Preset::new(
        "Add Sequential Numbers",
        RenameConfig {
            id: Uuid::new_v4(),
            name: "Add Sequential Numbers".to_string(),
            rules: vec![RenameRule::new(RuleType::Numbering(NumberingRule {
                start: 1,
                increment: 1,
                padding: 3,
                position: InsertPosition::Prefix,
                prefix: String::new(),
                suffix: "_".to_string(),
                reset_per_folder: false,
                format: NumberFormat::Decimal,
            }))],
            separate_extension: true,
            filter: None,
        },
    );
    preset.description = Some("Add sequential numbers at the beginning (001_, 002_, ...)".to_string());
    preset.tags = vec!["numbering".to_string()];
    preset.builtin = true;
    presets.push(preset);

    // 5. Date Prefix (Modified Date)
    let mut preset = Preset::new(
        "Date Prefix (Modified)",
        RenameConfig {
            id: Uuid::new_v4(),
            name: "Date Prefix (Modified)".to_string(),
            rules: vec![RenameRule::new(RuleType::DateTime(DateTimeRule {
                source: DateSource::Modified,
                format: "%Y-%m-%d_".to_string(),
                position: InsertPosition::Prefix,
            }))],
            separate_extension: true,
            filter: None,
        },
    );
    preset.description = Some("Add file modification date as prefix (YYYY-MM-DD_)".to_string());
    preset.tags = vec!["date".to_string(), "organization".to_string()];
    preset.builtin = true;
    presets.push(preset);

    // 6. Remove Brackets
    let mut preset = Preset::new(
        "Remove Brackets",
        RenameConfig {
            id: Uuid::new_v4(),
            name: "Remove Brackets".to_string(),
            rules: vec![RenameRule::new(RuleType::Remove(RemoveRule {
                target: RemoveTarget::Bracketed(BracketType::All),
            }))],
            separate_extension: true,
            filter: None,
        },
    );
    preset.description = Some("Remove all bracketed content ((), [], {}, <>)".to_string());
    preset.tags = vec!["cleanup".to_string()];
    preset.builtin = true;
    presets.push(preset);

    // 7. Clean Filename
    let mut preset = Preset::new(
        "Clean Filename",
        RenameConfig {
            id: Uuid::new_v4(),
            name: "Clean Filename".to_string(),
            rules: vec![RenameRule::new(RuleType::Cleanup(CleanupRule {
                collapse_spaces: true,
                remove_special: true,
                preserve: "-_".to_string(),
                space_replacement: None,
                remove_diacritics: true,
                normalize_unicode: true,
            }))],
            separate_extension: true,
            filter: None,
        },
    );
    preset.description = Some("Clean up filename: remove special chars, diacritics, extra spaces".to_string());
    preset.tags = vec!["cleanup".to_string()];
    preset.builtin = true;
    presets.push(preset);

    // 8. Photo Rename (EXIF Date)
    let mut preset = Preset::new(
        "Photo Rename (EXIF Date)",
        RenameConfig {
            id: Uuid::new_v4(),
            name: "Photo Rename (EXIF Date)".to_string(),
            rules: vec![RenameRule::new(RuleType::Expression(ExpressionRule {
                expression: "${filedate('exif', '%Y%m%d_%H%M%S')}_${camera}".to_string(),
            }))],
            separate_extension: true,
            filter: None,
        },
    );
    preset.description = Some("Rename photos using EXIF date and camera model".to_string());
    preset.tags = vec!["photo".to_string(), "exif".to_string()];
    preset.builtin = true;
    presets.push(preset);

    // 9. Music Rename (ID3 Tags)
    let mut preset = Preset::new(
        "Music Rename (ID3)",
        RenameConfig {
            id: Uuid::new_v4(),
            name: "Music Rename (ID3)".to_string(),
            rules: vec![RenameRule::new(RuleType::Expression(ExpressionRule {
                expression: "${num(track, 2)} - ${artist} - ${title}".to_string(),
            }))],
            separate_extension: true,
            filter: None,
        },
    );
    preset.description = Some("Rename music files using ID3 tags: Track - Artist - Title".to_string());
    preset.tags = vec!["music".to_string(), "id3".to_string()];
    preset.builtin = true;
    presets.push(preset);

    // 10. Snake Case
    let mut preset = Preset::new(
        "Snake Case",
        RenameConfig {
            id: Uuid::new_v4(),
            name: "Snake Case".to_string(),
            rules: vec![RenameRule::new(RuleType::ChangeCase(CaseRule {
                case_type: CaseType::Snake,
                include_extension: false,
            }))],
            separate_extension: true,
            filter: None,
        },
    );
    preset.description = Some("Convert filenames to snake_case".to_string());
    preset.tags = vec!["case".to_string(), "developer".to_string()];
    preset.builtin = true;
    presets.push(preset);

    presets
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a unique, empty directory for a test.
    fn test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bulk-renamer-preset-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn temp_files_in(dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.contains(".tmp-"))
                    .unwrap_or(false)
            })
            .collect()
    }

    #[test]
    fn atomic_save_writes_valid_file_and_leaves_no_temp_files() {
        let dir = test_dir();
        let mut manager = PresetManager::new(dir.clone());

        let preset = Preset::new("Atomic Test", RenameConfig::default());
        manager.save_preset(&preset).unwrap();
        // Save again to exercise the overwrite path (rename over existing target).
        manager.save_preset(&preset).unwrap();

        let target = dir.join(format!("{}.json", preset.id));
        assert!(target.exists(), "target preset file must exist after save");

        // The saved file must be complete, valid JSON.
        let content = fs::read_to_string(&target).unwrap();
        let loaded: Preset = serde_json::from_str(&content).unwrap();
        assert_eq!(loaded.id, preset.id);
        assert_eq!(loaded.name, "Atomic Test");
        assert_eq!(loaded.version, PRESET_SCHEMA_VERSION);

        // No temporary files may be left behind.
        let leftovers = temp_files_in(&dir);
        assert!(leftovers.is_empty(), "leftover temp files: {:?}", leftovers);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_preset_file_is_kept_and_others_still_load() {
        let dir = test_dir();

        // Save a valid user preset with a first manager.
        let mut manager = PresetManager::new(dir.clone());
        let preset = Preset::new("Survivor", RenameConfig::default());
        manager.save_preset(&preset).unwrap();
        drop(manager);

        // Plant a corrupt preset file.
        let corrupt_path = dir.join("corrupt.json");
        let corrupt_content = "{ this is definitely not valid json";
        fs::write(&corrupt_path, corrupt_content).unwrap();

        // Reload everything with a fresh manager.
        let manager = PresetManager::new(dir.clone());

        // The corrupt file must survive on disk, byte for byte.
        assert!(corrupt_path.exists(), "corrupt preset file must not be deleted");
        assert_eq!(fs::read_to_string(&corrupt_path).unwrap(), corrupt_content);

        // Valid presets must still load.
        assert!(manager.get_preset_by_name("Survivor").is_some());
        assert!(manager.get_preset(&preset.id).is_some());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pre_versioning_preset_json_still_deserializes() {
        // Hand-written JSON as written by builds that predate the `version` field.
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "name": "Legacy Preset",
            "description": "Saved before schema versioning",
            "config": {
                "id": "550e8400-e29b-41d4-a716-446655440001",
                "name": "Legacy Config",
                "rules": [
                    {
                        "id": "550e8400-e29b-41d4-a716-446655440002",
                        "enabled": true,
                        "rule_type": {
                            "type": "Replace",
                            "find": " ",
                            "replace": "_",
                            "use_regex": false,
                            "case_sensitive": true,
                            "replace_all": true,
                            "include_extension": false
                        }
                    }
                ],
                "separate_extension": true,
                "filter": null
            },
            "created": "2024-03-01T12:00:00+02:00",
            "modified": "2024-03-01T12:00:00+02:00",
            "tags": ["legacy"],
            "builtin": false
        }"#;

        let preset: Preset = serde_json::from_str(json).unwrap();
        assert_eq!(preset.version, PRESET_SCHEMA_VERSION);
        assert_eq!(preset.name, "Legacy Preset");
        assert_eq!(preset.config.name, "Legacy Config");
        assert_eq!(preset.config.rules.len(), 1);
        assert_eq!(preset.tags, vec!["legacy".to_string()]);
        assert!(!preset.builtin);
    }

    #[test]
    fn rule_missing_id_and_enabled_gets_defaults() {
        // A rule written without `id`/`enabled` (e.g. by a future or trimmed
        // format) must still load, with `enabled` defaulting to true.
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440010",
            "name": "Tolerant Config",
            "rules": [
                {
                    "rule_type": {
                        "type": "Expression",
                        "expression": "${name}"
                    }
                }
            ],
            "separate_extension": true,
            "filter": null
        }"#;

        let config: RenameConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.rules.len(), 1);
        assert!(config.rules[0].enabled);
    }

    /// Lookups went through HashMap order, which is random per map, so a user preset
    /// sharing a built-in's name was found or not depending on the process.
    #[test]
    fn a_user_preset_beats_a_builtin_of_the_same_name() {
        let dir = test_dir();
        let mut manager = PresetManager::new(dir.clone());
        let user = Preset::new("Lowercase All", RenameConfig::default());
        manager.add_preset(user.clone()).expect("save preset");

        for _ in 0..20 {
            let fresh = PresetManager::new(dir.clone());
            let found = fresh.get_preset_by_name("Lowercase All").expect("found");
            assert_eq!(found.id, user.id, "the built-in won over the user's preset");
        }

        let _ = fs::remove_dir_all(&dir);
    }

    /// Saving a preset twice under one name is how the dialog used to "update" it, and
    /// `apply --preset` then picked whichever copy the hash order produced.
    #[test]
    fn the_newest_of_duplicate_presets_wins() {
        let dir = test_dir();
        let mut manager = PresetManager::new(dir.clone());
        let base = Local::now();
        let mut newest = None;
        for offset in [5, 1, 9, 3, 7, 2] {
            let mut preset = Preset::new("Photos", RenameConfig::default());
            preset.modified = base + chrono::Duration::seconds(offset);
            if offset == 9 {
                newest = Some(preset.id);
            }
            manager.add_preset(preset).expect("save preset");
        }

        for _ in 0..20 {
            let fresh = PresetManager::new(dir.clone());
            assert_eq!(fresh.get_preset_by_name("Photos").map(|p| p.id), newest);
        }

        let _ = fs::remove_dir_all(&dir);
    }
}
