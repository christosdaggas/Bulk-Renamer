//! Headless command-line mode.
//!
//! This module handles the `preview`, `apply` and `list-presets` subcommands
//! (plus top-level `--help`/`--version`) entirely without GTK. Anything else —
//! no arguments, or plain file/directory arguments — is left untouched so the
//! GUI keeps its `HANDLES_OPEN` behavior.
//!
//! Argument parsing is done by hand on `std::env::args_os`, so a path that is not
//! UTF-8 reaches the program intact; no CLI crate is used.

use crate::core::{FileEntry, RenameConfig, RenamePreview, RenameStatus, ValidationErrorType};
use crate::engine::{execute_renames_with, RenameEngine, RenameValidator};
use crate::presets::PresetManager;
use crate::undo::UndoManager;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use uuid::Uuid;

/// Exit code for success.
const EXIT_OK: i32 = 0;
/// Exit code for an aborted apply or partially failed renames.
const EXIT_FAILURE: i32 = 1;
/// Exit code for usage errors, unknown presets, and plans with conflicts/errors.
const EXIT_BLOCKED: i32 = 2;

/// A parsed CLI command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help,
    Version,
    ListPresets,
    Preview(JobArgs),
    Apply(JobArgs),
}

/// Arguments shared by `preview` and `apply`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobArgs {
    /// Name of the preset to use.
    pub preset: String,
    /// Files and directories to operate on, in the order given.
    pub paths: Vec<PathBuf>,
    /// Walk directories to unlimited depth instead of one level.
    pub recursive: bool,
    /// Include hidden files (dotfiles).
    pub hidden: bool,
    /// Skip the confirmation prompt (`apply` only).
    pub yes: bool,
}

/// Outcome of parsing a subcommand's own arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
enum JobParse {
    /// `-h`/`--help` was given inside the subcommand.
    Help,
    Job(JobArgs),
}

/// Entry point called from `main` before any GTK initialization.
///
/// `args` are the process arguments without the program name. Returns
/// `Some(exit_code)` when the invocation was handled in CLI mode, and `None`
/// when the GUI should launch exactly as before.
pub fn run(args: Vec<OsString>) -> Option<i32> {
    match parse(&args) {
        Ok(Some(command)) => Some(execute(command)),
        Ok(None) => None,
        Err(message) => {
            eprintln!("bulk-renamer: {}", message);
            eprintln!("Run 'bulk-renamer --help' for usage.");
            Some(EXIT_BLOCKED)
        }
    }
}

/// Decide whether the arguments select CLI mode, and parse them if so.
///
/// `Ok(None)` means "not a CLI invocation": no arguments, or plain
/// file/directory arguments (including GTK/GLib options), which must keep
/// launching the GUI. A first argument that is not UTF-8 cannot be a command,
/// so it is a path for the GUI too.
pub fn parse(args: &[OsString]) -> Result<Option<Command>, String> {
    let Some(first) = args.first().and_then(|first| first.to_str()) else {
        return Ok(None);
    };

    match first {
        "-h" | "--help" => Ok(Some(Command::Help)),
        "-V" | "--version" => Ok(Some(Command::Version)),
        "list-presets" => match args.get(1).map(|arg| arg.to_string_lossy()) {
            None => Ok(Some(Command::ListPresets)),
            Some(arg) if arg == "-h" || arg == "--help" => Ok(Some(Command::Help)),
            Some(other) => Err(format!("unexpected argument '{}' for list-presets", other)),
        },
        "preview" => Ok(Some(match parse_job("preview", &args[1..], false)? {
            JobParse::Help => Command::Help,
            JobParse::Job(job) => Command::Preview(job),
        })),
        "apply" => Ok(Some(match parse_job("apply", &args[1..], true)? {
            JobParse::Help => Command::Help,
            JobParse::Job(job) => Command::Apply(job),
        })),
        // Anything else is GUI territory: file/directory arguments for
        // HANDLES_OPEN, or GTK/GLib options the application handles itself.
        _ => Ok(None),
    }
}

