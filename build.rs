fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/paperview.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let Ok(home) = std::env::var("USERPROFILE") else {
        return;
    };
    let windres = format!("{home}\\.cargo\\gnu-binutils\\windres.exe");
    if !std::path::Path::new(&windres).exists() {
        println!("cargo:warning=windres not found; exe icon not embedded");
        return;
    }
    let out = std::env::var("OUT_DIR").unwrap();
    let res = format!("{out}\\app_res.o");
    let status = std::process::Command::new(&windres)
        .current_dir("assets")
        .args([
            "--preprocessor=C:/Program Files/Git/usr/bin/cat.exe",
            "app.rc",
            "-O",
            "coff",
            "-o",
            &res,
        ])
        .status();
    match status {
        Ok(s) if s.success() => println!("cargo:rustc-link-arg-bins={res}"),
        _ => println!("cargo:warning=windres failed; exe icon not embedded"),
    }
}
