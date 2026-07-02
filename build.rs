fn main() {
    // Cargo sets `TARGET` for build scripts (the triple the crate is being
    // compiled for), but not for the crate itself - re-expose it as an env
    // var the crate can read via `env!("TARGET")` at compile time, e.g. for
    // `--version` output.
    println!("cargo:rustc-env=TARGET={}", std::env::var("TARGET").unwrap());
}