/// Parse the arguments of `preview` or `apply`.
fn parse_job(command: &str, args: &[OsString], allow_yes: bool) -> Result<JobParse, String> {
    let mut preset: Option<String> = None;
    let mut paths = Vec::new();
    let mut recursive = false;
    let mut hidden = false;
    let mut yes = false;
    let mut only_paths = false;

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if only_paths {
            paths.push(PathBuf::from(arg));
            continue;
        }

        // Options are ASCII, so the lossy text recognises them; paths keep their bytes.
        let text = arg.to_string_lossy();
        match text.as_ref() {
            "--" => only_paths = true,
            "-h" | "--help" => return Ok(JobParse::Help),
            "--preset" => {
                let value = iter
                    .next()
                    .ok_or_else(|| format!("{}: --preset requires a value", command))?;
                preset = Some(preset_name(command, value)?);
            }
            other if other.starts_with("--preset=") => {
                preset = Some(preset_name(command, arg)?["--preset=".len()..].to_string());
            }
            "-r" | "--recursive" => recursive = true,
            "--hidden" => hidden = true,
            "-y" | "--yes" if allow_yes => yes = true,
            other if other.starts_with('-') && other.len() > 1 => {
                return Err(format!("{}: unknown option '{}'", command, other));
            }
            _ => paths.push(PathBuf::from(arg)),
        }
    }

    let preset = preset
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("{}: missing required option --preset <NAME>", command))?;

    if paths.is_empty() {
        return Err(format!(
            "{}: at least one file or directory path is required",
            command
        ));
    }

    Ok(JobParse::Job(JobArgs {
        preset,
        paths,
        recursive,
        hidden,
        yes,
    }))
}

/// A `--preset` argument as text: preset names are text, so bytes that are not
/// UTF-8 can never name one.
fn preset_name(command: &str, value: &OsString) -> Result<String, String> {
    value
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| format!("{}: the --preset value is not valid UTF-8", command))
}

/// Run a parsed command and return its exit code.
fn execute(command: Command) -> i32 {
    match command {
        Command::Help => {
            print!("{}", usage());
            EXIT_OK
        }
        Command::Version => {
            println!("bulk-renamer {}", env!("CARGO_PKG_VERSION"));
            EXIT_OK
        }
        Command::ListPresets => list_presets(&PresetManager::default()),
        Command::Preview(job) => preview_with(&job, &PresetManager::default()),
        Command::Apply(job) => apply_with(
            &job,
            &PresetManager::default(),
            &mut UndoManager::default(),
        ),
    }
}

fn usage() -> String {
    format!(
        "Bulk Renamer {}\n\
         Batch file renaming for GNOME, with a headless command-line mode.\n\
         \n\
         USAGE:\n\
         \x20   bulk-renamer [FILE|DIRECTORY...]          Launch the graphical application\n\
         \x20   bulk-renamer <COMMAND> [OPTIONS] <PATH>...\n\
         \n\
         COMMANDS:\n\
         \x20   list-presets                        List available presets\n\
         \x20   preview --preset <NAME> <PATH>...   Show what a preset would rename\n\
         \x20   apply   --preset <NAME> <PATH>...   Apply a preset to files on disk\n\
         \n\
         OPTIONS (preview and apply):\n\
         \x20   --preset <NAME>    Preset to use (see list-presets)\n\
         \x20   -r, --recursive    Recurse into subdirectories (default: one level)\n\
         \x20   --hidden           Include hidden files (dotfiles)\n\
         \x20   -y, --yes          apply only: skip the confirmation prompt\n\
         \x20   --                 Treat every following argument as a path\n\
         \n\
         \x20   -h, --help         Print this help\n\
         \x20   -V, --version      Print the version\n\
         \n\
         EXIT CODES:\n\
         \x20   0   success\n\
         \x20   1   apply aborted at the prompt, or some files failed to rename\n\
         \x20   2   invalid usage, unknown preset, or the plan has conflicts/errors\n",
        env!("CARGO_PKG_VERSION")
    )
}

