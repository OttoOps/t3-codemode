use std::path::{Path, PathBuf};

fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let mut entries: Vec<_> =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("reading {}: {e}", dir.display())).flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name == "__pycache__" {
            continue;
        }
        if path.is_dir() {
            collect(root, &path, out);
        } else if path.is_file() {
            let rel = path.strip_prefix(root).expect("under the skills folder");
            let rel: Vec<_> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            out.push((rel.join("/"), path));
        }
    }
}

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets it")).join("skills");
    println!("cargo:rerun-if-changed=skills");
    let mut files = Vec::new();
    collect(&root, &root, &mut files);
    let mut code = String::from("pub static FILES: &[(&str, &[u8])] = &[\n");
    for (rel, path) in &files {
        code.push_str(&format!("    ({rel:?}, include_bytes!({:?})),\n", path.to_string_lossy()));
    }
    code.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets it")).join("skills.rs");
    std::fs::write(&out, code).unwrap_or_else(|e| panic!("writing {}: {e}", out.display()));
}
