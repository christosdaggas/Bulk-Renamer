//! Unified rule configuration dialogs.
//!
//! Every rule type gets exactly one dialog that serves both "add" and "edit":
//! `open(window, kind, None)` creates a new rule, `open(window, kind, Some(i))`
//! edits the rule at index `i` in place. The shared chrome (header buttons,
//! Escape/Enter handling, inline validation errors) lives in `present`.

use crate::core::{
    BracketType, CaseRule, CaseType, CleanupRule, DateSource, DateTimeRule, ExpressionRule,
    InsertPosition, InsertRule, InsertText, MetadataField, MetadataRule, NumberFormat,
    NumberingRule, PadRule, RearrangeRule, RemoveRule, RemoveTarget, ReplaceRule, RuleType,
    TransliterateRule, TransliterationMapping, TrimRule, TrimType,
};
use super::window::RenamerWindow;
use gettextrs::gettext;
use libadwaita as adw;
use adw::prelude::*;
use gtk4 as gtk;
use gtk::gdk;

/// The user-facing rule categories. `DateTime` covers both the legacy
/// `Insert(FileDate)` encoding produced by earlier versions and the native
/// `RuleType::DateTime` used by presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleKind {
    Replace,
    Case,
    Insert,
    Remove,
    Numbering,
    DateTime,
    Trim,
    Pad,
    Cleanup,
    Rearrange,
    Metadata,
    Expression,
    Transliterate,
}

impl RuleKind {
    /// Display metadata: (kind, icon, title, description).
    pub fn catalog() -> &'static [(RuleKind, &'static str, &'static str, &'static str)] {
        &[
            (RuleKind::Replace, "edit-find-replace-symbolic", "Replace Text", "Find and replace text or regex"),
            (RuleKind::Case, "format-text-rich-symbolic", "Change Case", "UPPER, lower, Title Case and more"),
            (RuleKind::Insert, "insert-text-symbolic", "Insert Text", "Add text or folder name at a position"),
            (RuleKind::Remove, "edit-delete-symbolic", "Remove Text", "Remove characters or patterns"),
            (RuleKind::Numbering, "view-list-ordered-symbolic", "Numbering", "Add sequential numbers"),
            (RuleKind::DateTime, "x-office-calendar-symbolic", "Date/Time", "Insert file or current date"),
            (RuleKind::Trim, "edit-cut-symbolic", "Trim", "Trim whitespace or limit length"),
            (RuleKind::Pad, "format-justify-fill-symbolic", "Pad", "Pad the name to a fixed length"),
            (RuleKind::Cleanup, "edit-clear-symbolic", "Clean Up", "Collapse spaces, strip special characters"),
            (RuleKind::Rearrange, "media-playlist-shuffle-symbolic", "Rearrange", "Reorder the parts of the name"),
            (RuleKind::Metadata, "camera-photo-symbolic", "Metadata", "Use EXIF or audio tags in the name"),
            (RuleKind::Expression, "utilities-terminal-symbolic", "Expression", "Template with variables and functions"),
            (RuleKind::Transliterate, "font-x-generic-symbolic", "Transliterate", "Greek/Cyrillic to Latin, strip accents"),
        ]
    }

    /// Classify an existing rule so editing reopens the dialog that made it.
    pub fn of(rule_type: &RuleType) -> RuleKind {
        match rule_type {
            RuleType::Replace(_) => RuleKind::Replace,
            RuleType::ChangeCase(_) => RuleKind::Case,
            RuleType::Insert(i) if matches!(i.text, InsertText::FileDate { .. }) => RuleKind::DateTime,
            RuleType::Insert(_) => RuleKind::Insert,
            RuleType::Remove(_) => RuleKind::Remove,
            RuleType::Numbering(_) => RuleKind::Numbering,
            RuleType::Trim(_) => RuleKind::Trim,
            RuleType::Pad(_) => RuleKind::Pad,
            RuleType::Cleanup(_) => RuleKind::Cleanup,
            RuleType::Rearrange(_) => RuleKind::Rearrange,
            RuleType::Metadata(_) => RuleKind::Metadata,
            RuleType::Expression(_) => RuleKind::Expression,
            RuleType::DateTime(_) => RuleKind::DateTime,
            RuleType::Transliterate(_) => RuleKind::Transliterate,
        }
    }

    fn title(self) -> &'static str {
        Self::catalog()
            .iter()
            .find(|(kind, ..)| *kind == self)
            .map(|(_, _, title, _)| *title)
            .unwrap_or("Rule")
    }
}

/// A built form: the widget tree plus a closure that reads the widgets back
/// into a `RuleType`, or explains why it cannot.
struct Form {
    widget: gtk::Widget,
    collect: Box<dyn Fn() -> Result<RuleType, String>>,
}

/// Open the configuration dialog for `kind`. With `edit_index`, the form is
/// pre-filled from the existing rule and saving replaces it instead of
/// appending.
pub fn open(window: &RenamerWindow, kind: RuleKind, edit_index: Option<usize>) {
    let existing = edit_index
        .and_then(|idx| window.rule_at(idx))
        .map(|rule| rule.rule_type);

    let form = match kind {
        RuleKind::Replace => replace_form(match &existing {
            Some(RuleType::Replace(r)) => Some(r.clone()),
            _ => None,
        }),
        RuleKind::Case => case_form(match &existing {
            Some(RuleType::ChangeCase(c)) => Some(c.clone()),
            _ => None,
        }),
        RuleKind::Insert => insert_form(match &existing {
            Some(RuleType::Insert(i)) => Some(i.clone()),
            _ => None,
        }),
        RuleKind::Remove => remove_form(match &existing {
            Some(RuleType::Remove(r)) => Some(r.clone()),
            _ => None,
        }),
        RuleKind::Numbering => numbering_form(match &existing {
            Some(RuleType::Numbering(n)) => Some(n.clone()),
            _ => None,
        }),
        RuleKind::DateTime => datetime_form(match &existing {
            Some(RuleType::Insert(i)) => DateTimeSeed::from_insert(i),
            Some(RuleType::DateTime(d)) => DateTimeSeed::from_rule(d),
            _ => DateTimeSeed::default(),
        }),
        RuleKind::Trim => trim_form(match &existing {
            Some(RuleType::Trim(t)) => Some(t.clone()),
            _ => None,
        }),
        RuleKind::Pad => pad_form(match &existing {
            Some(RuleType::Pad(p)) => Some(p.clone()),
            _ => None,
        }),
        RuleKind::Cleanup => cleanup_form(match &existing {
            Some(RuleType::Cleanup(c)) => Some(c.clone()),
            _ => None,
        }),
        RuleKind::Rearrange => rearrange_form(match &existing {
            Some(RuleType::Rearrange(r)) => Some(r.clone()),
            _ => None,
        }),
        RuleKind::Metadata => metadata_form(match &existing {
            Some(RuleType::Metadata(m)) => Some(m.clone()),
            _ => None,
        }),
        RuleKind::Expression => expression_form(match &existing {
            Some(RuleType::Expression(e)) => Some(e.clone()),
            _ => None,
        }),
        RuleKind::Transliterate => transliterate_form(match &existing {
            Some(RuleType::Transliterate(t)) => Some(t.clone()),
            _ => None,
        }),
    };

    present(window, kind.title(), form, edit_index);
}

