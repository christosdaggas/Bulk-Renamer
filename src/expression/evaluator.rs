//! Expression evaluator for the renamer DSL.

use crate::core::{FileEntry, RenamerError, RenamerResult};
use crate::engine::transformer::*;
use crate::core::NumberFormat;
use chrono::Local;
use regex::Regex;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Content-hash algorithms exposed as template variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum HashAlgo {
    Sha256,
    Sha1,
    Md5,
    Crc32,
}

/// The expression engine evaluates template expressions.
pub struct ExpressionEngine {
    /// Counter for sequential numbering.
    counter: i64,
    /// Total count of files (set externally).
    total: i64,
    /// Custom variables.
    variables: HashMap<String, String>,
    /// Per-pass cache of file content digests, keyed by (path, algorithm).
    /// RefCell because `evaluate` takes `&self`; cleared by `reset_counter`,
    /// which the rename engine calls at the start of every preview pass.
    hash_cache: RefCell<HashMap<(PathBuf, HashAlgo), String>>,
}

impl ExpressionEngine {
    /// Create a new expression engine.
    pub fn new() -> Self {
        Self {
            counter: 1,
            total: 0,
            variables: HashMap::new(),
            hash_cache: RefCell::new(HashMap::new()),
        }
    }

    /// Set the total file count.
    pub fn set_total(&mut self, total: i64) {
        self.total = total;
    }

    /// Reset per-pass state: the counter and the content-hash cache.
    pub fn reset_counter(&mut self) {
        self.counter = 1;
        // A new pass may see changed file contents, so cached digests expire here.
        self.hash_cache.borrow_mut().clear();
    }

    /// Set a custom variable.
    pub fn set_variable(&mut self, name: &str, value: &str) {
        self.variables.insert(name.to_string(), value.to_string());
    }

