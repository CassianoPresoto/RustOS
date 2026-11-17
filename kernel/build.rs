//! Escolhe o linker script da ISA alvo. Fora dos alvos bare-metal não faz nada.

fn main() {
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    println!("cargo:rerun-if-changed=linker/{arch}.ld");
    if os == "none" {
        println!("cargo:rustc-link-arg-bins=-T{dir}/linker/{arch}.ld");
        println!("cargo:rustc-link-arg-bins=-zmax-page-size=0x1000");
        println!("cargo:rustc-link-arg-bins=-pie");
        if arch == "aarch64" {
            println!("cargo:rustc-link-arg-bins=-znotext");
        }
    }
}
