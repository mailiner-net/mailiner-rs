use std::fs::{copy, File};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{exit, Command};
use std::{env, fs};
use which::which;

const OPENSSL_OPTIONS: [&str; 62] = [
    "-static",
    "no-aria",           // disable aria algorithm
    "no-asm",            // no ASM
    "no-async",          // no async operations (we don't use them)
    "no-autoalginit",    // don't auto-load all algos, leave it up to the app to load them
    "no-autoerrinit",    // don't auto-load error messages (should be implied by no-err)
    "no-bf",             // disable BF algorithm
    "no-blake2",         // disable blake2 algorithm
    "no-camellia",       // disable Camellia algorithm
    "no-cast",           // disable cast algorithm
    "no-comp",           // no compression support for TLS/SSL
    "no-cmp",            // no Certificate Management Protocol
    "no-cms",            // no Cryptograhpic Message Syntax
    "no-ct",             // don't build Certificate Transparency
    "no-des",            // disable DES algorithm
    "no-dsa",            // disable DSA algorithm
    "no-dgram",          // disable support for datagram-based BIO
    "no-dso",            // no dynamic loading of protocols/algos (requires no-engine)
    "no-dtls",           // we don't need DTLS
    "no-dtls1-method",   // don't compile DTLS at all
    "no-dtls1_2-method", // don't compile DTLS 1.2 at all
    "no-dynamic-engine", // disable dynamically loaded engines
    "no-egd",            // no entropy generation daemon
    "no-engine",         // no dynamic loading of protocols/algos (required by no-dso)
    "no-err",            // no error strings in the binary
    "no-filenames",      // don't compile in filename and line number information (e.g. for errors)
    "no-gost",           // disable GOST ciphersuites
    "no-idea",           // disable IDEA ciphersuites
    "no-legacy",         // disable legacy providers
    "no-module",         // don't build dynamically-loadable engines
    "no-md4",            // disable MD4 algorithm
    "no-ocb",            // disable OCB algorithm
    "no-ocsp",           // disable OCSP
    "no-padlockeng",     // don't build padlock engine
    "no-posix-io",       // duh, emscripten ain't posix
    "no-psk",            // no pre-shared key ciphersuites
    "no-rc2",            // disable RC2 algorithm
    "no-rc4",            // disable RC4 algorithm
    "no-rmd160",         // disable RMD160 algorithm
    "no-rdrand",         // no hardware RDRAND
    "no-sctp",           // don't build support for Stream Control Transmission Protocol (SCTP)
    "no-scrypt",         // disable SCRYPT algorithm
    "no-seed",           // disable Seed algorithm
    "no-shared",         // force static build
    "no-siphash",        // disable Siphash algorithm
    "no-siv",            // disable SIV algorithm
    "no-sm2",            // disable SM2 algorithms
    "no-sm3",            // disable SM3 algorithm
    "no-sm4",            // disable SM4 algorithm
    "no-sock",           // disable support for socket BIOs
    "no-srp",            // no Secure Remote Password (SRP) suport
    "no-srtp",           // no Secure Real-Time Transport protocol support
    "no-ssl",            // we don't want SSL
    "no-ssl3-method",    // do not build SSL method at all
    "no-ssl-trace",      // Disable SSL trace capatiblity (may reduce libssl size)
    "no-stdio",          // don't use anything from stdio.h
    "no-tests",          // no tests
    "no-threads",        // no thread support
    "no-ts",             // don't build Time Stamping Authority support
    "no-whirlpool",      // disable whirlpool algorithm
    "no-zlib",           // no zlib
    "no-uplink",         // Don't build support for UPLINK interface
];

fn fix_makefile(dir: &PathBuf) -> io::Result<()> {
    copy(dir.join("Makefile"), dir.join("Makefile.bkp"))?;

    let mut file = File::open(dir.join("Makefile"))?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;

    let new_contents = contents.replace("$(CROSS_COMPILE)", "");

    let mut file = File::create(dir.join("Makefile"))?;
    file.write_all(new_contents.as_bytes())?;

    Ok(())
}

fn find_executable(name: &str) -> io::Result<String> {
    match which(name) {
        Ok(path) => Ok(path.as_os_str().to_string_lossy().to_string()),
        Err(_) => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} not found", name),
        )),
    }
}