/// Shared dialog chrome: header with Cancel and Add/Save, Escape closes,
/// Enter saves, and validation failures surface inline instead of silently
/// discarding the input.
fn present(window: &RenamerWindow, title: &str, form: Form, edit_index: Option<usize>) {
    let editing = edit_index.is_some();
    let dialog = adw::Window::builder()
        .title(if editing {
            gettext("Edit {} Rule").replacen("{}", &gettext(title), 1)
        } else {
            gettext(title)
        })
        .default_width(420)
        .modal(true)
        .transient_for(window)
        .build();

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_show_end_title_buttons(false);
    header.set_show_start_title_buttons(false);

    let cancel_btn = gtk::Button::with_label(&gettext("Cancel"));
    cancel_btn.add_css_class("flat");
    let save_btn =
        gtk::Button::with_label(&if editing { gettext("Save") } else { gettext("Add Rule") });
    save_btn.add_css_class("suggested-action");
    header.pack_start(&cancel_btn);
    header.pack_end(&save_btn);
    toolbar_view.add_top_bar(&header);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .margin_start(24)
        .margin_end(24)
        .margin_top(12)
        .margin_bottom(24)
        .spacing(18)
        .build();

    let error_label = gtk::Label::builder()
        .css_classes(vec!["error", "caption"])
        .xalign(0.0)
        .wrap(true)
        .visible(false)
        .build();

    content.append(&form.widget);
    content.append(&error_label);

    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(560)
        .child(&content)
        .build();
    toolbar_view.set_content(Some(&scroll));
    dialog.set_content(Some(&toolbar_view));

    let dialog_clone = dialog.clone();
    cancel_btn.connect_clicked(move |_| dialog_clone.close());

    let dialog_clone = dialog.clone();
    let window_clone = window.clone();
    let collect = form.collect;
    save_btn.connect_clicked(move |_| match collect() {
        Ok(rule_type) => {
            window_clone.commit_rule(rule_type, edit_index);
            dialog_clone.close();
        }
        Err(message) => {
            error_label.set_label(&message);
            error_label.set_visible(true);
        }
    });

    // Escape closes, Enter activates the default (save) button.
    let key_controller = gtk::EventControllerKey::new();
    let dialog_clone = dialog.clone();
    key_controller.connect_key_pressed(move |_, keyval, _, _| {
        if keyval == gdk::Key::Escape {
            dialog_clone.close();
            glib_propagation_stop()
        } else {
            glib_propagation_proceed()
        }
    });
    dialog.add_controller(key_controller);
    dialog.set_default_widget(Some(&save_btn));

    dialog.present();
}

fn glib_propagation_stop() -> gtk::glib::Propagation {
    gtk::glib::Propagation::Stop
}

fn glib_propagation_proceed() -> gtk::glib::Propagation {
    gtk::glib::Propagation::Proceed
}

fn entry_row(title: &str, text: &str) -> adw::EntryRow {
    let row = adw::EntryRow::builder().title(title).text(text).build();
    row.set_activates_default(true);
    row
}

/// StringList whose entries are run through gettext at construction time.
/// The source arrays keep the English msgids.
fn translated_string_list(items: &[&str]) -> gtk::StringList {
    let translated: Vec<String> = items.iter().map(|item| gettext(*item)).collect();
    let refs: Vec<&str> = translated.iter().map(String::as_str).collect();
    gtk::StringList::new(&refs)
}

macro_rules! prefs_group {
    ($title:expr $(, $row:expr)* $(,)?) => {{
        let group = adw::PreferencesGroup::new();
        if let Some(title) = $title {
            group.set_title(title);
        }
        $( group.add($row); )*
        group
    }};
}

fn vbox(children: &[&adw::PreferencesGroup]) -> gtk::Widget {
    let box_ = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .build();
    for child in children {
        box_.append(*child);
    }
    box_.into()
}

// ============ Replace ============

fn replace_form(existing: Option<ReplaceRule>) -> Form {
    let seed = existing.unwrap_or_default();

    let find_entry = entry_row(&gettext("Find"), &seed.find);
    let replace_entry = entry_row(&gettext("Replace with"), &seed.replace);
    let case_sensitive = adw::SwitchRow::builder()
        .title(gettext("Case sensitive"))
        .active(seed.case_sensitive)
        .build();
    let use_regex = adw::SwitchRow::builder()
        .title(gettext("Use regular expressions"))
        .active(seed.use_regex)
        .build();
    let replace_all = adw::SwitchRow::builder()
        .title(gettext("Replace all occurrences"))
        .active(seed.replace_all)
        .build();

    let widget = vbox(&[
        &prefs_group!(None::<&str>, &find_entry),
        &prefs_group!(None::<&str>, &replace_entry),
        &prefs_group!(Some(gettext("Options").as_str()), &case_sensitive, &use_regex, &replace_all),
    ]);

    let include_extension = seed.include_extension;
    let collect = Box::new(move || {
        let find = find_entry.text().to_string();
        if find.is_empty() {
            return Err(gettext("Enter the text to find."));
        }
        if use_regex.is_active() {
            regex::Regex::new(&find).map_err(|e| {
                gettext("Invalid regular expression: {}").replacen("{}", &e.to_string(), 1)
            })?;
        }
        Ok(RuleType::Replace(ReplaceRule {
            find,
            replace: replace_entry.text().to_string(),
            use_regex: use_regex.is_active(),
            case_sensitive: case_sensitive.is_active(),
            replace_all: replace_all.is_active(),
            include_extension,
        }))
    });

    Form { widget, collect }
}

// ============ Change Case ============

const CASE_TYPES: &[(CaseType, &str, &str)] = &[
    (CaseType::Lower, "lowercase", "All letters become lowercase"),
    (CaseType::Upper, "UPPERCASE", "All letters become uppercase"),
    (CaseType::Title, "Title Case", "First letter of each word uppercase"),
    (CaseType::Sentence, "Sentence case", "First letter uppercase, rest lowercase"),
    (CaseType::Camel, "camelCase", "Words joined, capitals after the first"),
    (CaseType::Pascal, "PascalCase", "Words joined, every word capitalized"),
    (CaseType::Snake, "snake_case", "Words joined with underscores"),
    (CaseType::Kebab, "kebab-case", "Words joined with hyphens"),
    (CaseType::Constant, "CONSTANT_CASE", "Uppercase words joined with underscores"),
    (CaseType::Capitalize, "Capitalize", "Only the first letter uppercase"),
];

