fn main() {
    // The screencapturekit crate bridges through Swift, so the binary links
    // against the Swift runtime (libswift_Concurrency and friends) via
    // @rpath. On macOS those libraries live in the dyld shared cache under
    // /usr/lib/swift, which is not on the default search path for a plain
    // Rust binary — without this the app builds fine and then dies at launch
    // with "Library not loaded: @rpath/libswift_Concurrency.dylib".
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
        // Xcode's toolchain copy is the fallback for older systems that
        // predate the runtime being absorbed into the OS.
        println!(
            "cargo:rustc-link-arg=-Wl,-rpath,/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/macosx"
        );
    }

    tauri_build::build()
}
