fn main() {
    println!("cargo:rerun-if-changed=snapper.manifest");
    println!("cargo:rerun-if-changed=snapper.rc");
    // Per-monitor DPI v2 lives in snapper.manifest. The resource script
    // embeds that file as RT_MANIFEST. No-op when the target is not Windows.
    embed_resource::compile("snapper.rc", embed_resource::NONE);
}