    /// Evaluate an expression template.
    pub fn evaluate(&self, template: &str, entry: &FileEntry, current_name: &str) -> RenamerResult<String> {
        let mut result = String::new();
        let mut chars = template.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '$' && chars.peek() == Some(&'$') {
                // `$${` escapes to a literal `${`; any other `$$` passes through.
                let mut lookahead = chars.clone();
                lookahead.next(); // step past the second '$'
                if lookahead.peek() == Some(&'{') {
                    chars.next(); // consume the second '$'
                    result.push('$');
                    // The '{' is emitted as a literal on the next iteration.
                } else {
                    result.push(c);
                }
            } else if c == '$' && chars.peek() == Some(&'{') {
                chars.next(); // consume '{'

                // Collect the expression inside ${}. Braces inside string
                // literals are content, not nesting, so track quote state
                // (same delimiters as parse_args, which has no escapes).
                let mut expr = String::new();
                let mut depth = 1;
                let mut in_string = false;
                let mut string_char = '"';

                while let Some(c) = chars.next() {
                    match c {
                        '"' | '\'' => {
                            if !in_string {
                                in_string = true;
                                string_char = c;
                            } else if c == string_char {
                                in_string = false;
                            }
                            expr.push(c);
                        }
                        '{' if !in_string => {
                            depth += 1;
                            expr.push(c);
                        }
                        '}' if !in_string => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            expr.push(c);
                        }
                        _ => {
                            expr.push(c);
                        }
                    }
                }

                // Evaluate the expression
                let value = self.evaluate_expr(&expr, entry, current_name)?;
                result.push_str(&value);
            } else {
                result.push(c);
            }
        }

        Ok(result)
    }

    /// Evaluate a single expression (inside ${}).
    fn evaluate_expr(&self, expr: &str, entry: &FileEntry, current_name: &str) -> RenamerResult<String> {
        let expr = expr.trim();

        // Check if it's a function call
        if let Some(paren_pos) = expr.find('(') {
            if expr.ends_with(')') {
                let func_name = &expr[..paren_pos];
                let args_str = &expr[paren_pos + 1..expr.len() - 1];
                let args = self.parse_args(args_str, entry, current_name)?;
                return self.call_function(func_name, &args, entry, current_name);
            }
        }

        // Otherwise it's a variable
        self.get_variable(expr, entry, current_name)
    }

    /// Parse function arguments.
    fn parse_args(&self, args_str: &str, entry: &FileEntry, current_name: &str) -> RenamerResult<Vec<String>> {
        if args_str.trim().is_empty() {
            return Ok(Vec::new());
        }

        let mut args = Vec::new();
        let mut current = String::new();
        let mut depth = 0;
        let mut in_string = false;
        let mut string_char = '"';

        for c in args_str.chars() {
            match c {
                // String state must be tracked at every depth, so parens and
                // commas inside a quoted argument of a nested call stay literal.
                '"' | '\'' => {
                    if !in_string {
                        in_string = true;
                        string_char = c;
                    } else if c == string_char {
                        in_string = false;
                    }
                    current.push(c);
                }
                '(' if !in_string => {
                    depth += 1;
                    current.push(c);
                }
                ')' if !in_string => {
                    depth -= 1;
                    current.push(c);
                }
                ',' if depth == 0 && !in_string => {
                    args.push(self.evaluate_arg(&current.trim(), entry, current_name)?);
                    current.clear();
                }
                _ => {
                    current.push(c);
                }
            }
        }

        if !current.is_empty() {
            args.push(self.evaluate_arg(&current.trim(), entry, current_name)?);
        }

        Ok(args)
    }

    /// Evaluate a single argument.
    fn evaluate_arg(&self, arg: &str, entry: &FileEntry, current_name: &str) -> RenamerResult<String> {
        let arg = arg.trim();

        // String literal. A lone quote starts and ends with a quote too, and slicing it
        // as `arg[1..0]` panicked; it falls through and is reported as unknown.
        if arg.len() >= 2
            && ((arg.starts_with('"') && arg.ends_with('"'))
                || (arg.starts_with('\'') && arg.ends_with('\'')))
        {
            return Ok(arg[1..arg.len() - 1].to_string());
        }

        // Number literal
        if arg.parse::<f64>().is_ok() {
            return Ok(arg.to_string());
        }

        // Nested expression or variable
        self.evaluate_expr(arg, entry, current_name)
    }

    /// Get a variable value.
    fn get_variable(&self, name: &str, entry: &FileEntry, current_name: &str) -> RenamerResult<String> {
        // Check custom variables first
        if let Some(value) = self.variables.get(name) {
            return Ok(value.clone());
        }

        let result = match name {
            // File name components
            "name" => current_name.to_string(),
            "stem" => entry.stem(),
            "ext" | "extension" => entry.extension.clone().unwrap_or_default(),
            
            // Path components
            "parent" => entry.parent_name.clone().unwrap_or_default(),
            "grandparent" => entry
                .path
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            "path" => entry.path.to_string_lossy().to_string(),
            "dir" => entry
                .path
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),

            // File properties
            "size" => entry.size.to_string(),
            // Content hashes (lowercase hex); a read error resolves to an
            // empty string, matching how other variables behave on missing data.
            "sha256" => self.file_hash(entry, HashAlgo::Sha256),
            "sha1" => self.file_hash(entry, HashAlgo::Sha1),
            "md5" => self.file_hash(entry, HashAlgo::Md5),
            "crc32" => self.file_hash(entry, HashAlgo::Crc32),
            "created" => entry
                .created
                .map(|d| d.format("%Y%m%d").to_string())
                .unwrap_or_default(),
            "modified" => entry
                .modified
                .map(|d| d.format("%Y%m%d").to_string())
                .unwrap_or_default(),
            "accessed" => entry
                .accessed
                .map(|d| d.format("%Y%m%d").to_string())
                .unwrap_or_default(),
            "taken" => entry
                .metadata_cache
                .as_ref()
                .and_then(|m| m.exif.as_ref())
                .and_then(|e| e.date_taken)
                .map(|d| d.format("%Y%m%d").to_string())
                .unwrap_or_default(),

            // Counters
            "index" | "counter" => self.counter.to_string(),
            "total" => self.total.to_string(),

            // Current date/time
            "year" => Local::now().format("%Y").to_string(),
            "month" => Local::now().format("%m").to_string(),
            "day" => Local::now().format("%d").to_string(),
            "hour" => Local::now().format("%H").to_string(),
            "minute" => Local::now().format("%M").to_string(),
            "second" => Local::now().format("%S").to_string(),

            // Image metadata
            "width" => entry
                .metadata_cache
                .as_ref()
                .and_then(|m| m.dimensions)
                .map(|(w, _)| w.to_string())
                .unwrap_or_default(),
            "height" => entry
                .metadata_cache
                .as_ref()
                .and_then(|m| m.dimensions)
                .map(|(_, h)| h.to_string())
                .unwrap_or_default(),
            "camera" => entry
                .metadata_cache
                .as_ref()
                .and_then(|m| m.exif.as_ref())
                .and_then(|e| e.camera_model.clone())
                .unwrap_or_default(),
            "iso" => entry
                .metadata_cache
                .as_ref()
                .and_then(|m| m.exif.as_ref())
                .and_then(|e| e.iso)
                .map(|i| i.to_string())
                .unwrap_or_default(),
            "aperture" => entry
                .metadata_cache
                .as_ref()
                .and_then(|m| m.exif.as_ref())
                .and_then(|e| e.aperture)
                .map(|a| format!("f{:.1}", a))
                .unwrap_or_default(),
            "focal" => entry
                .metadata_cache
                .as_ref()
                .and_then(|m| m.exif.as_ref())
                .and_then(|e| e.focal_length)
                .map(|f| format!("{}mm", f))
                .unwrap_or_default(),

            // Audio metadata
            "artist" => entry
                .metadata_cache
                .as_ref()
                .and_then(|m| m.id3.as_ref())
                .and_then(|i| i.artist.clone())
                .unwrap_or_default(),
            "album" => entry
                .metadata_cache
                .as_ref()
                .and_then(|m| m.id3.as_ref())
                .and_then(|i| i.album.clone())
                .unwrap_or_default(),
            "title" => entry
                .metadata_cache
                .as_ref()
                .and_then(|m| m.id3.as_ref())
                .and_then(|i| i.title.clone())
                .unwrap_or_default(),
            "track" => entry
                .metadata_cache
                .as_ref()
                .and_then(|m| m.id3.as_ref())
                .and_then(|i| i.track)
                .map(|t| t.to_string())
                .unwrap_or_default(),
            "genre" => entry
                .metadata_cache
                .as_ref()
                .and_then(|m| m.id3.as_ref())
                .and_then(|i| i.genre.clone())
                .unwrap_or_default(),

            _ => {
                return Err(RenamerError::InvalidExpression(format!(
                    "Unknown variable: {}",
                    name
                )));
            }
        };

        Ok(result)
    }

    /// Call a function with arguments.
    fn call_function(
        &self,
        name: &str,
        args: &[String],
        entry: &FileEntry,
        current_name: &str,
    ) -> RenamerResult<String> {
        let result = match name {
            // String case functions
            "upper" | "uppercase" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                s.to_uppercase()
            }
            "lower" | "lowercase" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                s.to_lowercase()
            }
            "title" | "titlecase" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                titlecase::titlecase(s)
            }
            "sentence" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                transform_case(s, crate::core::CaseType::Sentence)
            }
            "camel" | "camelcase" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                transform_case(s, crate::core::CaseType::Camel)
            }
            "pascal" | "pascalcase" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                transform_case(s, crate::core::CaseType::Pascal)
            }
            "snake" | "snakecase" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                transform_case(s, crate::core::CaseType::Snake)
            }
            "kebab" | "kebabcase" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                transform_case(s, crate::core::CaseType::Kebab)
            }

            // String manipulation
            "trim" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                s.trim().to_string()
            }
            "ltrim" | "trimstart" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                s.trim_start().to_string()
            }
            "rtrim" | "trimend" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                s.trim_end().to_string()
            }
            "replace" => {
                if args.len() < 3 {
                    return Err(RenamerError::InvalidExpression(
                        "replace() requires 3 arguments: string, old, new".to_string(),
                    ));
                }
                args[0].replace(&args[1], &args[2])
            }
            "regex" => {
                if args.len() < 3 {
                    return Err(RenamerError::InvalidExpression(
                        "regex() requires 3 arguments: string, pattern, replacement".to_string(),
                    ));
                }
                let re = Regex::new(&args[1]).map_err(|e| {
                    RenamerError::InvalidExpression(format!("Invalid regex: {}", e))
                })?;
                re.replace_all(&args[0], &args[2] as &str).to_string()
            }
            "substr" | "substring" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                let start: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                let len: usize = args
                    .get(2)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(s.len());
                s.chars().skip(start).take(len).collect()
            }
            "left" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                s.chars().take(n).collect()
            }
            "right" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                let chars: Vec<char> = s.chars().collect();
                if n >= chars.len() {
                    s.to_string()
                } else {
                    chars[chars.len() - n..].iter().collect()
                }
            }
            "mid" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                let start: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                let len: usize = args
                    .get(2)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(s.len());
                s.chars().skip(start).take(len).collect()
            }
            "pad" | "lpad" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                let len: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(s.len());
                let pad_char: char = args
                    .get(2)
                    .and_then(|s| s.chars().next())
                    .unwrap_or(' ');
                let current_len = s.chars().count();
                if current_len >= len {
                    s.to_string()
                } else {
                    let padding: String =
                        std::iter::repeat(pad_char).take(len - current_len).collect();
                    format!("{}{}", padding, s)
                }
            }
            "rpad" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                let len: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(s.len());
                let pad_char: char = args
                    .get(2)
                    .and_then(|s| s.chars().next())
                    .unwrap_or(' ');
                let current_len = s.chars().count();
                if current_len >= len {
                    s.to_string()
                } else {
                    let padding: String =
                        std::iter::repeat(pad_char).take(len - current_len).collect();
                    format!("{}{}", s, padding)
                }
            }
            "repeat" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or("");
                let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
                s.repeat(n)
            }
            "reverse" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                s.chars().rev().collect()
            }
            "clean" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                s.chars()
                    .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '-' || *c == '_')
                    .collect()
            }
            "slug" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                remove_diacritics(s)
                    .to_lowercase()
                    .chars()
                    .map(|c| {
                        if c.is_alphanumeric() {
                            c
                        } else if c.is_whitespace() || c == '_' {
                            '-'
                        } else {
                            '-'
                        }
                    })
                    .collect::<String>()
                    .split('-')
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("-")
            }

            // Number formatting
            "num" | "number" => {
                let n: i64 = args.get(0).and_then(|s| s.parse().ok()).unwrap_or(0);
                let padding: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
                format_number(n, NumberFormat::Decimal, padding)
            }
            "hex" => {
                let n: i64 = args.get(0).and_then(|s| s.parse().ok()).unwrap_or(0);
                let padding: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
                format_number(n, NumberFormat::Hex, padding)
            }
            "roman" => {
                let n: i64 = args.get(0).and_then(|s| s.parse().ok()).unwrap_or(0);
                format_number(n, NumberFormat::Roman, 0)
            }
            "letter" => {
                let n: i64 = args.get(0).and_then(|s| s.parse().ok()).unwrap_or(1);
                format_number(n, NumberFormat::Letter, 0)
            }
            "round" => {
                let n: f64 = args.get(0).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                n.round().to_string()
            }
            "floor" => {
                let n: f64 = args.get(0).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                n.floor().to_string()
            }
            "ceil" => {
                let n: f64 = args.get(0).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                n.ceil().to_string()
            }
            "abs" => {
                let n: f64 = args.get(0).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                n.abs().to_string()
            }

            // Date formatting
            "date" => {
                let format = args.get(0).map(|s| s.as_str()).unwrap_or("%Y-%m-%d");
                format_date(&Local::now(), format)?
            }
            "filedate" => {
                let source = args.get(0).map(|s| s.as_str()).unwrap_or("modified");
                let format = args.get(1).map(|s| s.as_str()).unwrap_or("%Y-%m-%d");
                
                let date = match source {
                    "created" => entry.created,
                    "modified" => entry.modified,
                    "accessed" => entry.accessed,
                    "exif" | "taken" => entry
                        .metadata_cache
                        .as_ref()
                        .and_then(|m| m.exif.as_ref())
                        .and_then(|e| e.date_taken),
                    // Falling back to mtime renamed photos by the wrong date without
                    // saying so, so an unknown source is an error.
                    other => {
                        return Err(RenamerError::InvalidExpression(format!(
                            "Unknown filedate source: {}",
                            other
                        )));
                    }
                };

                format_optional_date(date, format)?
            }

            // Conditional
            "if" => {
                if args.len() < 3 {
                    return Err(RenamerError::InvalidExpression(
                        "if() requires 3 arguments: condition, then, else".to_string(),
                    ));
                }
                let cond = &args[0];
                let is_true = !cond.is_empty() && cond != "0" && cond.to_lowercase() != "false";
                if is_true {
                    args[1].clone()
                } else {
                    args[2].clone()
                }
            }
            "coalesce" => {
                args.iter()
                    .find(|s| !s.is_empty())
                    .cloned()
                    .unwrap_or_default()
            }
            "default" => {
                let val = args.get(0).map(|s| s.as_str()).unwrap_or("");
                let default = args.get(1).map(|s| s.as_str()).unwrap_or("");
                if val.is_empty() {
                    default.to_string()
                } else {
                    val.to_string()
                }
            }

            // Meta functions
            "len" | "length" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                s.chars().count().to_string()
            }
            "ext" | "extension" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                std::path::Path::new(s)
                    .extension()
                    .map(|e| e.to_string_lossy().to_string())
                    .unwrap_or_default()
            }
            "stem" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                std::path::Path::new(s)
                    .file_stem()
                    .map(|e| e.to_string_lossy().to_string())
                    .unwrap_or_else(|| s.to_string())
            }
            "dir" | "dirname" => {
                let path_str = entry.path.to_string_lossy().to_string();
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(&path_str);
                std::path::Path::new(s)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default()
            }
            "filename" | "basename" => {
                let s = args.get(0).map(|s| s.as_str()).unwrap_or(current_name);
                std::path::Path::new(s)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| s.to_string())
            }

            // Concat
            "concat" | "join" => args.join(""),

            _ => {
                return Err(RenamerError::InvalidExpression(format!(
                    "Unknown function: {}",
                    name
                )));
            }
        };

        Ok(result)
    }

    /// Increment the counter and return the current value.
    pub fn next_counter(&mut self) -> i64 {
        let current = self.counter;
        self.counter += 1;
        current
    }

    /// Digest of the entry's file content, cached per (path, algorithm) for the
    /// current pass. Unreadable files resolve to an empty string, and that
    /// result is cached too so a missing file is not retried per variable use.
    fn file_hash(&self, entry: &FileEntry, algo: HashAlgo) -> String {
        let key = (entry.path.clone(), algo);
        if let Some(cached) = self.hash_cache.borrow().get(&key) {
            return cached.clone();
        }
        let digest = compute_file_hash(&entry.path, algo).unwrap_or_default();
        self.hash_cache.borrow_mut().insert(key, digest.clone());
        digest
    }
}

