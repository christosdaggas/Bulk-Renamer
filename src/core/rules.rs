//! Rename rule definitions and configuration.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A complete rename configuration consisting of multiple rules.
///
/// `#[serde(default)]` lets configs written by older versions (or with fields
/// added in future versions) load gracefully instead of failing outright.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RenameConfig {
    /// Unique identifier.
    pub id: Uuid,
    /// Name of this configuration.
    pub name: String,
    /// Ordered list of rules to apply.
    pub rules: Vec<RenameRule>,
    /// Whether to process the extension separately.
    pub separate_extension: bool,
    /// Filter configuration.
    pub filter: Option<FilterConfig>,
}

impl Default for RenameConfig {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::from("Untitled"),
            rules: Vec::new(),
            separate_extension: true,
            filter: None,
        }
    }
}

fn default_rule_enabled() -> bool {
    true
}

/// A single rename rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameRule {
    /// Unique identifier for this rule.
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    /// Whether this rule is enabled.
    #[serde(default = "default_rule_enabled")]
    pub enabled: bool,
    /// The rule type and configuration.
    pub rule_type: RuleType,
}

impl RenameRule {
    pub fn new(rule_type: RuleType) -> Self {
        Self {
            id: Uuid::new_v4(),
            enabled: true,
            rule_type,
        }
    }
}

/// Types of rename rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RuleType {
    /// Replace text (simple or regex).
    Replace(ReplaceRule),
    /// Insert text at a position.
    Insert(InsertRule),
    /// Remove text.
    Remove(RemoveRule),
    /// Change case.
    ChangeCase(CaseRule),
    /// Add numbering.
    Numbering(NumberingRule),
    /// Trim whitespace or characters.
    Trim(TrimRule),
    /// Pad to a certain length.
    Pad(PadRule),
    /// Use expression/template.
    Expression(ExpressionRule),
    /// Rearrange parts of the filename.
    Rearrange(RearrangeRule),
    /// Date/time formatting.
    DateTime(DateTimeRule),
    /// Metadata-based renaming.
    Metadata(MetadataRule),
    /// Clean up filename (remove special chars, etc.).
    Cleanup(CleanupRule),
    /// Transliterate or convert encoding.
    Transliterate(TransliterateRule),
}

/// Replace rule configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReplaceRule {
    /// Text or pattern to find.
    pub find: String,
    /// Replacement text.
    pub replace: String,
    /// Whether to use regex.
    pub use_regex: bool,
    /// Case-sensitive matching.
    pub case_sensitive: bool,
    /// Replace all occurrences or just first.
    pub replace_all: bool,
    /// Apply to extension as well.
    pub include_extension: bool,
}

impl Default for ReplaceRule {
    fn default() -> Self {
        Self {
            find: String::new(),
            replace: String::new(),
            use_regex: false,
            case_sensitive: true,
            replace_all: true,
            include_extension: false,
        }
    }
}

/// Insert rule configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsertRule {
    /// Text to insert.
    pub text: InsertText,
    /// Position to insert at.
    pub position: InsertPosition,
}

/// What to insert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InsertText {
    /// Fixed text.
    Fixed(String),
    /// Parent folder name.
    ParentFolder,
    /// Grandparent folder name.
    GrandparentFolder,
    /// Current date.
    CurrentDate(String), // Format string
    /// File date (modified, created, accessed).
    FileDate { source: DateSource, format: String },
    /// Counter.
    Counter(CounterConfig),
    /// Clipboard content.
    Clipboard,
    /// Expression result.
    Expression(String),
}

/// Where to insert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InsertPosition {
    /// At the beginning.
    Prefix,
    /// At the end (before extension).
    Suffix,
    /// At specific character position.
    Position(i32), // Negative for from-end
    /// Before specific text.
    BeforeText(String),
    /// After specific text.
    AfterText(String),
    /// Before Nth occurrence of pattern.
    BeforeNth { pattern: String, n: usize },
    /// After Nth occurrence of pattern.
    AfterNth { pattern: String, n: usize },
}

