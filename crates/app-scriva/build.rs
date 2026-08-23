//! Stamps the executable with its icon, so that Explorer, a shortcut and
//! "Open with" show Scriva as Scriva when it is not running. See Calx's
//! build script: the two are the same step.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    brand_resource::stamp("scriva", "Scriva", brand::App::Scriva);
}

#[path = "../brand/src/resource.rs"]
mod brand_resource;
