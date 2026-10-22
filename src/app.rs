//! Main application module.

use libadwaita as adw;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk4 as gtk;
use gtk::{gio, glib};

use crate::ui::RenamerWindow;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct RenamerApplication {}

    #[glib::object_subclass]
    impl ObjectSubclass for RenamerApplication {
        const NAME: &'static str = "RenamerApplication";
        type Type = super::RenamerApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for RenamerApplication {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_actions();
            obj.setup_accelerators();
        }
    }

    impl ApplicationImpl for RenamerApplication {
        fn startup(&self) {
            self.parent_startup();
            
            // Add icon search paths for development mode
            // When installed, icons should be in /usr/share/icons/hicolor (default search path)
            if let Some(display) = gtk::gdk::Display::default() {
                let icon_theme = gtk::IconTheme::for_display(&display);
                
                // Try to find icons relative to the executable (for development/portable use)
                if let Ok(exe_path) = std::env::current_exe() {
                    if let Some(exe_dir) = exe_path.parent() {
                        // Check for icons relative to executable: ../../data/icons (from target/release/)
                        let dev_icons = exe_dir.join("../../data/icons");
                        if dev_icons.exists() {
                            if let Some(path_str) = dev_icons.canonicalize().ok().and_then(|p| p.to_str().map(String::from)) {
                                icon_theme.add_search_path(&path_str);
                            }
                        }
                    }
                }
                
                // Also check current working directory (for cargo run)
                icon_theme.add_search_path("data/icons");
            }
            
            // Set the default icon for all windows in the application
            gtk::Window::set_default_icon_name("com.chrisdaggas.bulk-renamer");
        }

        fn activate(&self) {
            let application = self.obj();
            
            // Get the current window or create a new one
            let window = if let Some(window) = application.active_window() {
                window
            } else {
                let window = RenamerWindow::new(&*application);
                window.upcast()
            };

            window.present();
        }

        fn open(&self, files: &[gio::File], _hint: &str) {
            let application = self.obj();
            application.activate();

            if let Some(window) = application.active_window() {
                if let Ok(renamer_window) = window.downcast::<RenamerWindow>() {
                    for file in files {
                        if let Some(path) = file.path() {
                            renamer_window.add_path(path);
                        }
                    }
                }
            }
        }

        fn shutdown(&self) {
            tracing::info!("Application shutting down");
            
            // Save settings on shutdown
            if let Some(window) = self.obj().active_window() {
                if let Ok(renamer_window) = window.downcast::<RenamerWindow>() {
                    renamer_window.save_on_shutdown();
                }
            }
            
            self.parent_shutdown();
        }
    }

    impl GtkApplicationImpl for RenamerApplication {}
    impl AdwApplicationImpl for RenamerApplication {}
}

glib::wrapper! {
    pub struct RenamerApplication(ObjectSubclass<imp::RenamerApplication>)
        @extends adw::Application, gtk::Application, gio::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl RenamerApplication {
    pub fn new(application_id: &str, flags: gio::ApplicationFlags) -> Self {
        glib::Object::builder()
            .property("application-id", application_id)
            .property("flags", flags)
            .build()
    }

    fn setup_actions(&self) {
        // Quit action
        let quit_action = gio::SimpleAction::new("quit", None);
        let app = self.clone();
        quit_action.connect_activate(move |_, _| {
            // Quitting stops the main loop at once, even in the middle of a batch,
            // which would leave files under their temporary names.
            if let Some(window) = app
                .windows()
                .into_iter()
                .filter_map(|window| window.downcast::<RenamerWindow>().ok())
                .find(|window| window.is_busy())
            {
                window.show_toast(&gettextrs::gettext("Wait until the current operation finishes"));
                return;
            }
            app.quit();
        });
        self.add_action(&quit_action);

        // About action (app-level)
        let about_action = gio::SimpleAction::new("about", None);
        let app = self.clone();
        about_action.connect_activate(move |_, _| {
            if let Some(window) = app.active_window() {
                let about = crate::ui::dialogs::create_about_dialog();
                about.set_transient_for(Some(&window));
                about.present();
            }
        });
        self.add_action(&about_action);
    }

    fn setup_accelerators(&self) {
        // Window actions
        self.set_accels_for_action("win.add-files", &["<Control>o"]);
        self.set_accels_for_action("win.add-folder", &["<Control><Shift>o"]);
        self.set_accels_for_action("win.execute-rename", &["<Control>Return"]);
        self.set_accels_for_action("win.undo", &["<Control>z"]);
        self.set_accels_for_action("win.redo", &["<Control><Shift>z"]);
        self.set_accels_for_action("win.clear-files", &["<Control><Shift>Delete"]);
        self.set_accels_for_action("win.preferences", &["<Control>comma"]);
        self.set_accels_for_action("win.load-preset", &["<Control>l"]);
        self.set_accels_for_action("win.save-preset", &["<Control>s"]);

        // App actions
        self.set_accels_for_action("app.quit", &["<Control>q"]);
        
        // Quick actions
        self.set_accels_for_action("win.quick-lowercase", &["<Control>1"]);
        self.set_accels_for_action("win.quick-uppercase", &["<Control>2"]);
        self.set_accels_for_action("win.quick-titlecase", &["<Control>3"]);
        self.set_accels_for_action("win.quick-number", &["<Control>4"]);
        self.set_accels_for_action("win.quick-cleanup", &["<Control>5"]);
        self.set_accels_for_action("win.clear-rules", &["<Control><Shift>k"]);
        self.set_accels_for_action("win.show-help-overlay", &["<Control>question"]);
    }
}

impl Default for RenamerApplication {
    fn default() -> Self {
        Self::new(
            "com.chrisdaggas.bulk-renamer",
            gio::ApplicationFlags::HANDLES_OPEN,
        )
    }
}