/// Remove rule configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveRule {
    /// What to remove.
    pub target: RemoveTarget,
}

/// What to remove.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RemoveTarget {
    /// Remove specific text.
    Text { text: String, case_sensitive: bool },
    /// Remove by regex pattern.
    Pattern(String),
    /// Remove characters at position.
    Range { start: i32, end: i32 },
    /// Remove first N characters.
    FirstN(usize),
    /// Remove last N characters.
    LastN(usize),
    /// Remove all digits.
    Digits,
    /// Remove all letters.
    Letters,
    /// Remove all symbols.
    Symbols,
    /// Remove all whitespace.
    Whitespace,
    /// Remove specific characters.
    Characters(String),
    /// Remove words.
    Words(Vec<String>),
    /// Remove bracketed content.
    Bracketed(BracketType),
    /// Remove duplicate characters.
    Duplicates,
    /// Remove leading zeros.
    LeadingZeros,
    /// Remove text before/after marker.
    BeforeAfter { marker: String, remove_before: bool, include_marker: bool },
}

/// Type of brackets.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum BracketType {
    Round,      // ()
    Square,     // []
    Curly,      // {}
    Angle,      // <>
    All,        // All types
}

/// Case change rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseRule {
    /// The case transformation to apply.
    pub case_type: CaseType,
    /// Apply to extension.
    pub include_extension: bool,
}

/// Types of case transformations.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum CaseType {
    /// lowercase
    Lower,
    /// UPPERCASE
    Upper,
    /// Title Case
    Title,
    /// Sentence case
    Sentence,
    /// tOGGLE cASE
    Toggle,
    /// camelCase
    Camel,
    /// PascalCase
    Pascal,
    /// snake_case
    Snake,
    /// kebab-case
    Kebab,
    /// CONSTANT_CASE
    Constant,
    /// First letter uppercase
    Capitalize,
    /// Alternating case
    Alternating,
    /// Random case
    Random,
}

/// Numbering rule configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NumberingRule {
    /// Starting number.
    pub start: i64,
    /// Increment per file.
    pub increment: i64,
    /// Zero-padding width.
    pub padding: usize,
    /// Position to insert number.
    pub position: InsertPosition,
    /// Separator before number.
    pub prefix: String,
    /// Separator after number.
    pub suffix: String,
    /// Reset counter per folder.
    pub reset_per_folder: bool,
    /// Number format.
    pub format: NumberFormat,
}

impl Default for NumberingRule {
    fn default() -> Self {
        Self {
            start: 1,
            increment: 1,
            padding: 2,
            position: InsertPosition::Suffix,
            prefix: String::from("_"),
            suffix: String::new(),
            reset_per_folder: false,
            format: NumberFormat::Decimal,
        }
    }
}

/// Number format.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub enum NumberFormat {
    #[default]
    Decimal,
    Hex,
    Octal,
    Binary,
    Roman,
    Letter, // A, B, C, ...
}

/// Trim rule configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrimRule {
    /// What to trim.
    pub trim_type: TrimType,
    /// Characters to trim (if custom).
    pub characters: Option<String>,
}

/// What to trim.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum TrimType {
    /// Trim whitespace from both ends.
    Both,
    /// Trim whitespace from start.
    Start,
    /// Trim whitespace from end.
    End,
    /// Trim specific characters.
    Characters,
    /// Trim to max length.
    MaxLength(usize),
}

/// Pad rule configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PadRule {
    /// Target length.
    pub length: usize,
    /// Padding character.
    pub pad_char: char,
    /// Pad at start or end.
    pub pad_start: bool,
}

/// Expression rule configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpressionRule {
    /// The expression template.
    pub expression: String,
}

/// Rearrange rule configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RearrangeRule {
    /// Separator to split filename.
    pub separator: String,
    /// New order of parts (0-indexed).
    pub order: Vec<usize>,
    /// New separator between parts.
    pub new_separator: String,
}