pub(super) fn case_type_label(case_type: CaseType) -> &'static str {
    CASE_TYPES
        .iter()
        .find(|(ct, ..)| (*ct as usize) == (case_type as usize))
        .map(|(_, name, _)| *name)
        .unwrap_or("Custom case")
}

fn case_form(existing: Option<CaseRule>) -> Form {
    let seed = existing.unwrap_or(CaseRule {
        case_type: CaseType::Lower,
        include_extension: false,
    });

    let names: Vec<&str> = CASE_TYPES.iter().map(|(_, name, _)| *name).collect();
    let selected = CASE_TYPES
        .iter()
        .position(|(ct, ..)| (*ct as usize) == (seed.case_type as usize))
        .unwrap_or(0);

    let dropdown = adw::ComboRow::builder()
        .title(gettext("Convert to"))
        .model(&translated_string_list(&names))
        .selected(selected as u32)
        .build();

    let desc_label = gtk::Label::builder()
        .label(gettext(CASE_TYPES[selected].2))
        .css_classes(vec!["dim-label", "caption"])
        .xalign(0.0)
        .build();
    let desc_clone = desc_label.clone();
    dropdown.connect_selected_notify(move |dd| {
        if let Some((_, _, desc)) = CASE_TYPES.get(dd.selected() as usize) {
            desc_clone.set_label(&gettext(*desc));
        }
    });

    let group = prefs_group!(Some(gettext("Case Type").as_str()), &dropdown);
    let box_ = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .build();
    box_.append(&group);
    box_.append(&desc_label);

    let include_extension = seed.include_extension;
    let collect = Box::new(move || {
        let case_type = CASE_TYPES
            .get(dropdown.selected() as usize)
            .map(|(ct, ..)| *ct)
            .unwrap_or(CaseType::Lower);
        Ok(RuleType::ChangeCase(CaseRule {
            case_type,
            include_extension,
        }))
    });

    Form { widget: box_.into(), collect }
}

// ============ Insert ============

fn insert_form(existing: Option<InsertRule>) -> Form {
    let (seed_source, seed_text) = match existing.as_ref().map(|r| &r.text) {
        Some(InsertText::ParentFolder) => (1u32, String::new()),
        Some(InsertText::GrandparentFolder) => (2u32, String::new()),
        Some(InsertText::Fixed(t)) => (0u32, t.clone()),
        _ => (0u32, String::new()),
    };
    let (seed_pos, seed_pos_value) = match existing.as_ref().map(|r| &r.position) {
        Some(InsertPosition::Suffix) => (1u32, 0i32),
        Some(InsertPosition::Position(p)) => (2u32, *p),
        _ => (0u32, 0i32),
    };

    let source_dropdown = adw::ComboRow::builder()
        .title(gettext("Insert"))
        .model(&translated_string_list(&[
            "Custom text",
            "Parent folder name",
            "Grandparent folder name",
        ]))
        .selected(seed_source)
        .build();

    let text_entry = entry_row(&gettext("Text to insert"), &seed_text);
    text_entry.set_sensitive(seed_source == 0);
    let text_clone = text_entry.clone();
    source_dropdown.connect_selected_notify(move |dd| {
        text_clone.set_sensitive(dd.selected() == 0);
    });

    let position_dropdown = adw::ComboRow::builder()
        .title(gettext("Insert at"))
        .model(&translated_string_list(&[
            "Beginning (prefix)",
            "End (suffix)",
            "At position",
        ]))
        .selected(seed_pos)
        .build();

    let position_spin = adw::SpinRow::builder()
        .title(gettext("Character position"))
        .adjustment(&gtk::Adjustment::new(
            seed_pos_value as f64,
            -999.0,
            999.0,
            1.0,
            10.0,
            0.0,
        ))
        .sensitive(seed_pos == 2)
        .build();
    let spin_clone = position_spin.clone();
    position_dropdown.connect_selected_notify(move |dd| {
        spin_clone.set_sensitive(dd.selected() == 2);
    });

    let widget = vbox(&[
        &prefs_group!(None::<&str>, &source_dropdown),
        &prefs_group!(None::<&str>, &text_entry),
        &prefs_group!(Some(gettext("Position").as_str()), &position_dropdown, &position_spin),
    ]);

    let collect = Box::new(move || {
        let text = match source_dropdown.selected() {
            1 => InsertText::ParentFolder,
            2 => InsertText::GrandparentFolder,
            _ => {
                let value = text_entry.text().to_string();
                if value.is_empty() {
                    return Err(gettext("Enter the text to insert."));
                }
                InsertText::Fixed(value)
            }
        };
        let position = match position_dropdown.selected() {
            1 => InsertPosition::Suffix,
            2 => InsertPosition::Position(position_spin.value() as i32),
            _ => InsertPosition::Prefix,
        };
        Ok(RuleType::Insert(InsertRule { text, position }))
    });

    Form { widget, collect }
}

// ============ Remove ============

const BRACKETS: &[(BracketType, &str)] = &[
    (BracketType::All, "All bracket types"),
    (BracketType::Round, "Round ( )"),
    (BracketType::Square, "Square [ ]"),
    (BracketType::Curly, "Curly { }"),
    (BracketType::Angle, "Angle < >"),
];

