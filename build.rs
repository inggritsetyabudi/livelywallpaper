//! Builds both native halves of the app:
//!
//!   * `src/native/lively_native.m` — the wallpaper engines (ObjC)
//!   * `src/ui/*.swift`              — the SwiftUI settings app
//!
//! Both are compiled to static libraries and linked into one binary, so the
//! app and the daemon are the same executable and cannot drift apart.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Minimum macOS version. All three halves of the build must agree on this,
/// or the linker warns that objects were built for a newer OS than the one
/// being linked and runtime behaviour becomes undefined.
const DEPLOYMENT_TARGET: &str = "14.0";

fn main() {
    emit_rerun_directives();
    embed_builtin_wallpapers();

    // CARGO_CFG_TARGET_OS, not cfg!(target_os): a build script is compiled for
    // the *host*, so cfg! would answer the wrong question when cross-compiling.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    if target_os == "macos" {
        build_macos();
    } else if target_os == "linux" {
        build_linux();
    } else {
        println!(
            "cargo:warning=unsupported target OS; only macOS and Linux are supported. \
             See src/platform/mod.rs."
        );
    }
}

/// macOS: Objective-C engines plus the SwiftUI settings app, linked into one
/// binary alongside the Rust control plane.
fn build_macos() {
    // A Rust build otherwise defaults to an older floor than the native code.
    println!("cargo:rustc-link-arg=-mmacosx-version-min={DEPLOYMENT_TARGET}");

    build_objc();
    let swift_lib = build_swift();

    println!("cargo:rustc-link-search=native={}", swift_lib.display());
    println!("cargo:rustc-link-lib=static=LivelyUI");
    link_swift_runtime();
}

/// Linux: a C shim over X11 / Wayland layer-shell / Cairo.
///
/// Deliberately small and in C rather than pulling in a windowing toolkit:
/// the only surface needed is one desktop-level window per screen. Anything
/// heavier would be a large dependency for a single use.
fn build_linux() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));

    let mut build = cc::Build::new();
    build
        .file("src/platform/native/lively_linux.c")
        .flag("-std=c11")
        // Built-ins are not defined in the shim; the common headers expect them.
        .define("_DEFAULT_SOURCE", None)
        .warnings(true);

    let has = |name: &str| std::env::var(format!("CARGO_FEATURE_{name}")).is_ok();

    if has("LINUX_VIDEO") {
        build.define("LIVELY_HAVE_VIDEO", "1");
    }
    if has("LINUX_WEB") {
        build.define("LIVELY_HAVE_WEB", "1");
    }

    build.compile("lively_linux");
    println!("cargo:rustc-link-search=native={}", out_dir.display());

    // Core graphics surface + input.
    //
    // xcb-randr is not optional. The backend enumerates monitors through
    // RANDR because X11 screens are not monitors: a dual-display machine is one
    // screen with two outputs. Without this the wallpaper window would be sized
    // for the whole virtual desktop instead of a single panel, and
    // per-monitor wallpapers would be impossible.
    for lib in ["xcb", "xcb-randr", "cairo", "EGL", "gl", "m", "pthread"] {
        println!("cargo:rustc-link-lib={lib}");
    }

    if has("LINUX_VIDEO") {
        for lib in ["gstreamer-1.0", "gstreamer-app-1.0", "gstreamer-video-1.0"] {
            println!("cargo:rustc-link-lib={lib}");
        }
    }

    if has("LINUX_WEB") {
        println!("cargo:rustc-link-lib=webkit2gtk-4.1");
        println!("cargo:rustc-link-lib=javascriptcoregtk-4.1");
    }

    // Wayland layer-shell, only meaningful on a Wayland session.
    if has("LINUX_WAYLAND") {
        println!("cargo:rustc-link-lib=wayland-client");
    }
}

fn emit_rerun_directives() {
    for file in ["src/native/lively_native.h", "src/native/lively_native.m"] {
        println!("cargo:rerun-if-changed={file}");
    }
    println!("cargo:rerun-if-changed=src/ui/");
    println!("cargo:rerun-if-changed=src/ui/bridge.h");
    println!("cargo:rerun-if-changed=resources/wallpapers/");
    println!("cargo:rerun-if-changed=build.rs");
}

