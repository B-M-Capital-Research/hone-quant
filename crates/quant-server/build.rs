// Exposes the release revision (HONE_QUANT_REVISION, set by scripts/build-release.sh) as
// HONE_QUANT_REVISION_OR_DEV so `hone-quant --version` can print it.
fn main() {
    println!("cargo:rerun-if-env-changed=HONE_QUANT_REVISION");
    let revision = std::env::var("HONE_QUANT_REVISION").unwrap_or_else(|_| "dev".into());
    println!("cargo:rustc-env=HONE_QUANT_REVISION_OR_DEV={revision}");
}
