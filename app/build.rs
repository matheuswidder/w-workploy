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

    emit_mobile_mock_assets();
}

/// Payload sintético hostil da página mobile, consumido só pelo teste
/// `runner_script_stays_parseable_with_hostile_mobile_payload`.
///
/// `vps.rs` inclui `OUT_DIR/mobile_mock_assets.rs` como expressão. Com
/// `WORKPLOY_TEST_BUILD=1` (o `[env]` de `app/.cargo/config.toml`) a lista
/// traz `"""`, crase, `${`, `\` e `\r`; sem a variável é `&[]`, e o binário
/// embute só os arquivos reais. O `install_runner` recusa publicar um runner
/// gerado com mock de qualquer forma.
fn emit_mobile_mock_assets() {
    println!("cargo:rerun-if-changed=assets/mobile/index.html");
    println!("cargo:rerun-if-changed=assets/mobile/app.css");
    println!("cargo:rerun-if-changed=assets/mobile/app.js");
    println!("cargo:rerun-if-env-changed=WORKPLOY_TEST_BUILD");

    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    let path = std::path::Path::new(&out).join("mobile_mock_assets.rs");
    let test_build = std::env::var("WORKPLOY_TEST_BUILD").is_ok_and(|v| v == "1");

    let body = if test_build {
        let hostile = [
            "const tpl = `a ${b} c`;",
            "const r = /\\/+$/;",
            "const d = \"\"\"triple\"\"\" and '''also''';",
            "// ${x} `y` \\n literal \\\\\\\\ backslashes",
            "$(\"#a\").val(1);",
            "const tpl2 = `line\nbreak ${x} ''`;",
            "re.sub(r\"[^a-z]+\", \"-\", t)  # } % ( )",
            "carriage\rreturn solo",
            "$'dq' and $\"dq\" and `'",
        ]
        .join("\n");
        let js = format!("{hostile}\n{hostile}");
        format!(
            "    (\"index\", \"{}\"),\n    (\"css\", \"{}\"),\n    (\"js\", \"{}\"),\n",
            escape(&hostile),
            escape("body{}"),
            escape(&js)
        )
    } else {
        String::new()
    };

    let source = if body.is_empty() {
        "// Gerado por build.rs. Não edite.\n&[]\n".to_string()
    } else {
        format!("// Gerado por build.rs. Não edite (payload hostil de teste).\n&[\n{body}]\n")
    };
    std::fs::write(&path, source).expect("escrever mobile_mock_assets.rs");
}

/// Escapa o payload para caber num literal Rust normal (`"..."`): só o que o
/// lexer do Rust exigiria (barra invertida, aspas, quebra de linha, controle).
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out
}