/// `list-presets`: print name, description and origin of every preset.
fn list_presets(presets: &PresetManager) -> i32 {
    let all = presets.get_all();
    if all.is_empty() {
        println!("No presets found.");
        return EXIT_OK;
    }

    let name_width = all
        .iter()
        .map(|preset| preset.name.chars().count())
        .max()
        .unwrap_or(0)
        .max("NAME".len());

    println!("{:<width$}  DESCRIPTION", "NAME", width = name_width);
    for preset in all {
        let description = preset.description.as_deref().unwrap_or("");
        let origin = if preset.builtin { "  [built-in]" } else { "" };
        println!(
            "{:<width$}  {}{}",
            preset.name,
            description,
            origin,
            width = name_width
        );
    }

    EXIT_OK
}

/// `preview`: print the rename plan, exit 0 when clean, 2 on conflicts/errors.
fn preview_with(job: &JobArgs, presets: &PresetManager) -> i32 {
    let config = match preset_config(presets, &job.preset) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("bulk-renamer: {}", message);
            return EXIT_BLOCKED;
        }
    };

    let entries = match collect_entries(job) {
        Ok(entries) => entries,
        Err(message) => {
            eprintln!("bulk-renamer: {}", message);
            return EXIT_BLOCKED;
        }
    };

    let outcome = build_previews(config, &entries);
    print_previews(&outcome.previews);

    if count_blockers(&outcome.previews) > 0 {
        EXIT_BLOCKED
    } else {
        EXIT_OK
    }
}

/// `apply`: preview, refuse on conflicts/errors, confirm, execute, record undo.
fn apply_with(job: &JobArgs, presets: &PresetManager, undo: &mut UndoManager) -> i32 {
    let config = match preset_config(presets, &job.preset) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("bulk-renamer: {}", message);
            return EXIT_BLOCKED;
        }
    };

    let entries = match collect_entries(job) {
        Ok(entries) => entries,
        Err(message) => {
            eprintln!("bulk-renamer: {}", message);
            return EXIT_BLOCKED;
        }
    };

    let outcome = build_previews(config, &entries);
    print_previews(&outcome.previews);

    let blockers = count_blockers(&outcome.previews);
    if blockers > 0 {
        eprintln!(
            "bulk-renamer: refusing to rename: the plan has {} conflict(s)/error(s)",
            blockers
        );
        return EXIT_BLOCKED;
    }

    let to_rename = outcome
        .previews
        .iter()
        .filter(|preview| matches!(preview.status, RenameStatus::WillRename))
        .count();
    if to_rename == 0 {
        println!("Nothing to rename.");
        return EXIT_OK;
    }

    if !job.yes {
        if !std::io::stdin().is_terminal() {
            eprintln!(
                "bulk-renamer: stdin is not a terminal, cannot ask for confirmation; \
                 pass --yes to rename without a prompt"
            );
            return EXIT_BLOCKED;
        }
        print!("Rename {} file(s)? [y/N] ", to_rename);
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        if std::io::stdin().read_line(&mut answer).is_err()
            || !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
        {
            println!("Aborted; nothing was renamed.");
            return EXIT_FAILURE;
        }
    }

    // A journal in the history directory (the GUI's, `undo::default_data_dir()`, for
    // the real manager) is what lets the next start of the app put files back if this
    // process is killed between the two phases, Ctrl+C included, and its lock tells a
    // running batch from an interrupted one.
    let journal_dir = undo.data_dir().to_path_buf();
    let result = match execute_renames_with(
        &outcome.previews,
        &outcome.files_by_id,
        Some(&journal_dir),
        |_, _| {},
        &AtomicBool::new(false),
    ) {
        Ok(result) => result,
        Err(err) => {
            eprintln!("bulk-renamer: rename failed: {}", err);
            return EXIT_BLOCKED;
        }
    };

    // Record the batch so the GUI can undo it later. A failed record must not
    // report the rename itself as failed: the files are already renamed. This
    // process ends here, so the disk copy is the only undo there is, and a write
    // that failed has to be said out loud.
    if let Some(batch) = result.batch.clone() {
        if let Err(err) = undo.record_batch_checked(batch) {
            eprintln!(
                "bulk-renamer: warning: could not record the batch for undo: {}",
                err
            );
        }
    }

    println!("Renamed {} file(s), {} failed.", result.success_count(), result.failure_count());
    for failure in &result.failures {
        eprintln!(
            "bulk-renamer: failed: '{}': {}",
            failure.target_path.display(),
            failure.error
        );
    }

    if result.failure_count() > 0 {
        EXIT_FAILURE
    } else {
        EXIT_OK
    }
}