/// Compile the bundled wallpapers into the binary.
///
/// They are embedded rather than shipped as loose files so the app has
/// something to show on first launch, before the user has imported anything,
/// and so a single executable stays self-contained. The generated module is
/// an `include_bytes!` table keyed by wallpaper name and file name — cheap to
/// compile and impossible to get out of sync with the source directory.
fn embed_builtin_wallpapers() {
    let root = PathBuf::from("resources/wallpapers");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));

    let mut wallpapers: Vec<(String, Vec<(String, PathBuf)>)> = Vec::new();

    if root.is_dir() {
        for entry in std::fs::read_dir(&root)
            .expect("read resources/wallpapers")
            .flatten()
        {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
            else {
                continue;
            };

            // Only ship files a wallpaper could legitimately need. Anything
            // else is a build artefact or an editor leftover.
            let mut files: Vec<(String, PathBuf)> = Vec::new();
            collect_wallpaper_files(&path, &mut files);

            if files.is_empty() {
                continue;
            }
            files.sort();
            wallpapers.push((name, files));
        }
    }

    wallpapers.sort_by(|a, b| a.0.cmp(&b.0));

    let mut source = String::new();
    source.push_str("// @generated by build.rs — do not edit.\n");
    source.push_str("//\n");
    source.push_str("// The bundled wallpaper collection, embedded so the app has content to\n");
    source.push_str("// show before the user imports anything.\n\n");
    // A named alias keeps the tuple nesting readable and keeps clippy from
    // flagging the generated signature as too complex.
    source.push_str("/// One embedded wallpaper: `(file name, contents)`.\n");
    source.push_str("pub type BuiltinFile<'a> = (&'a str, &'a [u8]);\n\n");
    source.push_str("/// `(wallpaper name, files)`\n");
    source.push_str("pub static BUILTIN_WALLPAPERS: &[(&str, &[BuiltinFile])] = &[\n");

    let mut total_bytes = 0usize;
    for (name, files) in &wallpapers {
        source.push_str("    (\n");
        source.push_str(&format!("        {name:?},\n"));
        source.push_str("        &[\n");
        for (file, path) in files {
            total_bytes += std::fs::metadata(path)
                .map(|m| m.len() as usize)
                .unwrap_or(0);
            // `include_bytes!` takes a literal and resolves relative paths against the
            // *generated file's* directory (OUT_DIR), not the crate root, so
            // an absolute path is the only thing that works here.
            let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
            source.push_str(&format!(
                "            ({file:?}, include_bytes!({:?})),\n",
                absolute.to_string_lossy()
            ));
        }
        source.push_str("        ],\n");
        source.push_str("    ),\n");
    }
    source.push_str("];\n");

    let generated = out_dir.join("builtin_wallpapers.rs");
    std::fs::write(&generated, source).expect("write generated wallpaper module");

    if wallpapers.is_empty() {
        println!("cargo:warning=no built-in wallpapers found in resources/wallpapers");
    } else {
        println!(
            "cargo:warning=embedded {} built-in wallpaper(s), {:.0} KiB",
            wallpapers.len(),
            total_bytes as f64 / 1024.0
        );
    }
}

/// Collect every file in a wallpaper folder, relative to it.
fn collect_wallpaper_files(dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_wallpaper_files(&path, out);
            continue;
        }

        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };

        // Skip dotfiles and editor droppings.
        if name.starts_with('.') {
            continue;
        }
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        const ALLOWED: &[&str] = &[
            "json", "png", "jpg", "jpeg", "webp", "heic", "gif", "mp4", "mov", "webm", "mkv",
            "avi", "m4v", "html", "htm", "css", "js", "metal", "fragment", "shader", "txt",
        ];

        if ALLOWED.contains(&extension.as_str()) {
            out.push((name, path));
        }
    }
}

fn build_objc() {
    cc::Build::new()
        .file("src/native/lively_native.m")
        .flag("-ObjC")
        .flag("-fobjc-arc")
        .flag(format!("-mmacosx-version-min={DEPLOYMENT_TARGET}"))
        .warnings(true)
        .compile("lively_native");

    for framework in [
        "Cocoa",
        "AVFoundation",
        "WebKit",
        "Metal",
        "MetalKit",
        "IOKit",
        "CoreGraphics",
        "QuartzCore",
        "CoreMedia",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}

/// Compile the SwiftUI sources into `OUT_DIR/libLivelyUI.a`.
///
/// Returns the directory the archive landed in so the caller can add a link
/// search path.
fn build_swift() -> PathBuf {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let archive = out_dir.join("libLivelyUI.a");

    // A stale archive from a previous run would silently win over fresh
    // sources, so always start clean.
    let _ = std::fs::remove_file(&archive);

    let sdk = sdk_path();
    let mut sources: Vec<PathBuf> = Vec::new();
    collect_swift_files(Path::new("src/ui"), &mut sources);
    sources.sort();

    if sources.is_empty() {
        // Keep the link line valid if the UI sources are ever removed; the Rust
        // side guards its entry points so a missing UI is a clear runtime
        // message rather than a link error.
        println!("cargo:warning=no Swift sources found in src/ui; building without UI");
        std::fs::write(&archive, b"").ok();
        return out_dir;
    }

    println!(
        "cargo:warning=compiling {} Swift source(s) for the settings app",
        sources.len()
    );

    let mut cmd = Command::new("swiftc");
    cmd.arg("-emit-library")
        .arg("-static")
        .arg("-O")
        .arg("-whole-module-optimization")
        .arg("-parse-as-library")
        .arg("-swift-version")
        .arg("5")
        .arg("-sdk")
        .arg(&sdk)
        .arg("-target")
        .arg(swift_target())
        .arg("-module-name")
        .arg("LivelyUI")
        // SwiftUI/AppKit are linked by the Swift driver; Rust needs them too.
        .arg("-framework")
        .arg("SwiftUI")
        .arg("-framework")
        .arg("AppKit")
        .arg("-o")
        .arg(&archive);
    cmd.args(&sources);

    let status = cmd.status().unwrap_or_else(|e| {
        panic!(
            "failed to run swiftc ({e}). Building the UI requires the Xcode \
             command line tools; install with `xcode-select --install`."
        )
    });

    assert!(
        status.success(),
        "swiftc failed with {status}. Fix the Swift sources under src/ui/ or run \
         `xcode-select --install` to install the toolchain."
    );

    out_dir
}

fn collect_swift_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_swift_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "swift") {
            out.push(path);
        }
    }
}

