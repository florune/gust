fn main() {
    println!("cargo:rerun-if-changed=assets/gust.rc");
    println!("cargo:rerun-if-changed=assets/gust.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/gust.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("the icon resource compiles");
    }
}
