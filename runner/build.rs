use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const BOCFEL_VERSION: &str = "2.5";
const BOCFEL_URL: &str = "https://cspiegel.github.io/bocfel/downloads/bocfel-2.5.tar.gz";
const BOCFEL_SOURCES: &[&str] = &[
    "blorb.cpp",
    "branch.cpp",
    "dict.cpp",
    "iff.cpp",
    "io.cpp",
    "mathop.cpp",
    "meta.cpp",
    "memory.cpp",
    "objects.cpp",
    "options.cpp",
    "osdep.cpp",
    "patches.cpp",
    "process.cpp",
    "random.cpp",
    "screen.cpp",
    "sound.cpp",
    "stack.cpp",
    "stash.cpp",
    "unicode.cpp",
    "util.cpp",
    "zterp.cpp",
];

fn main() {
    let bocfel_dir = bocfel_source_dir();

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .include("bocfel/include")
        .include(&bocfel_dir)
        .file("bocfel/src/bocfel_embed.cpp")
        .define(platform_macro(), None)
        .define("ZTERP_NO_CURSES", None)
        .define("ZTERP_NO_V6", None)
        .define("ZTERP_NO_CHEAT", None)
        .define("ZTERP_NO_WATCHPOINTS", None)
        .define("ZTERP_NO_OPTIONS", None)
        .define("main", "bocfel_cli_main")
        .flag_if_supported("-std=c++17")
        .flag_if_supported("/std:c++17");

    for source in BOCFEL_SOURCES {
        build.file(bocfel_dir.join(source));
    }

    build.compile("bocfel_embedded");

    println!("cargo:rerun-if-changed=bocfel/src/bocfel_embed.cpp");
    println!("cargo:rerun-if-changed=bocfel/include/bocfel_embed.h");
    println!("cargo:rerun-if-env-changed=RUNNER_BOCFEL_SOURCE_DIR");
    println!("cargo:rerun-if-env-changed=RUNNER_BOCFEL_TARBALL");
}

fn bocfel_source_dir() -> PathBuf {
    if let Ok(source_dir) = env::var("RUNNER_BOCFEL_SOURCE_DIR") {
        return PathBuf::from(source_dir);
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is not set"));
    let source_dir = out_dir.join(format!("bocfel-{BOCFEL_VERSION}"));

    if source_dir.join("zterp.cpp").exists() {
        return source_dir;
    }

    let tarball = match env::var("RUNNER_BOCFEL_TARBALL") {
        Ok(path) => PathBuf::from(path),
        Err(_) => {
            let path = out_dir.join(format!("bocfel-{BOCFEL_VERSION}.tar.gz"));
            download_bocfel(&path);
            path
        }
    };

    extract_bocfel(&tarball, &out_dir);

    if !source_dir.join("zterp.cpp").exists() {
        panic!(
            "Bocfel extraction did not produce expected source directory: {}",
            source_dir.display()
        );
    }

    source_dir
}

fn download_bocfel(tarball: &Path) {
    let status = if cfg!(windows) {
        Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Invoke-WebRequest -Uri $args[0] -OutFile $args[1]",
            ])
            .arg(BOCFEL_URL)
            .arg(tarball)
            .status()
    } else {
        Command::new("curl")
            .args(["--fail", "--location", "--output"])
            .arg(tarball)
            .arg(BOCFEL_URL)
            .status()
    }
    .expect("failed to start Bocfel download command");

    if !status.success() {
        panic!(
            "failed to download Bocfel {BOCFEL_VERSION} from {BOCFEL_URL}; \
             set RUNNER_BOCFEL_TARBALL or RUNNER_BOCFEL_SOURCE_DIR for offline builds"
        );
    }
}

fn extract_bocfel(tarball: &Path, out_dir: &Path) {
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(tarball)
        .arg("-C")
        .arg(out_dir)
        .status()
        .expect("failed to start tar to extract Bocfel sources");

    if !status.success() {
        panic!(
            "failed to extract Bocfel source tarball {}; set RUNNER_BOCFEL_SOURCE_DIR instead",
            tarball.display()
        );
    }
}

fn platform_macro() -> &'static str {
    if cfg!(windows) {
        "ZTERP_OS_WIN32"
    } else if cfg!(target_os = "macos")
        || cfg!(target_os = "linux")
        || cfg!(target_os = "freebsd")
        || cfg!(target_os = "openbsd")
        || cfg!(target_os = "netbsd")
    {
        "ZTERP_OS_UNIX"
    } else {
        "ZTERP_NO_CURSES"
    }
}