fn remove_form(existing: Option<RemoveRule>) -> Form {
    // Dropdown index -> RemoveTarget family.
    let (seed_type, seed_text, seed_case, seed_num, seed_bracket) = match existing.map(|r| r.target)
    {
        Some(RemoveTarget::Text { text, case_sensitive }) => (0u32, text, case_sensitive, 1f64, 0u32),
        Some(RemoveTarget::Pattern(p)) => (1, p, true, 1.0, 0),
        Some(RemoveTarget::FirstN(n)) => (2, String::new(), true, n as f64, 0),
        Some(RemoveTarget::LastN(n)) => (3, String::new(), true, n as f64, 0),
        Some(RemoveTarget::Digits) => (4, String::new(), true, 1.0, 0),
        Some(RemoveTarget::Whitespace) => (5, String::new(), true, 1.0, 0),
        Some(RemoveTarget::LeadingZeros) => (6, String::new(), true, 1.0, 0),
        Some(RemoveTarget::Bracketed(bracket)) => (
            7,
            String::new(),
            true,
            1.0,
            BRACKETS
                .iter()
                .position(|(b, _)| std::mem::discriminant(b) == std::mem::discriminant(&bracket))
                .unwrap_or(0) as u32,
        ),
        _ => (0, String::new(), true, 1.0, 0),
    };

    let type_dropdown = adw::ComboRow::builder()
        .title(gettext("Remove"))
        .model(&translated_string_list(&[
            "Specific text",
            "Regex pattern",
            "First N characters",
            "Last N characters",
            "All digits",
            "All whitespace",
            "Leading zeros",
            "Bracketed content",
        ]))
        .selected(seed_type)
        .build();

    let text_entry = entry_row(&gettext("Text or pattern"), &seed_text);
    let case_switch = adw::SwitchRow::builder()
        .title(gettext("Case sensitive"))
        .active(seed_case)
        .build();
    let num_spin = adw::SpinRow::builder()
        .title(gettext("Number of characters"))
        .adjustment(&gtk::Adjustment::new(seed_num, 1.0, 999.0, 1.0, 10.0, 0.0))
        .build();
    let bracket_dropdown = adw::ComboRow::builder()
        .title(gettext("Bracket type"))
        .model(&translated_string_list(
            &BRACKETS.iter().map(|(_, name)| *name).collect::<Vec<_>>(),
        ))
        .selected(seed_bracket)
        .build();

    let update_sensitivity = {
        let text_entry = text_entry.clone();
        let case_switch = case_switch.clone();
        let num_spin = num_spin.clone();
        let bracket_dropdown = bracket_dropdown.clone();
        move |selected: u32| {
            text_entry.set_sensitive(selected <= 1);
            case_switch.set_sensitive(selected == 0);
            num_spin.set_sensitive(selected == 2 || selected == 3);
            bracket_dropdown.set_sensitive(selected == 7);
        }
    };
    update_sensitivity(seed_type);
    let sensitivity = update_sensitivity.clone();
    type_dropdown.connect_selected_notify(move |dd| sensitivity(dd.selected()));

    let widget = vbox(&[
        &prefs_group!(None::<&str>, &type_dropdown),
        &prefs_group!(None::<&str>, &text_entry),
        &prefs_group!(Some(gettext("Options").as_str()), &case_switch, &num_spin, &bracket_dropdown),
    ]);

    let collect = Box::new(move || {
        let target = match type_dropdown.selected() {
            0 => {
                let text = text_entry.text().to_string();
                if text.is_empty() {
                    return Err(gettext("Enter the text to remove."));
                }
                RemoveTarget::Text {
                    text,
                    case_sensitive: case_switch.is_active(),
                }
            }
            1 => {
                let pattern = text_entry.text().to_string();
                if pattern.is_empty() {
                    return Err(gettext("Enter the pattern to remove."));
                }
                regex::Regex::new(&pattern).map_err(|e| {
                    gettext("Invalid regular expression: {}").replacen("{}", &e.to_string(), 1)
                })?;
                RemoveTarget::Pattern(pattern)
            }
            2 => RemoveTarget::FirstN(num_spin.value() as usize),
            3 => RemoveTarget::LastN(num_spin.value() as usize),
            4 => RemoveTarget::Digits,
            5 => RemoveTarget::Whitespace,
            6 => RemoveTarget::LeadingZeros,
            _ => RemoveTarget::Bracketed(
                BRACKETS
                    .get(bracket_dropdown.selected() as usize)
                    .map(|(b, _)| *b)
                    .unwrap_or(BracketType::All),
            ),
        };
        Ok(RuleType::Remove(RemoveRule { target }))
    });

    Form { widget, collect }
}

// ============ Numbering ============

const NUMBER_FORMATS: &[(NumberFormat, &str)] = &[
    (NumberFormat::Decimal, "Decimal (1, 2, 3)"),
    (NumberFormat::Hex, "Hexadecimal (1, a, b)"),
    (NumberFormat::Octal, "Octal"),
    (NumberFormat::Binary, "Binary"),
    (NumberFormat::Roman, "Roman (i, ii, iii)"),
    (NumberFormat::Letter, "Letters (a, b, c)"),
];

fn numbering_form(existing: Option<NumberingRule>) -> Form {
    let seed = existing.unwrap_or_default();

    let start_spin = adw::SpinRow::builder()
        .title(gettext("Start at"))
        .adjustment(&gtk::Adjustment::new(seed.start as f64, 0.0, 999_999.0, 1.0, 10.0, 0.0))
        .build();
    let increment_spin = adw::SpinRow::builder()
        .title(gettext("Increment by"))
        .adjustment(&gtk::Adjustment::new(seed.increment as f64, 1.0, 1000.0, 1.0, 10.0, 0.0))
        .build();
    let padding_spin = adw::SpinRow::builder()
        .title(gettext("Digits (zero-padding)"))
        .adjustment(&gtk::Adjustment::new(seed.padding as f64, 1.0, 10.0, 1.0, 1.0, 0.0))
        .build();
    let format_dropdown = adw::ComboRow::builder()
        .title(gettext("Number format"))
        .model(&translated_string_list(
            &NUMBER_FORMATS.iter().map(|(_, name)| *name).collect::<Vec<_>>(),
        ))
        .selected(
            NUMBER_FORMATS
                .iter()
                .position(|(f, _)| (*f as usize) == (seed.format as usize))
                .unwrap_or(0) as u32,
        )
        .build();

    let is_prefix = matches!(seed.position, InsertPosition::Prefix);
    let position_dropdown = adw::ComboRow::builder()
        .title(gettext("Insert at"))
        .model(&translated_string_list(&["Beginning (prefix)", "End (suffix)"]))
        .selected(if is_prefix { 0 } else { 1 })
        .build();

    // The separator sits between the number and the name, so it maps to
    // `suffix` when the number is a prefix and `prefix` when it is a suffix.
    let seed_separator = if is_prefix { seed.suffix.clone() } else { seed.prefix.clone() };
    let separator_entry = entry_row(&gettext("Separator"), &seed_separator);

    let reset_switch = adw::SwitchRow::builder()
        .title(gettext("Restart numbering per folder"))
        .active(seed.reset_per_folder)
        .build();

    let widget = vbox(&[
        &prefs_group!(Some(gettext("Numbering").as_str()), &start_spin, &increment_spin, &padding_spin),
        &prefs_group!(Some(gettext("Format").as_str()), &format_dropdown),
        &prefs_group!(Some(gettext("Position").as_str()), &position_dropdown, &separator_entry, &reset_switch),
    ]);

    let collect = Box::new(move || {
        let separator = separator_entry.text().to_string();
        let (position, prefix, suffix) = if position_dropdown.selected() == 0 {
            (InsertPosition::Prefix, String::new(), separator)
        } else {
            (InsertPosition::Suffix, separator, String::new())
        };
        Ok(RuleType::Numbering(NumberingRule {
            start: start_spin.value() as i64,
            increment: increment_spin.value() as i64,
            padding: padding_spin.value() as usize,
            position,
            prefix,
            suffix,
            reset_per_folder: reset_switch.is_active(),
            format: NUMBER_FORMATS
                .get(format_dropdown.selected() as usize)
                .map(|(f, _)| *f)
                .unwrap_or(NumberFormat::Decimal),
        }))
    });

    Form { widget, collect }
}