/// Look up a preset by name and return its rename configuration.
fn preset_config(presets: &PresetManager, name: &str) -> Result<RenameConfig, String> {
    match presets.get_preset_by_name(name) {
        Some(preset) => Ok(preset.config.clone()),
        None => {
            let available = presets
                .get_all()
                .iter()
                .map(|preset| preset.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "no preset named '{}'; available presets: {}",
                name, available
            ))
        }
    }
}

/// Build the file list the same way the GUI's `add_paths` does: plain files
/// directly at depth 0, directories walked with walkdir, each entry created
/// with `FileEntry::from_path(path, depth)` and enriched via `load_metadata`.
fn collect_entries(job: &JobArgs) -> Result<Vec<FileEntry>, String> {
    let mut entries = Vec::new();

    for path in &job.paths {
        // Recorded paths go into the undo history, which the GUI reads from a
        // working directory of its own, so they must not depend on this one.
        let path = &absolute(path)?;
        if path.is_dir() {
            let max_depth = if job.recursive { usize::MAX } else { 1 };
            let show_hidden = job.hidden;
            let walker = walkdir::WalkDir::new(path)
                .min_depth(1)
                .max_depth(max_depth)
                .sort_by_file_name()
                .into_iter()
                .filter_entry(move |entry| {
                    show_hidden || !entry.file_name().to_string_lossy().starts_with('.')
                });

            for entry in walker.filter_map(|entry| entry.ok()) {
                // Mirror the GUI: an entry that cannot be stat'ed is skipped.
                if let Ok(mut file_entry) =
                    FileEntry::from_path(entry.path().to_path_buf(), entry.depth())
                {
                    let _ = crate::metadata::load_metadata(&mut file_entry);
                    entries.push(file_entry);
                }
            }
        } else if path.exists() {
            // A path named explicitly is never hidden-filtered, like in the GUI.
            let mut file_entry = FileEntry::from_path(path.clone(), 0)
                .map_err(|err| format!("cannot read '{}': {}", path.display(), err))?;
            let _ = crate::metadata::load_metadata(&mut file_entry);
            entries.push(file_entry);
        } else {
            return Err(format!("path '{}' does not exist", path.display()));
        }
    }

    if entries.is_empty() {
        return Err("no files found under the given paths".to_string());
    }

    Ok(entries)
}

/// `path` made absolute against the working directory, without resolving symlinks.
fn absolute(path: &Path) -> Result<PathBuf, String> {
    std::path::absolute(path).map_err(|err| format!("cannot resolve '{}': {}", path.display(), err))
}

/// Previews plus the file map the executor needs.
struct PreviewOutcome {
    previews: Vec<RenamePreview>,
    files_by_id: HashMap<Uuid, FileEntry>,
}