/// Date/time rule configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DateTimeRule {
    /// Date source.
    pub source: DateSource,
    /// Format string.
    pub format: String,
    /// Position to insert.
    pub position: InsertPosition,
}

/// Source of date information.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum DateSource {
    /// File creation date.
    Created,
    /// File modification date.
    Modified,
    /// File access date.
    Accessed,
    /// Current date/time.
    Now,
    /// EXIF date taken.
    ExifDateTaken,
}

/// Metadata-based rule configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataRule {
    /// The metadata field to use.
    pub field: MetadataField,
    /// Format or transformation.
    pub format: Option<String>,
    /// Fallback if metadata not available.
    pub fallback: String,
    /// Position to insert.
    pub position: InsertPosition,
}

/// Metadata fields that can be used in renaming.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MetadataField {
    // Image EXIF
    ExifDateTaken,
    ExifCameraMake,
    ExifCameraModel,
    ExifFocalLength,
    ExifAperture,
    ExifISO,
    ExifExposure,
    ExifWidth,
    ExifHeight,
    ExifGpsLatitude,
    ExifGpsLongitude,

    // Audio ID3
    Id3Title,
    Id3Artist,
    Id3Album,
    Id3Year,
    Id3Track,
    Id3Genre,
    Id3Duration,
    Id3Bitrate,

    // File properties
    FileSize,
    FileCreated,
    FileModified,
    FileAccessed,
    FilePath,
    FileParent,
    FileExtension,

    // Custom field
    Custom(String),
}

/// Cleanup rule configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CleanupRule {
    /// Remove extra spaces.
    pub collapse_spaces: bool,
    /// Remove special characters.
    pub remove_special: bool,
    /// Characters to preserve.
    pub preserve: String,
    /// Replace spaces with.
    pub space_replacement: Option<char>,
    /// Remove diacritics.
    pub remove_diacritics: bool,
    /// Normalize unicode.
    pub normalize_unicode: bool,
}

impl Default for CleanupRule {
    fn default() -> Self {
        Self {
            collapse_spaces: true,
            remove_special: false,
            preserve: String::from("-_"),
            space_replacement: None,
            remove_diacritics: false,
            normalize_unicode: true,
        }
    }
}

/// Transliterate rule configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransliterateRule {
    /// Transliteration mapping.
    pub mapping: TransliterationMapping,
}

/// Predefined transliteration mappings.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum TransliterationMapping {
    /// Cyrillic to Latin.
    CyrillicToLatin,
    /// Greek to Latin.
    GreekToLatin,
    /// Remove diacritics.
    RemoveDiacritics,
    /// Normalize unicode.
    NormalizeUnicode,
}

/// Counter configuration for numbering.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CounterConfig {
    pub start: i64,
    pub increment: i64,
    pub padding: usize,
    pub format: NumberFormat,
}

impl Default for CounterConfig {
    fn default() -> Self {
        Self {
            start: 1,
            increment: 1,
            padding: 2,
            format: NumberFormat::Decimal,
        }
    }
}

/// Filter configuration for selecting files.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FilterConfig {
    /// Include/exclude mode.
    pub mode: FilterMode,
    /// Filter rules.
    pub rules: Vec<FilterRule>,
}

/// Filter mode.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub enum FilterMode {
    #[default]
    Include,
    Exclude,
}

/// A single filter rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterRule {
    /// What to filter by.
    pub field: FilterField,
    /// Filter operator.
    pub operator: FilterOperator,
    /// Filter value.
    pub value: String,
}

/// Fields to filter by.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum FilterField {
    Name,
    Extension,
    Path,
    Size,
    Created,
    Modified,
    NameLength,
    PathLength,
}

/// Filter operators.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum FilterOperator {
    Equals,
    NotEquals,
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    Matches,      // Regex
    MatchesGlob,  // Wildcard
    GreaterThan,
    LessThan,
    GreaterOrEqual,
    LessOrEqual,
    Between,
}