// ============ Date/Time ============

const DATE_SOURCES: &[(DateSource, &str)] = &[
    (DateSource::Modified, "File modified date"),
    (DateSource::Created, "File created date"),
    (DateSource::Accessed, "File accessed date"),
    (DateSource::Now, "Current date"),
    (DateSource::ExifDateTaken, "EXIF date taken"),
];

const DATE_FORMATS: &[(&str, &str)] = &[
    ("%Y-%m-%d", "2026-01-06"),
    ("%Y%m%d", "20260106"),
    ("%d-%m-%Y", "06-01-2026"),
    ("%b %d, %Y", "Jan 06, 2026"),
    ("%Y-%m-%d_%H-%M-%S", "2026-01-06_14-30-00"),
];

/// The DateTime dialog serves two encodings: the legacy `Insert(FileDate)`
/// rules the app has always produced, and native `DateTime` rules found in
/// presets. Saving keeps whichever encoding it was given.
struct DateTimeSeed {
    source: DateSource,
    format: String,
    suffix: bool,
    native: bool,
}

impl Default for DateTimeSeed {
    fn default() -> Self {
        Self {
            source: DateSource::Modified,
            format: DATE_FORMATS[0].0.to_string(),
            suffix: false,
            native: false,
        }
    }
}

impl DateTimeSeed {
    fn from_insert(rule: &InsertRule) -> Self {
        match &rule.text {
            InsertText::FileDate { source, format } => Self {
                source: *source,
                format: format.clone(),
                suffix: matches!(rule.position, InsertPosition::Suffix),
                native: false,
            },
            _ => Self::default(),
        }
    }

    fn from_rule(rule: &DateTimeRule) -> Self {
        Self {
            source: rule.source,
            format: rule.format.clone(),
            suffix: matches!(rule.position, InsertPosition::Suffix),
            native: true,
        }
    }
}

fn datetime_form(seed: DateTimeSeed) -> Form {
    let source_selected = DATE_SOURCES
        .iter()
        .position(|(s, _)| (*s as usize) == (seed.source as usize))
        .unwrap_or(0);
    let source_dropdown = adw::ComboRow::builder()
        .title(gettext("Date source"))
        .model(&translated_string_list(
            &DATE_SOURCES.iter().map(|(_, name)| *name).collect::<Vec<_>>(),
        ))
        .selected(source_selected as u32)
        .build();

    // Known formats select their preset entry; anything else is "Custom".
    // The format examples are literal dates, so they stay untranslated.
    let custom_label = gettext("Custom…");
    let mut format_names: Vec<&str> = DATE_FORMATS.iter().map(|(_, example)| *example).collect();
    format_names.push(custom_label.as_str());
    let known_format = DATE_FORMATS.iter().position(|(fmt, _)| *fmt == seed.format);
    let format_dropdown = adw::ComboRow::builder()
        .title(gettext("Date format"))
        .model(&gtk::StringList::new(&format_names))
        .selected(known_format.unwrap_or(DATE_FORMATS.len()) as u32)
        .build();

    let custom_entry = entry_row(&gettext("Custom format (chrono)"), &seed.format);
    custom_entry.set_sensitive(known_format.is_none());
    let custom_clone = custom_entry.clone();
    format_dropdown.connect_selected_notify(move |dd| {
        custom_clone.set_sensitive(dd.selected() as usize >= DATE_FORMATS.len());
    });

    let position_dropdown = adw::ComboRow::builder()
        .title(gettext("Insert at"))
        .model(&translated_string_list(&["Beginning (prefix)", "End (suffix)"]))
        .selected(if seed.suffix { 1 } else { 0 })
        .build();

    let widget = vbox(&[
        &prefs_group!(None::<&str>, &source_dropdown),
        &prefs_group!(Some(gettext("Format").as_str()), &format_dropdown, &custom_entry, &position_dropdown),
    ]);

    let native = seed.native;
    let collect = Box::new(move || {
        let source = DATE_SOURCES
            .get(source_dropdown.selected() as usize)
            .map(|(s, _)| *s)
            .unwrap_or(DateSource::Modified);
        let format = if (format_dropdown.selected() as usize) < DATE_FORMATS.len() {
            DATE_FORMATS[format_dropdown.selected() as usize].0.to_string()
        } else {
            let value = custom_entry.text().to_string();
            if value.is_empty() {
                return Err(gettext("Enter a chrono date format, e.g. %Y-%m-%d."));
            }
            value
        };
        let position = if position_dropdown.selected() == 1 {
            InsertPosition::Suffix
        } else {
            InsertPosition::Prefix
        };
        if native {
            Ok(RuleType::DateTime(DateTimeRule { source, format, position }))
        } else {
            Ok(RuleType::Insert(InsertRule {
                text: InsertText::FileDate { source, format },
                position,
            }))
        }
    });

    Form { widget, collect }
}

// ============ Trim ============

fn trim_form(existing: Option<TrimRule>) -> Form {
    let (seed_type, seed_chars, seed_len) = match existing {
        Some(TrimRule { trim_type: TrimType::Both, .. }) => (0u32, String::new(), 32f64),
        Some(TrimRule { trim_type: TrimType::Start, .. }) => (1, String::new(), 32.0),
        Some(TrimRule { trim_type: TrimType::End, .. }) => (2, String::new(), 32.0),
        Some(TrimRule { trim_type: TrimType::Characters, characters }) => {
            (3, characters.unwrap_or_default(), 32.0)
        }
        Some(TrimRule { trim_type: TrimType::MaxLength(n), .. }) => (4, String::new(), n as f64),
        None => (0, String::new(), 32.0),
    };

    let type_dropdown = adw::ComboRow::builder()
        .title(gettext("Trim"))
        .model(&translated_string_list(&[
            "Whitespace from both ends",
            "Whitespace from the start",
            "Whitespace from the end",
            "Specific characters from both ends",
            "Truncate to a maximum length",
        ]))
        .selected(seed_type)
        .build();

    let chars_entry = entry_row(&gettext("Characters to trim"), &seed_chars);
    let length_spin = adw::SpinRow::builder()
        .title(gettext("Maximum length"))
        .adjustment(&gtk::Adjustment::new(seed_len, 1.0, 255.0, 1.0, 10.0, 0.0))
        .build();

    let update = {
        let chars_entry = chars_entry.clone();
        let length_spin = length_spin.clone();
        move |selected: u32| {
            chars_entry.set_sensitive(selected == 3);
            length_spin.set_sensitive(selected == 4);
        }
    };
    update(seed_type);
    type_dropdown.connect_selected_notify(move |dd| update(dd.selected()));

    let widget = vbox(&[
        &prefs_group!(None::<&str>, &type_dropdown),
        &prefs_group!(Some(gettext("Options").as_str()), &chars_entry, &length_spin),
    ]);

    let collect = Box::new(move || {
        let (trim_type, characters) = match type_dropdown.selected() {
            1 => (TrimType::Start, None),
            2 => (TrimType::End, None),
            3 => {
                let chars = chars_entry.text().to_string();
                if chars.is_empty() {
                    return Err(gettext("Enter the characters to trim."));
                }
                (TrimType::Characters, Some(chars))
            }
            4 => (TrimType::MaxLength(length_spin.value() as usize), None),
            _ => (TrimType::Both, None),
        };
        Ok(RuleType::Trim(TrimRule { trim_type, characters }))
    });

    Form { widget, collect }
}