/// Run the same pipeline as the GUI's `update_preview`: generate previews,
/// validate the batch with file access checks, and fold validation errors
/// back into the preview statuses.
fn build_previews(config: RenameConfig, entries: &[FileEntry]) -> PreviewOutcome {
    let mut engine = RenameEngine::new(config);
    let mut previews = engine.generate_previews(entries);

    let files_by_id: HashMap<Uuid, FileEntry> = entries
        .iter()
        .map(|entry| (entry.id, entry.clone()))
        .collect();

    let validator = RenameValidator::new();
    for error in validator.validate_batch_with_files(&previews, &files_by_id) {
        if let Some(preview) = previews.get_mut(error.file_index) {
            preview.status = match error.error_type {
                ValidationErrorType::Conflict => RenameStatus::InternalConflict,
                _ => RenameStatus::Error,
            };
            preview.message = Some(error.message);
        }
    }

    PreviewOutcome {
        previews,
        files_by_id,
    }
}

/// Number of previews that block execution.
fn count_blockers(previews: &[RenamePreview]) -> usize {
    previews
        .iter()
        .filter(|preview| {
            matches!(
                preview.status,
                RenameStatus::Conflict
                    | RenameStatus::InternalConflict
                    | RenameStatus::Error
                    | RenameStatus::Failed
            )
        })
        .count()
}

/// One-word status column value for a preview.
fn status_word(status: RenameStatus) -> &'static str {
    match status {
        RenameStatus::WillRename => "rename",
        RenameStatus::Unchanged => "unchanged",
        RenameStatus::Conflict | RenameStatus::InternalConflict => "conflict",
        RenameStatus::Error | RenameStatus::Failed => "error",
        RenameStatus::Completed => "renamed",
        RenameStatus::Skipped => "skipped",
    }
}