fn build_openssl() -> io::Result<PathBuf>
{
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set!"));
    let openssl_build_dir = out_dir.join("openssl-build");
    fs::create_dir_all(&openssl_build_dir).expect("Failed to create openssl-src dir");
    let src_dir = env::current_dir().expect("Failed to get CWD");
    let openssl_src_dir = src_dir.join("openssl-src");
    let openssl_install_dir = out_dir.join("openssl-install");
    fs::create_dir_all(&openssl_install_dir).expect("Failed to create openssl-install dir");

    let target_family =
        env::var("CARGO_CFG_TARGET_FAMILY").expect("CARGO_CFG_TARGET_FAMILY not set!");
    let mut cmd = if target_family == "wasm" {
        let emconfigure = find_executable("emconfigure").expect("emconfigure not found");
        println!("Found emconfigure: {}", emconfigure);

        let mut cmd = Command::new(emconfigure);
        cmd.env("CFLAGS", "-fPIC -flto=thin -Os")
            .env("LDFLAGS", "-flto=thin -sSIDE_MODULE=2 -Os")
            .arg(openssl_src_dir.join("Configure"))
                .arg("linux-x32"); // out target platform for WASM
        cmd
    } else {
        let mut cmd = Command::new(openssl_src_dir.join("Configure"));
        cmd.env("CFLAGS", "-fPIC");
        cmd
    };
    cmd.args(&OPENSSL_OPTIONS).current_dir(&openssl_build_dir);
    let output = cmd.output().expect("Failed to configure OpenSSL");
    if !output.status.success() {
        eprintln!("Failed to configure OpenSSL: {}", output.status);
        eprintln!("Build dir: {}", openssl_build_dir.display());
        eprintln!("Command: {:?}", cmd);
        eprintln!(
            "Command output: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        exit(1);
    }
    println!("Configured OpenSSL");
    println!("Directory: {}", openssl_build_dir.display());
    println!("Command output: {}", String::from_utf8_lossy(&output.stdout));

    if target_family == "wasm" {
        if let Err(err) = fix_makefile(&openssl_build_dir) {
            eprintln!("Failed to patch Makefile: {}", err);
            exit(1);
        }
    }

    let mut cmd = if target_family == "wasm" {
        let emmake = find_executable("emmake").expect("emmake not found");
        println!("Found emmake: {}", emmake);

        let mut cmd = Command::new(emmake);
        cmd.arg("make");
        cmd
    } else {
        Command::new("make")
    };
    cmd.arg("-j")
        .current_dir(&openssl_build_dir)
        .output()
        .expect("Failed to build OpenSSL");
    let output = cmd.output().expect("Failed to build OpenSSL");
    if !output.status.success() {
        eprintln!("Failed to build OpenSSL: {}", output.status);
        eprintln!("Build dir: {}", openssl_build_dir.display());
        eprintln!("Command: {:?}", cmd);
        eprintln!(
            "Command output: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        exit(1);
    }
    println!("Built OpenSSL");

    let mut cmd = Command::new("make");
    cmd.arg(format!("DESTDIR={}", openssl_install_dir.display()))
        .arg("install_sw")
        .current_dir(&openssl_build_dir);
    let output = cmd.output().expect("Failed to install OpenSSL");
    if !output.status.success() {
        eprintln!("Failed to install OpenSSL: {}", output.status);
        eprintln!("Build dir: {}", openssl_build_dir.display());
        eprintln!("Command: {:?}", cmd);
        eprintln!(
            "Command output: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        exit(1);
    }
    println!("Installed OpenSSL");

    if target_family == "wasm" {
        println!(
            "cargo:rustc-link-search=native={}/usr/local/libx32",
            openssl_install_dir.display()
        );
    } else {
        println!(
            "cargo:rustc-link-search=native={}/usr/local/lib64",
            openssl_install_dir.display()
        );
    }
    println!("cargo:rustc-link-lib=static=crypto");
    println!("cargo:rustc-link-lib=static=ssl");
    println!("cargo:rerun-if-changed=openssl-src");

    Ok(openssl_install_dir)
}

fn main() {
    let openssl_install_dir = build_openssl().expect("Failed to build OpenSSL");

    // The bindgen::Builder is the main entry point
    // to bindgen, and lets you build up options for
    // the resulting bindings.
    let bindings = bindgen::Builder::default()
        // The input header we would like to generate
        // bindings for.
        .header("src/openssl.h")
        .clang_arg(format!("-I{}/usr/local/include/", openssl_install_dir.display()))
        // Tell cargo to invalidate the built crate whenever any of the
        // included header files changed.
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        // Finish the builder and generate the bindings.
        .generate()
        // Unwrap the Result and panic on failure.
        .expect("Unable to generate bindings");

    // Write the bindings to the $OUT_DIR/bindings.rs file.
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");

    println!("cargo:rerun-if-changed=src/openssl.h");
}
