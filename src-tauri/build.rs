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

    #[cfg(target_os = "windows")]
    {
        // Keep Tauri's icon and version resources, but compile the manifest as
        // a separate resource for every final artifact. The lib unit-test
        // harness imports TaskDialogIndirect too, so it must activate Common
        // Controls v6 just like the application executable does.
        let attributes = tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        tauri_build::try_build(attributes).expect("failed to run tauri-build");

        embed_resource::compile_for_everything("windows-app-manifest.rc", embed_resource::NONE)
            .manifest_required()
            .expect("failed to compile the Windows application manifest");
        return;
    }

    #[cfg(not(target_os = "windows"))]
    tauri_build::build()
}