// ============ Pad ============

fn pad_form(existing: Option<PadRule>) -> Form {
    let seed = existing.unwrap_or(PadRule {
        length: 8,
        pad_char: '0',
        pad_start: true,
    });

    let length_spin = adw::SpinRow::builder()
        .title(gettext("Target length"))
        .adjustment(&gtk::Adjustment::new(seed.length as f64, 1.0, 255.0, 1.0, 10.0, 0.0))
        .build();
    let char_entry = entry_row(&gettext("Padding character"), &seed.pad_char.to_string());
    let direction_dropdown = adw::ComboRow::builder()
        .title(gettext("Pad at"))
        .model(&translated_string_list(&["Start", "End"]))
        .selected(if seed.pad_start { 0 } else { 1 })
        .build();

    let widget = vbox(&[&prefs_group!(
        Some(gettext("Padding").as_str()),
        &length_spin,
        &char_entry,
        &direction_dropdown
    )]);

    let collect = Box::new(move || {
        let pad_char = char_entry
            .text()
            .chars()
            .next()
            .ok_or_else(|| gettext("Enter a padding character."))?;
        Ok(RuleType::Pad(PadRule {
            length: length_spin.value() as usize,
            pad_char,
            pad_start: direction_dropdown.selected() == 0,
        }))
    });

    Form { widget, collect }
}

// ============ Cleanup ============

const SPACE_REPLACEMENTS: &[(Option<char>, &str)] = &[
    (None, "Keep spaces"),
    (Some('_'), "Replace with underscore"),
    (Some('-'), "Replace with hyphen"),
    (Some('.'), "Replace with dot"),
];

fn cleanup_form(existing: Option<CleanupRule>) -> Form {
    let seed = existing.unwrap_or_default();

    let collapse_switch = adw::SwitchRow::builder()
        .title(gettext("Collapse repeated spaces"))
        .active(seed.collapse_spaces)
        .build();
    let special_switch = adw::SwitchRow::builder()
        .title(gettext("Remove special characters"))
        .active(seed.remove_special)
        .build();
    let preserve_entry = entry_row(&gettext("Characters to preserve"), &seed.preserve);
    let diacritics_switch = adw::SwitchRow::builder()
        .title(gettext("Remove accents and diacritics"))
        .active(seed.remove_diacritics)
        .build();
    let normalize_switch = adw::SwitchRow::builder()
        .title(gettext("Normalize Unicode"))
        .active(seed.normalize_unicode)
        .build();
    let space_dropdown = adw::ComboRow::builder()
        .title(gettext("Spaces"))
        .model(&translated_string_list(
            &SPACE_REPLACEMENTS.iter().map(|(_, name)| *name).collect::<Vec<_>>(),
        ))
        .selected(
            SPACE_REPLACEMENTS
                .iter()
                .position(|(c, _)| *c == seed.space_replacement)
                .unwrap_or(0) as u32,
        )
        .build();

    let widget = vbox(&[&prefs_group!(Some(gettext("Clean Up").as_str()), &collapse_switch,
            &space_dropdown,
            &special_switch,
            &preserve_entry,
            &diacritics_switch,
            &normalize_switch)]);

    let collect = Box::new(move || {
        Ok(RuleType::Cleanup(CleanupRule {
            collapse_spaces: collapse_switch.is_active(),
            remove_special: special_switch.is_active(),
            preserve: preserve_entry.text().to_string(),
            space_replacement: SPACE_REPLACEMENTS
                .get(space_dropdown.selected() as usize)
                .and_then(|(c, _)| *c),
            remove_diacritics: diacritics_switch.is_active(),
            normalize_unicode: normalize_switch.is_active(),
        }))
    });

    Form { widget, collect }
}

// ============ Rearrange ============

fn rearrange_form(existing: Option<RearrangeRule>) -> Form {
    let seed = existing.unwrap_or(RearrangeRule {
        separator: "_".to_string(),
        order: vec![1, 0],
        new_separator: "_".to_string(),
    });

    let separator_entry = entry_row(&gettext("Split on"), &seed.separator);
    let order_text = seed
        .order
        .iter()
        .map(|i| (i + 1).to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let order_entry = entry_row(&gettext("New order (e.g. 2, 1, 3)"), &order_text);
    let new_separator_entry = entry_row(&gettext("Join with"), &seed.new_separator);

    let help = gtk::Label::builder()
        .label(gettext("The name is split on the separator; parts are numbered from 1. Parts left out of the order are dropped."))
        .css_classes(vec!["dim-label", "caption"])
        .xalign(0.0)
        .wrap(true)
        .build();

    let box_ = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .build();
    box_.append(&prefs_group!(
        Some(gettext("Rearrange").as_str()),
        &separator_entry,
        &order_entry,
        &new_separator_entry
    ));
    box_.append(&help);

    let collect = Box::new(move || {
        let separator = separator_entry.text().to_string();
        if separator.is_empty() {
            return Err(gettext("Enter the separator to split on."));
        }
        let order: Result<Vec<usize>, String> = order_entry
            .text()
            .split(',')
            .map(|part| {
                part.trim()
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| n.checked_sub(1))
                    .ok_or_else(|| {
                        gettext(
                            "The order must be a comma-separated list of part numbers, starting at 1.",
                        )
                    })
            })
            .collect();
        let order = order?;
        if order.is_empty() {
            return Err(gettext("Enter at least one part number."));
        }
        Ok(RuleType::Rearrange(RearrangeRule {
            separator,
            order,
            new_separator: new_separator_entry.text().to_string(),
        }))
    });

    Form { widget: box_.into(), collect }
}

// ============ Metadata ============

