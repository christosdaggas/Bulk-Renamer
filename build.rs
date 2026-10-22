//! Build script for Bulk Renamer
//!
//! This compiles the GResource XML file into a binary resource.

use std::process::Command;
use std::path::Path;

fn main() {
    // Tell cargo to re-run this script if the source files change
    println!("cargo:rerun-if-changed=data/resources/style.css");
    println!("cargo:rerun-if-changed=data/resources/bulk-renamer.gresource.xml");

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let gresource_xml = "data/resources/bulk-renamer.gresource.xml";
    let gresource_out = Path::new(&out_dir).join("bulk-renamer.gresource");

    // Compile the GResource
    let status = Command::new("glib-compile-resources")
        .arg("--sourcedir=data/resources")
        .arg(format!("--target={}", gresource_out.display()))
        .arg(gresource_xml)
        .status();

    match status {
        Ok(status) if status.success() => {
            println!("cargo:rustc-env=GRESOURCE_FILE={}", gresource_out.display());
        }
        Ok(status) => {
            panic!("glib-compile-resources failed with status: {}", status);
        }
        Err(e) => {
            panic!(
                "Failed to run glib-compile-resources: {}. Make sure glib2-devel (or libglib2.0-dev) is installed.",
                e
            );
        }
    }
}
