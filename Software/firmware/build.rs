//! Build script: puts memory.x where the linker can find it.
//!
//! cortex-m-rt's link.x includes memory.x, which the linker looks for in its search
//! path. Copying it into OUT_DIR and adding that to the search path means the
//! build doesn't depend on which directory cargo was started from.

use std::{env, fs, path::PathBuf};

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::copy("memory.x", out.join("memory.x")).expect("could not copy memory.x");
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");
}
