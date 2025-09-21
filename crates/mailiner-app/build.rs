fn main() {
    println!("cargo:rustc-env=IMAP_PASSWORD={}", std::env::var("IMAP_PASSWORD").unwrap());
}