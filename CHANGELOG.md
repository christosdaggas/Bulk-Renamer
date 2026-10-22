# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0] - 2026-10-22

First release, made for Fedora with the GNOME desktop (tested on Fedora 44).

### Added
- Rule-based bulk renaming with 13 rule types: replace (plain text or regular
  expressions), insert, remove, change case, numbering, trim, pad, date/time,
  expression templates, rearrange, photo and music metadata, cleanup, and
  transliteration (Greek and Cyrillic to Latin).
- Live preview with per-file status, conflict detection and automatic
  resolution of name clashes.
- Safe two-phase renaming, so swaps and rotations work, plus a crash-recovery
  journal that restores files when a rename is interrupted.
- Undo and redo, a rename history browser with per-batch undo, and export of
  an undo shell script.
- Presets (built-in and your own), CSV import of old-name/new-name pairs and
  CSV export of the preview.
- Natural sorting, name search and extension filters, per-file exclusion and
  inline new-name overrides.
- Expression templates with file, date, counter, content-hash (`${sha256}`,
  `${md5}`, …) and EXIF/ID3 variables.
- Headless command line: `preview`, `apply` and `list-presets`.
- Partial translations for Arabic, Chinese, French, German, Greek, Hindi,
  Italian, Portuguese, Russian and Spanish.
- RPM package for Fedora, built by `scripts/package-rpm.sh`.
