// Nothing to build.  `links` in the manifest needs a build script: it makes two versions of rt in
// one program a clear message from Cargo, not E0152 (two panic handlers) from the compiler.
fn main() {
    println!("cargo::rerun-if-changed=build.rs");
}
