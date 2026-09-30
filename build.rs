fn main() {
    // The exe's icon, which the tray also loads (src/tray.rs). Regenerate it with
    // scripts/make-icon after editing assets/icon.svg.
    println!("cargo:rerun-if-changed=assets/proj.rc");
    println!("cargo:rerun-if-changed=assets/proj.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/proj.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}