fn sdk_path() -> String {
    let output = Command::new("xcrun").arg("--show-sdk-path").output();
    match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        _ => panic!(
            "cannot locate the macOS SDK. Install the Xcode command line tools \
             with `xcode-select --install`."
        ),
    }
}

/// Ask `swiftc` which target triple to use, so the Swift half and the Rust half
/// are guaranteed to agree on architecture.
fn swift_target() -> String {
    // The arch follows the host and the deployment target is fixed above, so
    // there is nothing to parse out of `swift -print-target-info`.
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86_64"
    };
    format!("{arch}-apple-macosx{DEPLOYMENT_TARGET}")
}

/// Link the Swift runtime and its dependencies into the final binary.
///
/// A Rust build normally does not pull these in; without them the executable
/// fails to launch with an undefined-symbol error on the first Swift call.
///
/// The compatibility shims are resolved against whichever SDK is actually
/// installed and skipped when absent — their names and availability have
/// changed across Swift releases, and pinning one breaks the build on other
/// toolchains.
fn link_swift_runtime() {
    let sdk = sdk_path();

    for lib in [
        "swiftCore",
        "swiftFoundation",
        "swiftDispatch",
        "swiftDarwin",
        "swiftCompatibilitySpan",
        "swiftCompatibility51",
    ] {
        // Missing shims are normal: their names and availability change between
        // Swift releases, and an SDK that omits one does not need it.
        if swift_library_exists(&sdk, lib) {
            println!("cargo:rustc-link-lib=dylib={lib}");
        }
    }

    // Foundation, AppKit and SwiftUI ship as frameworks on macOS, so they are
    // linked as frameworks rather than plain dylibs.
    for framework in ["Foundation", "AppKit", "SwiftUI"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }

    link_swift_back_deployment();
}

/// Link the compiler-rt OS X library, which supplies the Swift back-deployment
/// runtime hooks (`___isPlatformVersionAtLeast` and friends).
///
/// Swift emits calls to these when the deployment target is older than the SDK
/// it is compiled against. They live in the compiler's runtime archive rather
/// than any Swift library, and Rust builds pass `-nodefaultlibs`, so it has to
/// be requested explicitly or the final link fails on those symbols.
fn link_swift_back_deployment() {
    let Some(dir) = clang_runtime_dir() else {
        println!(
            "cargo:warning=could not locate the compiler-rt runtime; the final \
             link may fail on Swift back-deployment symbols"
        );
        return;
    };

    println!("cargo:rustc-link-search=native={}", dir.display());

    let osx = dir.join("libclang_rt.osx.a");
    if osx.exists() {
        println!("cargo:rustc-link-lib=static=clang_rt.osx");
    } else {
        println!(
            "cargo:warning=libclang_rt.osx.a not found in {}; the final link may \
             fail on Swift back-deployment symbols",
            dir.display()
        );
    }
}

/// Directory holding `libclang_rt.*.a`, derived from the active clang.
fn clang_runtime_dir() -> Option<PathBuf> {
    let output = Command::new("clang")
        .arg("--print-runtime-dir")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        return None;
    }

    // `--print-runtime-dir` reports e.g. .../lib/clang/21/lib/darwin, but the
    // archive for macOS lives one level up.
    let darwin = PathBuf::from(path);
    let up = darwin.parent().map(|p| p.join("darwin"));
    match up {
        Some(candidate) if candidate.join("libclang_rt.osx.a").exists() => Some(candidate),
        _ if darwin.join("libclang_rt.osx.a").exists() => Some(darwin),
        _ => Some(darwin),
    }
}

/// Whether a Swift runtime dylib exists in the SDK.
fn swift_library_exists(sdk: &str, lib: &str) -> bool {
    let name = format!("lib{lib}.tbd");
    let candidates = [
        Path::new(sdk).join("usr/lib/swift").join(&name),
        Path::new(sdk).join("usr/lib/swift/macosx").join(&name),
        Path::new(sdk).join("usr/lib").join(&name),
    ];
    candidates.iter().any(|p| p.exists())
}