const METADATA_FIELDS: &[(MetadataField, &str)] = &[
    (MetadataField::ExifDateTaken, "EXIF: Date taken"),
    (MetadataField::ExifCameraMake, "EXIF: Camera make"),
    (MetadataField::ExifCameraModel, "EXIF: Camera model"),
    (MetadataField::ExifISO, "EXIF: ISO"),
    (MetadataField::ExifWidth, "EXIF: Image width"),
    (MetadataField::ExifHeight, "EXIF: Image height"),
    (MetadataField::Id3Title, "Audio: Title"),
    (MetadataField::Id3Artist, "Audio: Artist"),
    (MetadataField::Id3Album, "Audio: Album"),
    (MetadataField::Id3Year, "Audio: Year"),
    (MetadataField::Id3Track, "Audio: Track number"),
    (MetadataField::Id3Genre, "Audio: Genre"),
    (MetadataField::FileSize, "File: Size"),
    (MetadataField::FileModified, "File: Modified date"),
    (MetadataField::FileCreated, "File: Created date"),
    (MetadataField::FileParent, "File: Parent folder"),
    (MetadataField::FileExtension, "File: Extension"),
];

pub(super) fn metadata_field_label(field: &MetadataField) -> &'static str {
    METADATA_FIELDS
        .iter()
        .find(|(f, _)| std::mem::discriminant(f) == std::mem::discriminant(field))
        .map(|(_, name)| *name)
        .unwrap_or("Metadata")
}

fn metadata_form(existing: Option<MetadataRule>) -> Form {
    let seed = existing.unwrap_or(MetadataRule {
        field: MetadataField::ExifDateTaken,
        format: None,
        fallback: String::new(),
        position: InsertPosition::Prefix,
    });

    let selected = METADATA_FIELDS
        .iter()
        .position(|(f, _)| std::mem::discriminant(f) == std::mem::discriminant(&seed.field))
        .unwrap_or(0);
    let field_dropdown = adw::ComboRow::builder()
        .title(gettext("Metadata field"))
        .model(&translated_string_list(
            &METADATA_FIELDS.iter().map(|(_, name)| *name).collect::<Vec<_>>(),
        ))
        .selected(selected as u32)
        .build();

    let format_entry = entry_row(
        &gettext("Format (optional, for dates)"),
        seed.format.as_deref().unwrap_or(""),
    );
    let fallback_entry = entry_row(&gettext("Fallback if missing"), &seed.fallback);
    let position_dropdown = adw::ComboRow::builder()
        .title(gettext("Insert at"))
        .model(&translated_string_list(&["Beginning (prefix)", "End (suffix)"]))
        .selected(if matches!(seed.position, InsertPosition::Suffix) { 1 } else { 0 })
        .build();

    let widget = vbox(&[
        &prefs_group!(None::<&str>, &field_dropdown),
        &prefs_group!(Some(gettext("Options").as_str()), &format_entry, &fallback_entry, &position_dropdown),
    ]);

    let collect = Box::new(move || {
        let field = METADATA_FIELDS
            .get(field_dropdown.selected() as usize)
            .map(|(f, _)| f.clone())
            .unwrap_or(MetadataField::ExifDateTaken);
        let format = {
            let value = format_entry.text().to_string();
            if value.is_empty() { None } else { Some(value) }
        };
        let position = if position_dropdown.selected() == 1 {
            InsertPosition::Suffix
        } else {
            InsertPosition::Prefix
        };
        Ok(RuleType::Metadata(MetadataRule {
            field,
            format,
            fallback: fallback_entry.text().to_string(),
            position,
        }))
    });

    Form { widget, collect }
}

// ============ Expression ============

fn expression_form(existing: Option<ExpressionRule>) -> Form {
    let seed = existing.unwrap_or(ExpressionRule {
        expression: String::new(),
    });

    let expression_entry = entry_row(&gettext("Expression"), &seed.expression);

    let help = gtk::Label::builder()
        .label(gettext(
            "Variables: {name}, {ext}, {parent}, {size}, {counter}, {date}.\n\
             Functions: ${upper(name)}, ${lower(name)}, ${replace(name, \"a\", \"b\")}, \
             ${substr(name, 0, 5)}, ${trim(name)}, ${pad(counter, 3)}.",
        ))
        .css_classes(vec!["dim-label", "caption"])
        .xalign(0.0)
        .wrap(true)
        .build();

    let box_ = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .build();
    box_.append(&prefs_group!(Some(gettext("Template").as_str()), &expression_entry));
    box_.append(&help);

    let collect = Box::new(move || {
        let expression = expression_entry.text().to_string();
        if expression.is_empty() {
            return Err(gettext("Enter an expression template."));
        }
        Ok(RuleType::Expression(ExpressionRule { expression }))
    });

    Form { widget: box_.into(), collect }
}

// ============ Transliterate ============

const TRANSLITERATIONS: &[(TransliterationMapping, &str)] = &[
    (TransliterationMapping::GreekToLatin, "Greek to Latin"),
    (TransliterationMapping::CyrillicToLatin, "Cyrillic to Latin"),
    (TransliterationMapping::RemoveDiacritics, "Remove accents and diacritics"),
    (TransliterationMapping::NormalizeUnicode, "Normalize Unicode"),
];

fn transliterate_form(existing: Option<TransliterateRule>) -> Form {
    let seed = existing.unwrap_or(TransliterateRule {
        mapping: TransliterationMapping::GreekToLatin,
    });

    let dropdown = adw::ComboRow::builder()
        .title(gettext("Conversion"))
        .model(&translated_string_list(
            &TRANSLITERATIONS.iter().map(|(_, name)| *name).collect::<Vec<_>>(),
        ))
        .selected(
            TRANSLITERATIONS
                .iter()
                .position(|(m, _)| (*m as usize) == (seed.mapping as usize))
                .unwrap_or(0) as u32,
        )
        .build();

    let widget = vbox(&[&prefs_group!(Some(gettext("Transliterate").as_str()), &dropdown)]);

    let collect = Box::new(move || {
        Ok(RuleType::Transliterate(TransliterateRule {
            mapping: TRANSLITERATIONS
                .get(dropdown.selected() as usize)
                .map(|(m, _)| *m)
                .unwrap_or(TransliterationMapping::GreekToLatin),
        }))
    });

    Form { widget, collect }
}

// ============ Rule summaries ============

