/// The app manifest conduit ships on Windows.
///
/// Two things here are load-bearing:
///
///   - **`dpiAwareness`.** tao calls `SetProcessDpiAwarenessContext` at
///     `EventLoop::new`, but everything before that runs DPI-unaware, and every
///     coordinate conduit reports on Windows is a physical pixel — which is
///     only coherent under per-monitor-v2. Declaring it here makes it true from
///     the first instruction and removes the ordering dependency.
///   - **`asInvoker`.** Never `requireAdministrator`. Elevation is a per-launch
///     choice the user makes from the readiness card when they actually need to
///     drive an elevated window; forcing a UAC prompt on every start to make an
///     uncommon case work would be backwards.
///
/// The Common-Controls v6 dependency is carried over from tauri-build's default
/// manifest — comctl32 needs it, and supplying our own manifest replaces
/// theirs wholesale rather than merging.
#[cfg(target_os = "windows")]
const MANIFEST: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0" xmlns:asmv3="urn:schemas-microsoft-com:asm.v3">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*" />
    </dependentAssembly>
  </dependency>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v2">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
  <asmv3:application>
    <asmv3:windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/PM</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2, PerMonitor</dpiAwareness>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
    </asmv3:windowsSettings>
  </asmv3:application>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <!-- Windows 10 and 11 -->
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}" />
    </application>
  </compatibility>
</assembly>
"#;

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
        let attributes = tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new().app_manifest(MANIFEST));
        tauri_build::try_build(attributes).expect("failed to run tauri-build");
        return;
    }

    #[cfg(not(target_os = "windows"))]
    tauri_build::build()
}
