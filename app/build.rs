fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::path::Path::new("assets/icon.ico").exists()
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(err) = res.compile() {
            println!("cargo:warning=Não foi possível embutir assets/icon.ico: {err}");
        }
    }
}