/// Stream a file through `update` in fixed-size chunks so arbitrarily large
/// files are hashed without ever being held in memory.
fn stream_file<F: FnMut(&[u8])>(path: &Path, mut update: F) -> std::io::Result<()> {
    let mut file = std::fs::File::open(path)?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        update(&buf[..n]);
    }
}

/// Render digest bytes as lowercase hex.
fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Compute the lowercase-hex content digest of `path` with `algo`.
fn compute_file_hash(path: &Path, algo: HashAlgo) -> std::io::Result<String> {
    use sha2::Digest;

    match algo {
        HashAlgo::Sha256 => {
            let mut hasher = sha2::Sha256::new();
            stream_file(path, |chunk| hasher.update(chunk))?;
            Ok(to_hex(&hasher.finalize()))
        }
        HashAlgo::Sha1 => {
            let mut hasher = sha1::Sha1::new();
            stream_file(path, |chunk| hasher.update(chunk))?;
            Ok(to_hex(&hasher.finalize()))
        }
        HashAlgo::Md5 => {
            let mut hasher = md5::Md5::new();
            stream_file(path, |chunk| hasher.update(chunk))?;
            Ok(to_hex(&hasher.finalize()))
        }
        HashAlgo::Crc32 => {
            let mut hasher = crc32fast::Hasher::new();
            stream_file(path, |chunk| hasher.update(chunk))?;
            Ok(format!("{:08x}", hasher.finalize()))
        }
    }
}

