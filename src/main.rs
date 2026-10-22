//! Bulk Renamer - A bulk file renaming application for GNOME.
//!
//! This is the main entry point for the application.

use gtk4 as gtk;
use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use libadwaita as adw;

use bulk_renamer::app::RenamerApplication;
use bulk_renamer::undo::logging::init_tracing;

/// Application ID following reverse-DNS convention
pub const APP_ID: &str = "com.chrisdaggas.bulk-renamer";

/// Human-readable application name
pub const APP_NAME: &str = "Bulk Renamer";

fn main() -> glib::ExitCode {
    // Headless CLI mode: `preview`, `apply`, `list-presets`, --help and
    // --version are handled before any GTK/libadwaita initialization. Plain
    // file/directory arguments fall through to the GUI (HANDLES_OPEN) as before.
    // `args_os`, because `std::env::args` panics on an argument that is not UTF-8.
    if let Some(code) = bulk_renamer::cli::run(std::env::args_os().skip(1).collect()) {
        std::process::exit(code);
    }

    // Set the program name to match StartupWMClass in the .desktop file
    // This is critical for Wayland/GNOME Shell to match the window to the correct icon
    glib::set_prgname(Some(APP_ID));
    glib::set_application_name(APP_NAME);

    // Initialize tracing/logging
    init_tracing("info");

    // Translations: the .mo catalogues live under the install prefix; the env
    // override serves development builds (BULK_RENAMER_LOCALEDIR=target/locale).
    init_i18n();

    // Initialize libadwaita
    adw::init().expect("Failed to initialize libadwaita");

    // Create the application
    let app = RenamerApplication::new(
        APP_ID,
        gio::ApplicationFlags::HANDLES_OPEN,
    );

    // Connect to startup signal to load CSS after GTK is initialized
    app.connect_startup(|_| {
        register_resources();
        load_css();
    });

    // gio's `run()` reads `std::env::args`, which panics on an argument that is not
    // UTF-8, and `run_with_args` only takes text, so such a path cannot reach the
    // window. It is dropped with a warning instead of taking the app down.
    app.run_with_args(&utf8_args())
}

/// The process arguments that are valid UTF-8, with a warning for each one dropped.
fn utf8_args() -> Vec<String> {
    let mut args = Vec::new();
    for (index, arg) in std::env::args_os().enumerate() {
        match arg.into_string() {
            Ok(arg) => args.push(arg),
            // The program name only has to be there.
            Err(_) if index == 0 => args.push("bulk-renamer".to_string()),
            Err(arg) => tracing::warn!(
                "Ignoring an argument that is not valid UTF-8: {}",
                std::path::Path::new(&arg).display()
            ),
        }
    }
    args
}

fn init_i18n() {
    use gettextrs::{LocaleCategory, bind_textdomain_codeset, bindtextdomain, setlocale, textdomain};

    setlocale(LocaleCategory::LcAll, "");
    let locale_dir = std::env::var("BULK_RENAMER_LOCALEDIR")
        .unwrap_or_else(|_| "/usr/share/locale".to_string());
    let _ = bindtextdomain("bulk-renamer", locale_dir);
    let _ = bind_textdomain_codeset("bulk-renamer", "UTF-8");
    let _ = textdomain("bulk-renamer");
}

fn register_resources() {
    let bytes = glib::Bytes::from_static(include_bytes!(env!("GRESOURCE_FILE")));
    match gio::Resource::from_data(&bytes) {
        Ok(resource) => gio::resources_register(&resource),
        Err(err) => tracing::error!("Failed to register resources: {}", err),
    }
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    
    // Try to load from compiled GResource first, fallback to file system
    let gresource_path = "/com/chrisdaggas/BulkRenamer/style.css";
    
    // Check if the resource exists
    if gio::resources_lookup_data(gresource_path, gio::ResourceLookupFlags::NONE).is_ok() {
        provider.load_from_resource(gresource_path);
    } else {
        // Fallback to file system for development
        let css_paths = [
            "data/resources/style.css",
            "../data/resources/style.css",
            "../../data/resources/style.css",
        ];
        
        for path in &css_paths {
            let css_path = std::path::Path::new(path);
            if css_path.exists() {
                provider.load_from_path(css_path);
                break;
            }
        }
    }

    // Handle headless environments gracefully
    let display = match gdk::Display::default() {
        Some(d) => d,
        None => {
            tracing::error!("No display found. This application requires a graphical environment.");
            return;
        }
    };

    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // Get the style manager for theme change detection
    let style_manager = adw::StyleManager::default();

    // Clone provider for closures
    let provider_weak = provider.downgrade();

    // Reload CSS on color scheme changes (light/dark mode)
    let provider_clone = provider_weak.clone();
    style_manager.connect_color_scheme_notify(move |_| {
        if let Some(provider) = provider_clone.upgrade() {
            reload_css_provider(&provider);
        }
    });

    // Reload CSS on dark mode toggle
    let provider_clone = provider_weak.clone();
    style_manager.connect_dark_notify(move |_| {
        if let Some(provider) = provider_clone.upgrade() {
            reload_css_provider(&provider);
        }
    });

    // Reload CSS on high contrast changes
    let provider_clone = provider_weak.clone();
    style_manager.connect_high_contrast_notify(move |_| {
        if let Some(provider) = provider_clone.upgrade() {
            reload_css_provider(&provider);
        }
    });

    // Listen to GTK settings for additional theme changes
    if let Some(settings) = gtk::Settings::default() {
        let provider_clone = provider_weak.clone();
        settings.connect_gtk_theme_name_notify(move |_| {
            if let Some(provider) = provider_clone.upgrade() {
                reload_css_provider(&provider);
            }
        });
    }
}

fn reload_css_provider(provider: &gtk::CssProvider) {
    let gresource_path = "/com/chrisdaggas/BulkRenamer/style.css";
    
    if gio::resources_lookup_data(gresource_path, gio::ResourceLookupFlags::NONE).is_ok() {
        provider.load_from_resource(gresource_path);
    } else {
        let css_paths = [
            "data/resources/style.css",
            "../data/resources/style.css",
            "../../data/resources/style.css",
        ];
        
        for path in &css_paths {
            let css_path = std::path::Path::new(path);
            if css_path.exists() {
                provider.load_from_path(css_path);
                break;
            }
        }
    }
}
