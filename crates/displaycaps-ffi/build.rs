// SPDX-License-Identifier: LGPL-2.1-or-later
// Copied from AudioCaps v1.0.0 (commit 049d053) crates/audiocaps-ffi/build.rs; renamed for
// DisplayCaps.
//! Sets the shared library's soname to `libdisplaycaps.so.<ABI major>`.

fn main() {
    // Keep in step with DC_ABI_VERSION in src/lib.rs.
    println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libdisplaycaps.so.1");
    println!("cargo:rerun-if-changed=build.rs");
}
