//! Stamps the executable with its icon, so that Explorer, a shortcut and
//! "Open with" show Calx as Calx when it is not running.
//!
//! The picture is the one the window shows, drawn by the `brand` crate; here
//! it is written as an `.ico`, wrapped in a resource script and compiled by
//! `windres`, which the GNU toolchain this is built with carries. Without
//! `windres` the build goes on and the executable wears the generic icon: an
//! icon is not worth failing a build over.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    brand_resource::stamp("calx", "Calx", brand::App::Calx);
}

#[path = "../brand/src/resource.rs"]
mod brand_resource;