/// (title, subtitle, icon) used for the rule rows in the Rules panel. Covers
/// every rule type so presets round-trip without falling back to "Custom rule".
pub fn rule_summary(rule_type: &RuleType) -> (String, String, String) {
    match rule_type {
        RuleType::Replace(r) => {
            let subtitle = if r.replace.is_empty() {
                gettext("Remove \"{}\"").replacen("{}", &r.find, 1)
            } else {
                format!("\"{}\" → \"{}\"", r.find, r.replace)
            };
            (gettext("Replace"), subtitle, "edit-find-replace-symbolic".into())
        }
        RuleType::ChangeCase(c) => (
            gettext("Change Case"),
            gettext(case_type_label(c.case_type)),
            "format-text-rich-symbolic".into(),
        ),
        RuleType::Insert(i) if matches!(i.text, InsertText::FileDate { .. }) => (
            gettext("Date/Time"),
            datetime_subtitle_for(i),
            "x-office-calendar-symbolic".into(),
        ),
        RuleType::Insert(i) => {
            let text = match &i.text {
                InsertText::Fixed(t) => format!("\"{}\"", t),
                InsertText::ParentFolder => gettext("Parent folder name"),
                InsertText::GrandparentFolder => gettext("Grandparent folder name"),
                _ => gettext("Dynamic"),
            };
            let pos = match &i.position {
                InsertPosition::Prefix => gettext("at beginning"),
                InsertPosition::Suffix => gettext("at end"),
                InsertPosition::Position(p) => {
                    gettext("at position {}").replacen("{}", &p.to_string(), 1)
                }
                _ => gettext("custom"),
            };
            (gettext("Insert"), format!("{} {}", text, pos), "insert-text-symbolic".into())
        }
        RuleType::Remove(r) => {
            let subtitle = match &r.target {
                RemoveTarget::Text { text, .. } => format!("\"{}\"", text),
                RemoveTarget::Pattern(p) => gettext("Pattern /{}/").replacen("{}", p, 1),
                RemoveTarget::FirstN(n) => {
                    gettext("First {} chars").replacen("{}", &n.to_string(), 1)
                }
                RemoveTarget::LastN(n) => {
                    gettext("Last {} chars").replacen("{}", &n.to_string(), 1)
                }
                RemoveTarget::Digits => gettext("All digits"),
                RemoveTarget::Whitespace => gettext("All whitespace"),
                RemoveTarget::LeadingZeros => gettext("Leading zeros"),
                RemoveTarget::Bracketed(_) => gettext("Bracketed content"),
                _ => gettext("Custom"),
            };
            (gettext("Remove"), subtitle, "edit-delete-symbolic".into())
        }
        RuleType::Numbering(n) => {
            let pos = match &n.position {
                InsertPosition::Prefix => gettext("prefix"),
                InsertPosition::Suffix => gettext("suffix"),
                _ => gettext("custom"),
            };
            (
                gettext("Numbering"),
                gettext("Start: {}, Pad: {} digits, {}")
                    .replacen("{}", &n.start.to_string(), 1)
                    .replacen("{}", &n.padding.to_string(), 1)
                    .replacen("{}", &pos, 1),
                "view-list-ordered-symbolic".into(),
            )
        }
        RuleType::Trim(t) => {
            let subtitle = match t.trim_type {
                TrimType::Both => gettext("Whitespace from both ends"),
                TrimType::Start => gettext("Whitespace from the start"),
                TrimType::End => gettext("Whitespace from the end"),
                TrimType::Characters => gettext("Characters: {}")
                    .replacen("{}", t.characters.as_deref().unwrap_or(""), 1),
                TrimType::MaxLength(n) => {
                    gettext("Max length {}").replacen("{}", &n.to_string(), 1)
                }
            };
            (gettext("Trim"), subtitle, "edit-cut-symbolic".into())
        }
        RuleType::Pad(p) => (
            gettext("Pad"),
            gettext("To {} chars with '{}' at {}")
                .replacen("{}", &p.length.to_string(), 1)
                .replacen("{}", &p.pad_char.to_string(), 1)
                .replacen(
                    "{}",
                    &if p.pad_start { gettext("start") } else { gettext("end") },
                    1,
                ),
            "format-justify-fill-symbolic".into(),
        ),
        RuleType::Cleanup(c) => {
            let mut parts = Vec::new();
            if c.collapse_spaces {
                parts.push(gettext("collapse spaces"));
            }
            if c.remove_special {
                parts.push(gettext("strip special chars"));
            }
            if c.remove_diacritics {
                parts.push(gettext("strip accents"));
            }
            if c.space_replacement.is_some() {
                parts.push(gettext("replace spaces"));
            }
            let subtitle = if parts.is_empty() {
                gettext("Normalize")
            } else {
                parts.join(", ")
            };
            (gettext("Clean Up"), subtitle, "edit-clear-symbolic".into())
        }
        RuleType::Rearrange(r) => (
            gettext("Rearrange"),
            gettext("Split on \"{}\", order {}")
                .replacen("{}", &r.separator, 1)
                .replacen(
                    "{}",
                    &r.order
                        .iter()
                        .map(|i| (i + 1).to_string())
                        .collect::<Vec<_>>()
                        .join("-"),
                    1,
                ),
            "media-playlist-shuffle-symbolic".into(),
        ),
        RuleType::DateTime(d) => {
            let source = DATE_SOURCES
                .iter()
                .find(|(s, _)| (*s as usize) == (d.source as usize))
                .map(|(_, name)| gettext(*name))
                .unwrap_or_else(|| gettext("Date"));
            let pos = if matches!(d.position, InsertPosition::Suffix) {
                gettext("suffix")
            } else {
                gettext("prefix")
            };
            (
                gettext("Date/Time"),
                gettext("{} as {}")
                    .replacen("{}", &source, 1)
                    .replacen("{}", &pos, 1),
                "x-office-calendar-symbolic".into(),
            )
        }
        RuleType::Metadata(m) => (
            gettext("Metadata"),
            gettext(metadata_field_label(&m.field)),
            "camera-photo-symbolic".into(),
        ),
        RuleType::Expression(e) => (
            gettext("Expression"),
            e.expression.clone(),
            "utilities-terminal-symbolic".into(),
        ),
        RuleType::Transliterate(t) => (
            gettext("Transliterate"),
            TRANSLITERATIONS
                .iter()
                .find(|(m, _)| (*m as usize) == (t.mapping as usize))
                .map(|(_, name)| gettext(*name))
                .unwrap_or_else(|| gettext("Conversion")),
            "font-x-generic-symbolic".into(),
        ),
    }
}

/// Subtitle for the legacy `Insert(FileDate)` encoding; kept byte-identical to
/// the strings the app has always shown so rebuilt rows match created ones.
pub fn datetime_subtitle_for(rule: &InsertRule) -> String {
    let source_name = match &rule.text {
        InsertText::FileDate { source, .. } => match source {
            DateSource::Modified => gettext("Modified date"),
            DateSource::Created => gettext("Created date"),
            DateSource::Now => gettext("Current date"),
            DateSource::ExifDateTaken => gettext("EXIF date"),
            DateSource::Accessed => gettext("Accessed date"),
        },
        _ => gettext("Date"),
    };
    let pos_name = match rule.position {
        InsertPosition::Suffix => gettext("suffix"),
        _ => gettext("prefix"),
    };
    gettext("{} as {}")
        .replacen("{}", &source_name, 1)
        .replacen("{}", &pos_name, 1)
}
