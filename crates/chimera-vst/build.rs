use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for path in paths {
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name == ".DS_Store" || name.ends_with(".map") {
            continue;
        }
        if path.is_dir() {
            collect_files(root, &path, out);
        } else if path.is_file() {
            if let Ok(rel) = path.strip_prefix(root) {
                let web_path = format!("/{}", rel.to_string_lossy().replace('\\', "/"));
                out.push((web_path, path));
            }
        }
    }
}

fn mime_for(path: &str) -> &'static str {
    if path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".js") || path.ends_with(".mjs") {
        "text/javascript; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".wasm") {
        "application/wasm"
    } else if path.ends_with(".woff2") {
        "font/woff2"
    } else if path.ends_with(".otf") {
        "font/otf"
    } else if path.ends_with(".json") {
        "application/json; charset=utf-8"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".txt") {
        "text/plain; charset=utf-8"
    } else {
        "application/octet-stream"
    }
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let dist_dir = manifest_dir.join("../../dist");
    println!("cargo:rerun-if-changed={}", dist_dir.join("index.html").display());
    println!("cargo:rerun-if-changed={}", dist_dir.join("assets").display());

    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=WebKit");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let dest = out_dir.join("web_assets.rs");

    let mut files = Vec::new();
    if dist_dir.exists() {
        if let Ok(canon) = dist_dir.canonicalize() {
            collect_files(&canon, &canon, &mut files);
        } else {
            collect_files(&dist_dir, &dist_dir, &mut files);
        }
    }

    let mut code = String::new();
    code.push_str("pub fn lookup_web_asset(path: &str) -> Option<(&'static str, &'static [u8])> {\n");
    code.push_str("    let p = if path.is_empty() || path == \"/\" { \"/index.html\" } else { path };\n");
    code.push_str("    match p {\n");

    if files.is_empty() {
        code.push_str(
            "        \"/index.html\" => Some((\"text/html; charset=utf-8\", b\"<!doctype html><html><body style='background:#000;color:#4fb6f2;font-family:monospace;padding:24px'>SIREN UI bundle missing. Run ./tools/build-vst.sh to build dist/ before bundling.</body></html>\")),\n",
        );
    } else {
        for (web_path, disk_path) in &files {
            let mime = mime_for(web_path);
            let escaped_disk = disk_path.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
            code.push_str(&format!(
                "        {web_path:?} => Some(({mime:?}, include_bytes!(\"{escaped_disk}\"))),\n"
            ));
        }
    }

    code.push_str("        _ => None,\n");
    code.push_str("    }\n");
    code.push_str("}\n");

    fs::write(dest, code).unwrap();
}