/// Print the `OLD NAME -> NEW NAME  STATUS` table plus a summary line.
fn print_previews(previews: &[RenamePreview]) {
    let old_width = previews
        .iter()
        .map(|preview| preview.original_name.chars().count())
        .max()
        .unwrap_or(0)
        .max("OLD NAME".len());
    let new_width = previews
        .iter()
        .map(|preview| preview.new_name.chars().count())
        .max()
        .unwrap_or(0)
        .max("NEW NAME".len());

    println!(
        "{:<old_width$}    {:<new_width$}  STATUS",
        "OLD NAME", "NEW NAME"
    );
    for preview in previews {
        let mut line = format!(
            "{:<old_width$} -> {:<new_width$}  {}",
            preview.original_name,
            preview.new_name,
            status_word(preview.status)
        );
        if let Some(message) = &preview.message {
            line.push_str(&format!(" ({})", message));
        }
        println!("{}", line);
    }

    let renames = previews
        .iter()
        .filter(|p| matches!(p.status, RenameStatus::WillRename))
        .count();
    let unchanged = previews
        .iter()
        .filter(|p| matches!(p.status, RenameStatus::Unchanged))
        .count();
    let conflicts = previews
        .iter()
        .filter(|p| {
            matches!(
                p.status,
                RenameStatus::Conflict | RenameStatus::InternalConflict
            )
        })
        .count();
    let errors = previews
        .iter()
        .filter(|p| matches!(p.status, RenameStatus::Error | RenameStatus::Failed))
        .count();
    println!(
        "{} rename, {} unchanged, {} conflict(s), {} error(s)",
        renames, unchanged, conflicts, errors
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    /// An argument made of raw bytes, the way a shell passes a Latin-1 file name.
    #[cfg(unix)]
    fn raw(bytes: &[u8]) -> OsString {
        use std::os::unix::ffi::OsStrExt;
        std::ffi::OsStr::from_bytes(bytes).to_os_string()
    }

    /// `std::env::args` panics on an argument that is not UTF-8, before the program
    /// can even look at it.
    #[test]
    #[cfg(unix)]
    fn a_first_argument_that_is_not_utf8_launches_the_gui() {
        assert_eq!(parse(&[raw(b"caf\xe9.jpg")]), Ok(None));
        assert_eq!(parse(&[raw(b"caf\xe9.jpg"), OsString::from("apply")]), Ok(None));
    }

    #[test]
    #[cfg(unix)]
    fn a_path_that_is_not_utf8_keeps_its_bytes() {
        let name = raw(b"caf\xe9.jpg");
        let parsed = parse(&[
            OsString::from("preview"),
            OsString::from("--preset"),
            OsString::from("X"),
            name.clone(),
        ])
        .expect("parses");
        match parsed {
            Some(Command::Preview(job)) => assert_eq!(job.paths, vec![PathBuf::from(name)]),
            other => panic!("unexpected parse result: {:?}", other),
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_preset_name_that_is_not_utf8_is_refused() {
        let mut equals = b"--preset=caf".to_vec();
        equals.push(0xe9);
        for preset_args in [
            vec![OsString::from("--preset"), raw(b"caf\xe9")],
            vec![raw(&equals)],
        ] {
            let mut list = vec![OsString::from("apply")];
            list.extend(preset_args);
            list.push(OsString::from("a.txt"));
            assert!(parse(&list).is_err(), "{:?}", list);
        }
    }

    /// A batch whose history cannot be written used to report success: the CLI
    /// exits right after, so the in-memory record undoes nothing, and the warning
    /// went to a tracing subscriber the CLI never installs.
    #[test]
    fn apply_records_the_batch_where_the_gui_finds_it() {
        let (dir, presets, mut undo) = temp_setup("apply-history");
        let work = dir.join("files");
        std::fs::create_dir_all(&work).expect("create work dir");
        std::fs::write(work.join("UPPER.TXT"), b"a").expect("write file");

        let job = JobArgs {
            preset: "Lowercase All".to_string(),
            paths: vec![work.clone()],
            recursive: false,
            hidden: false,
            yes: true,
        };
        assert_eq!(apply_with(&job, &presets, &mut undo), EXIT_OK);
        assert!(work.join("upper.txt").exists());

        // The GUI's manager reads the record back, and the journal is gone.
        let mut gui = UndoManager::new(dir.join("state").join("undo"), true);
        gui.load_from_disk().expect("load history");
        assert_eq!(gui.undo_count(), 1);
        assert_eq!(gui.undo().expect("undo").success_count, 1);
        assert!(work.join("UPPER.TXT").exists());
        assert_eq!(crate::engine::pending_recovery_count(&dir.join("state").join("undo")), 0);
        let journals = std::fs::read_dir(dir.join("state").join("undo"))
            .expect("read data dir")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("recovery-"))
            .count();
        assert_eq!(journals, 0, "a clean batch leaves no journal");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------- Subcommand detection ----------

    #[test]
    fn no_args_launches_gui() {
        assert_eq!(parse(&[]), Ok(None));
    }

    #[test]
    fn plain_paths_launch_gui() {
        assert_eq!(parse(&args(&["photo.jpg", "some/dir"])), Ok(None));
    }

    #[test]
    fn gtk_style_options_launch_gui() {
        // GLib/GTK application options must fall through untouched.
        assert_eq!(parse(&args(&["--gapplication-service"])), Ok(None));
    }

    #[test]
    fn help_and_version_are_detected() {
        assert_eq!(parse(&args(&["--help"])), Ok(Some(Command::Help)));
        assert_eq!(parse(&args(&["-h"])), Ok(Some(Command::Help)));
        assert_eq!(parse(&args(&["--version"])), Ok(Some(Command::Version)));
        assert_eq!(parse(&args(&["-V"])), Ok(Some(Command::Version)));
    }

    #[test]
    fn list_presets_is_detected() {
        assert_eq!(parse(&args(&["list-presets"])), Ok(Some(Command::ListPresets)));
    }

    #[test]
    fn list_presets_rejects_extra_arguments() {
        assert!(parse(&args(&["list-presets", "extra"])).is_err());
    }

    #[test]
    fn subcommand_help_prints_usage() {
        assert_eq!(
            parse(&args(&["preview", "--help"])),
            Ok(Some(Command::Help))
        );
        assert_eq!(parse(&args(&["apply", "-h"])), Ok(Some(Command::Help)));
    }

    // ---------- Flag parsing ----------

    #[test]
    fn preview_parses_flags_and_paths_in_order() {
        let parsed = parse(&args(&[
            "preview",
            "--preset",
            "Lowercase All",
            "a.txt",
            "--recursive",
            "--hidden",
            "b.txt",
        ]))
        .unwrap();

        assert_eq!(
            parsed,
            Some(Command::Preview(JobArgs {
                preset: "Lowercase All".to_string(),
                paths: vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")],
                recursive: true,
                hidden: true,
                yes: false,
            }))
        );
    }

    #[test]
    fn preset_accepts_equals_form() {
        let parsed = parse(&args(&["preview", "--preset=Title Case", "dir"])).unwrap();
        match parsed {
            Some(Command::Preview(job)) => assert_eq!(job.preset, "Title Case"),
            other => panic!("unexpected parse result: {:?}", other),
        }
    }

    #[test]
    fn preset_value_is_required() {
        assert!(parse(&args(&["preview", "--preset"])).is_err());
        assert!(parse(&args(&["preview", "--preset="])).is_err());
        assert!(parse(&args(&["preview", "a.txt"])).is_err());
    }

    #[test]
    fn paths_are_required() {
        assert!(parse(&args(&["preview", "--preset", "X"])).is_err());
        assert!(parse(&args(&["apply", "--preset", "X", "--yes"])).is_err());
    }

    #[test]
    fn unknown_options_are_rejected() {
        assert!(parse(&args(&["preview", "--preset", "X", "--bogus", "a"])).is_err());
    }

    #[test]
    fn yes_is_apply_only() {
        assert!(parse(&args(&["preview", "--preset", "X", "--yes", "a"])).is_err());

        let parsed = parse(&args(&["apply", "--preset", "X", "--yes", "a"])).unwrap();
        match parsed {
            Some(Command::Apply(job)) => assert!(job.yes),
            other => panic!("unexpected parse result: {:?}", other),
        }
    }

    #[test]
    fn double_dash_ends_option_parsing() {
        let parsed = parse(&args(&["apply", "--preset", "X", "--", "--hidden"])).unwrap();
        match parsed {
            Some(Command::Apply(job)) => {
                assert_eq!(job.paths, vec![PathBuf::from("--hidden")]);
                assert!(!job.hidden);
            }
            other => panic!("unexpected parse result: {:?}", other),
        }
    }

    // ---------- End-to-end ----------

    /// Fresh managers rooted in a throwaway directory. `PresetManager::new`
    /// seeds the built-in presets, so "Lowercase All" is available.
    fn temp_setup(tag: &str) -> (PathBuf, PresetManager, UndoManager) {
        let dir = std::env::temp_dir().join(format!("bulk-renamer-cli-{}-{}", tag, Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let presets = PresetManager::new(dir.join("state").join("presets"));
        let undo = UndoManager::new(dir.join("state").join("undo"), true);
        (dir, presets, undo)
    }

    #[test]
    fn apply_lowercases_files_end_to_end() {
        let (dir, presets, mut undo) = temp_setup("apply");
        let work = dir.join("files");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("PHOTO One.TXT"), b"a").unwrap();
        std::fs::write(work.join("Notes.MD"), b"b").unwrap();
        std::fs::write(work.join(".Hidden.TXT"), b"c").unwrap();

        let job = JobArgs {
            preset: "Lowercase All".to_string(),
            paths: vec![work.clone()],
            recursive: false,
            hidden: false,
            yes: true,
        };

        let code = apply_with(&job, &presets, &mut undo);
        assert_eq!(code, EXIT_OK);

        assert!(work.join("photo one.txt").exists());
        assert!(work.join("notes.md").exists());
        assert!(!work.join("PHOTO One.TXT").exists());
        assert!(!work.join("Notes.MD").exists());
        // Hidden file untouched without --hidden.
        assert!(work.join(".Hidden.TXT").exists());
        // The batch is recorded so the GUI can undo it.
        assert!(undo.can_undo());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn preview_changes_nothing_and_flags_conflicts() {
        let (dir, presets, _undo) = temp_setup("preview");
        let work = dir.join("files");
        std::fs::create_dir_all(&work).unwrap();
        // Both lowercase to "dup.txt": one unchanged, one blocked as a conflict.
        std::fs::write(work.join("DUP.txt"), b"a").unwrap();
        std::fs::write(work.join("dup.txt"), b"b").unwrap();

        let job = JobArgs {
            preset: "Lowercase All".to_string(),
            paths: vec![work.clone()],
            recursive: false,
            hidden: false,
            yes: false,
        };

        let code = preview_with(&job, &presets);
        assert_eq!(code, EXIT_BLOCKED);

        // Preview never touches the filesystem.
        assert!(work.join("DUP.txt").exists());
        assert!(work.join("dup.txt").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_refuses_when_plan_has_conflicts() {
        let (dir, presets, mut undo) = temp_setup("refuse");
        let work = dir.join("files");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("DUP.txt"), b"a").unwrap();
        std::fs::write(work.join("dup.txt"), b"b").unwrap();

        let job = JobArgs {
            preset: "Lowercase All".to_string(),
            paths: vec![work.clone()],
            recursive: false,
            hidden: false,
            yes: true,
        };

        let code = apply_with(&job, &presets, &mut undo);
        assert_eq!(code, EXIT_BLOCKED);
        assert!(work.join("DUP.txt").exists());
        assert!(work.join("dup.txt").exists());
        assert!(!undo.can_undo());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_path_is_an_error() {
        let (dir, presets, _undo) = temp_setup("missing");

        let job = JobArgs {
            preset: "Lowercase All".to_string(),
            paths: vec![dir.join("does-not-exist.txt")],
            recursive: false,
            hidden: false,
            yes: false,
        };

        assert_eq!(preview_with(&job, &presets), EXIT_BLOCKED);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_preset_is_an_error() {
        let (dir, presets, _undo) = temp_setup("preset");
        let work = dir.join("files");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("a.txt"), b"a").unwrap();

        let job = JobArgs {
            preset: "No Such Preset".to_string(),
            paths: vec![work],
            recursive: false,
            hidden: false,
            yes: false,
        };

        assert_eq!(preview_with(&job, &presets), EXIT_BLOCKED);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `../..` up to the root from `from`, then down to `target`.
    fn relative_to(from: &std::path::Path, target: &std::path::Path) -> PathBuf {
        let mut path = PathBuf::new();
        for _ in from.components().skip(1) {
            path.push("..");
        }
        path.join(target.strip_prefix("/").expect("absolute target"))
    }

    /// Relative paths went into the undo history as typed, and the GUI resolves them
    /// against its own working directory, where the files are not.
    #[test]
    #[cfg(unix)]
    fn collected_paths_are_absolute() {
        let (dir, _presets, _undo) = temp_setup("absolute");
        let work = dir.join("files");
        std::fs::create_dir_all(&work).expect("create work dir");
        std::fs::write(work.join("a.txt"), b"a").expect("write file");

        // The same folder spelled relative to the working directory, the way a user
        // types `bulk-renamer apply --preset X Holiday`.
        let relative = relative_to(&std::env::current_dir().expect("cwd"), &work);
        assert!(relative.is_relative());
        let job = JobArgs {
            preset: "Lowercase All".to_string(),
            paths: vec![relative.clone(), relative.join("a.txt")],
            recursive: false,
            hidden: false,
            yes: false,
        };

        let entries = collect_entries(&job).expect("entries");
        assert_eq!(entries.len(), 2);
        for entry in &entries {
            assert!(entry.path.is_absolute(), "{} is relative", entry.path.display());
            assert!(entry.path.exists());
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
