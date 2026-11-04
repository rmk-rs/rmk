fn main() {
    println!("cargo:rerun-if-changed=keyboard.toml");
    println!("cargo:rustc-link-arg-bins=-Tlinkall.x");
}