impl Default for ExpressionEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{ExifData, MetadataCache};
    use chrono::TimeZone;
    use std::path::PathBuf;

    /// A photo whose EXIF date differs from its mtime, so falling back to mtime shows.
    fn make_photo_entry() -> FileEntry {
        let taken = Local
            .with_ymd_and_hms(2021, 7, 4, 9, 30, 0)
            .single()
            .expect("valid date taken");
        let modified = Local
            .with_ymd_and_hms(2024, 1, 2, 3, 4, 5)
            .single()
            .expect("valid modified time");

        FileEntry {
            modified: Some(modified),
            metadata_cache: Some(MetadataCache {
                exif: Some(ExifData {
                    date_taken: Some(taken),
                    camera_make: None,
                    camera_model: None,
                    focal_length: None,
                    aperture: None,
                    iso: None,
                    exposure_time: None,
                    gps_latitude: None,
                    gps_longitude: None,
                    orientation: None,
                    width: None,
                    height: None,
                }),
                ..MetadataCache::default()
            }),
            ..make_test_entry()
        }
    }

    fn make_test_entry() -> FileEntry {
        FileEntry {
            id: uuid::Uuid::new_v4(),
            path: PathBuf::from("/home/user/photos/vacation.jpg"),
            original_name: "vacation.jpg".to_string(),
            extension: Some("jpg".to_string()),
            is_directory: false,
            size: 1024000,
            modified: None,
            created: None,
            accessed: None,
            depth: 0,
            parent_name: Some("photos".to_string()),
            metadata_cache: None,
        }
    }

    #[test]
    fn test_simple_variable() {
        let engine = ExpressionEngine::new();
        let entry = make_test_entry();
        
        let result = engine.evaluate("${name}", &entry, "vacation").unwrap();
        assert_eq!(result, "vacation");
    }

    #[test]
    fn test_function_call() {
        let engine = ExpressionEngine::new();
        let entry = make_test_entry();
        
        let result = engine.evaluate("${upper(name)}", &entry, "vacation").unwrap();
        assert_eq!(result, "VACATION");
    }

    #[test]
    fn test_template_with_literal() {
        let engine = ExpressionEngine::new();
        let entry = make_test_entry();
        
        let result = engine.evaluate("prefix_${name}_suffix", &entry, "vacation").unwrap();
        assert_eq!(result, "prefix_vacation_suffix");
    }

    #[test]
    fn test_replace_function() {
        let engine = ExpressionEngine::new();
        let entry = make_test_entry();
        
        let result = engine.evaluate("${replace(name, 'a', 'o')}", &entry, "vacation").unwrap();
        assert_eq!(result, "vocotion");
    }

    #[test]
    fn test_number_formatting() {
        let engine = ExpressionEngine::new();
        let entry = make_test_entry();

        let result = engine.evaluate("file_${num(counter, 3)}", &entry, "vacation").unwrap();
        assert_eq!(result, "file_001");
    }

    #[test]
    fn filedate_exif_reads_the_date_taken() {
        let engine = ExpressionEngine::new();
        let entry = make_photo_entry();

        // The shape the "Photo Rename (EXIF Date)" preset uses.
        let taken = engine
            .evaluate("${filedate('exif', '%Y%m%d_%H%M%S')}", &entry, "photo")
            .expect("evaluate exif date");
        assert_eq!(taken, "20210704_093000");

        let alias = engine
            .evaluate("${filedate('taken', '%Y%m%d')}", &entry, "photo")
            .expect("evaluate taken date");
        assert_eq!(alias, "20210704");

        // The other sources still read what they say they read.
        let modified = engine
            .evaluate("${filedate('modified', '%Y%m%d')}", &entry, "photo")
            .expect("evaluate modified date");
        assert_eq!(modified, "20240102");
    }

    #[test]
    fn filedate_reports_an_unknown_source() {
        let engine = ExpressionEngine::new();
        let entry = make_photo_entry();

        assert!(
            engine
                .evaluate("${filedate('modifed', '%Y%m%d')}", &entry, "photo")
                .is_err(),
            "a typo must surface instead of silently returning the mtime"
        );
    }

    #[test]
    fn taken_variable_reads_the_exif_date() {
        let engine = ExpressionEngine::new();
        let entry = make_photo_entry();

        let result = engine
            .evaluate("${taken}", &entry, "photo")
            .expect("evaluate taken");
        assert_eq!(result, "20210704");
    }

    #[test]
    fn counter_and_total_follow_the_engine() {
        let mut engine = ExpressionEngine::new();
        let entry = make_test_entry();
        engine.set_total(3);

        assert_eq!(
            engine
                .evaluate("${index}/${total}", &entry, "vacation")
                .expect("evaluate first"),
            "1/3"
        );

        engine.next_counter();
        assert_eq!(
            engine
                .evaluate("${counter}/${total}", &entry, "vacation")
                .expect("evaluate second"),
            "2/3"
        );

        engine.reset_counter();
        assert_eq!(
            engine
                .evaluate("${index}", &entry, "vacation")
                .expect("evaluate after reset"),
            "1"
        );
    }

    #[test]
    fn quoted_closing_brace_does_not_end_the_placeholder() {
        let engine = ExpressionEngine::new();
        let entry = make_test_entry();

        let result = engine
            .evaluate(r#"${replace(name, "}", "_")}"#, &entry, "a}b")
            .expect("evaluate quoted closing brace");
        assert_eq!(result, "a_b");
    }

    #[test]
    fn quoted_opening_brace_does_not_nest_the_placeholder() {
        let engine = ExpressionEngine::new();
        let entry = make_test_entry();

        let result = engine
            .evaluate(r#"${replace(name, "{", "_")}"#, &entry, "a{b")
            .expect("evaluate quoted opening brace");
        assert_eq!(result, "a_b");
    }

    #[test]
    fn quoted_paren_inside_nested_call_stays_literal() {
        let engine = ExpressionEngine::new();
        let entry = make_test_entry();

        let result = engine
            .evaluate(r#"${concat(replace(name, "(", ""), "x")}"#, &entry, "a(b")
            .expect("evaluate quoted paren in nested call");
        assert_eq!(result, "abx");
    }

    #[test]
    fn quoted_comma_is_a_single_argument() {
        let engine = ExpressionEngine::new();
        let entry = make_test_entry();

        let result = engine
            .evaluate(r#"${replace(name, ",", "-")}"#, &entry, "a,b")
            .expect("evaluate quoted comma");
        assert_eq!(result, "a-b");
    }

    #[test]
    fn nested_calls_still_evaluate() {
        let engine = ExpressionEngine::new();
        let entry = make_test_entry();

        let result = engine
            .evaluate("${upper(replace(name, 'a', 'o'))}", &entry, "vacation")
            .expect("evaluate nested call");
        assert_eq!(result, "VOCOTION");

        let result = engine
            .evaluate("${concat(left(name, 3), '_', upper(right(name, 4)))}", &entry, "vacation")
            .expect("evaluate multi-argument nested call");
        assert_eq!(result, "vac_TION");
    }

    /// A real on-disk file with known content, plus an entry pointing at it.
    fn make_hash_fixture(content: &[u8]) -> (PathBuf, FileEntry) {
        let path = std::env::temp_dir().join(format!(
            "bulk-renamer-hash-test-{}.txt",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, content).expect("write hash fixture");
        let entry = FileEntry {
            path: path.clone(),
            ..make_test_entry()
        };
        (path, entry)
    }

    #[test]
    fn hash_variables_digest_file_content() {
        let (path, entry) = make_hash_fixture(b"hello world");
        let engine = ExpressionEngine::new();

        assert_eq!(
            engine.evaluate("${sha256}", &entry, "hw").expect("sha256"),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
        assert_eq!(
            engine.evaluate("${sha1}", &entry, "hw").expect("sha1"),
            "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed"
        );
        assert_eq!(
            engine.evaluate("${md5}", &entry, "hw").expect("md5"),
            "5eb63bbbe01eeed093cb22bb8f5acdc3"
        );
        // Leading zero also proves the crc32 output is padded to 8 digits.
        assert_eq!(
            engine.evaluate("${crc32}", &entry, "hw").expect("crc32"),
            "0d4a1185"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn hash_variables_compose_with_functions() {
        let (path, entry) = make_hash_fixture(b"hello world");
        let engine = ExpressionEngine::new();

        // Function arguments resolve variables, so substr truncates a digest.
        assert_eq!(
            engine
                .evaluate("${substr(sha256, 0, 8)}", &entry, "hw")
                .expect("substr over sha256"),
            "b94d27b9"
        );
        assert_eq!(
            engine
                .evaluate("${upper(left(md5, 4))}", &entry, "hw")
                .expect("nested functions over md5"),
            "5EB6"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn hash_variables_resolve_empty_on_read_error() {
        let engine = ExpressionEngine::new();
        let entry = FileEntry {
            path: PathBuf::from("/nonexistent/bulk-renamer-missing-file.bin"),
            ..make_test_entry()
        };

        for template in ["${sha256}", "${sha1}", "${md5}", "${crc32}"] {
            assert_eq!(
                engine
                    .evaluate(template, &entry, "x")
                    .expect("missing file evaluates"),
                "",
                "{template} should resolve to an empty string on read error"
            );
        }
    }

    #[test]
    fn hash_cache_holds_within_a_pass_and_clears_on_reset() {
        let (path, entry) = make_hash_fixture(b"hello world");
        let mut engine = ExpressionEngine::new();

        assert_eq!(
            engine.evaluate("${crc32}", &entry, "x").expect("first read"),
            "0d4a1185"
        );

        // Mid-pass content changes are not re-read: the digest is cached.
        std::fs::write(&path, b"changed").expect("rewrite fixture");
        assert_eq!(
            engine.evaluate("${crc32}", &entry, "x").expect("cached read"),
            "0d4a1185"
        );

        // reset_counter starts a new pass and drops the cache.
        engine.reset_counter();
        let fresh = engine
            .evaluate("${crc32}", &entry, "x")
            .expect("post-reset read");
        assert_ne!(fresh, "0d4a1185");
        assert_eq!(fresh.len(), 8);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn double_dollar_escapes_a_literal_placeholder() {
        let engine = ExpressionEngine::new();
        let entry = make_test_entry();

        let result = engine
            .evaluate("$${name}_${name}", &entry, "vacation")
            .expect("evaluate escaped placeholder");
        assert_eq!(result, "${name}_vacation");

        // `$$` not followed by `{` passes through untouched.
        let result = engine
            .evaluate("cost_$$5_${name}", &entry, "vacation")
            .expect("evaluate plain double dollar");
        assert_eq!(result, "cost_$$5_vacation");
    }

    /// A lone quote passed the "starts and ends with a quote" test and was then
    /// sliced as `arg[1..0]`, a panic the release profile turns into an abort.
    #[test]
    fn a_lone_quote_argument_is_an_error_not_a_panic() {
        let engine = ExpressionEngine::new();
        let entry = make_test_entry();

        for template in [r#"${upper(")"#, r#"${upper(")}"#, "${upper(')", r#"${concat(name, " )"#] {
            assert!(
                engine.evaluate(template, &entry, "vacation").is_err(),
                "{} must be reported as an error",
                template
            );
        }
    }

    /// chrono's `to_string` panics on an invalid specifier, so a typo in a date
    /// pattern took the whole app down.
    #[test]
    fn an_invalid_date_format_is_an_error_not_a_panic() {
        let engine = ExpressionEngine::new();
        let photo = make_photo_entry();

        for template in [
            "${date('%Q')}",
            "${date('%Y-%')}",
            "${filedate('modified', '%Q')}",
            "${filedate('exif', '%Y-%')}",
        ] {
            let err = engine
                .evaluate(template, &photo, "photo")
                .expect_err(template);
            assert!(err.to_string().contains("date format"), "{}: {}", template, err);
        }

        // A file without that date is no excuse for letting the pattern through.
        let undated = make_test_entry();
        assert!(engine
            .evaluate("${filedate('modified', '%Q')}", &undated, "x")
            .is_err());
    }
}
