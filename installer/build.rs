use flate2::Compression;
use flate2::write::GzEncoder;
use std::env;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter};
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rustc-check-cfg=cfg(has_payload)");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../app/target/release/workploy.exe");
    println!("cargo:rerun-if-changed=../app/assets/icon.ico");

    let icon_src = Path::new("../app/assets/icon.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") && icon_src.exists() {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../app/assets/icon.ico");
        res.set("ProductName", "Workploy");
        res.set("FileDescription", "Instalador do Workploy");
        res.set("CompanyName", "Workploy");
        if let Err(err) = res.compile() {
            println!("cargo:warning=Não foi possível embutir ../app/assets/icon.ico: {err}");
        }
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR não definido"));
    let out_gz = out_dir.join("workploy.exe.gz");
    let out_ico = out_dir.join("workploy.ico");

    if icon_src.exists() {
        fs::copy(icon_src, &out_ico).expect("Falha ao copiar workploy.ico para OUT_DIR");
    } else {
        fs::write(&out_ico, []).expect("Falha ao criar placeholder workploy.ico em OUT_DIR");
    }

    let payload_exe = Path::new("../app/target/release/workploy.exe");
    if payload_exe.exists() {
        compress_file(payload_exe, &out_gz)
            .expect("Falha ao comprimir ../app/target/release/workploy.exe");
        println!("cargo:rustc-cfg=has_payload");
    } else {
        // Placeholder vazio permite `cargo check` e `cargo test` antes do build release do app.
        fs::write(&out_gz, []).expect("Falha ao criar placeholder workploy.exe.gz em OUT_DIR");
    }
}

fn compress_file(src: &Path, dst: &Path) -> io::Result<()> {
    let input = File::open(src)?;
    let mut reader = BufReader::new(input);
    let output = File::create(dst)?;
    let writer = BufWriter::new(output);
    let mut encoder = GzEncoder::new(writer, Compression::best());
    io::copy(&mut reader, &mut encoder)?;
    encoder.finish()?;
    Ok(())
}
