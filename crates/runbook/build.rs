use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

fn main() {
    if let Err(e) = generate() {
        eprintln!("build script failed: {}", e);
        std::process::exit(1);
    }
}

fn generate() -> io::Result<()> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").map_err(io::Error::other)?;
    let library_dir = Path::new(&manifest_dir).join("../../library");
    let library_dir = library_dir.canonicalize()?;

    let out_dir = env::var("OUT_DIR").map_err(io::Error::other)?;
    let dest_path = Path::new(&out_dir).join("builtin_libraries.rs");
    let mut f = fs::File::create(&dest_path)?;

    // Collect library directories (sorted, skip dotfiles like .drafts)
    let mut entries: Vec<_> = fs::read_dir(&library_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                && !e.file_name().to_string_lossy().starts_with('.')
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    writeln!(f, "static BUILTIN_LIBRARIES: &[BuiltinLibrary] = &[")?;

    for entry in &entries {
        let dir_name = entry.file_name().to_string_lossy().to_string();
        let source = format!("oj/{}", dir_name);

        // Collect .hcl files in this directory (sorted)
        let mut hcl_entries: Vec<_> = fs::read_dir(entry.path())?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|ext| ext == "hcl").unwrap_or(false))
            .collect();
        hcl_entries.sort_by_key(|e| e.file_name());

        if hcl_entries.is_empty() {
            continue;
        }

        writeln!(f, "    BuiltinLibrary {{")?;
        writeln!(f, "        source: \"{}\",", source)?;
        writeln!(f, "        files: &[")?;
        for hcl_entry in &hcl_entries {
            let filename = hcl_entry.file_name().to_string_lossy().to_string();
            let abs_path = hcl_entry.path().canonicalize()?;
            writeln!(
                f,
                "            (\"{}\", include_str!(\"{}\")),",
                filename,
                abs_path.display()
            )?;
        }
        writeln!(f, "        ],")?;
        writeln!(f, "    }},")?;
    }

    writeln!(f, "];")?;

    // Rerun on any change to the library directory
    println!("cargo:rerun-if-changed={}", library_dir.display());
    for entry in &entries {
        println!("cargo:rerun-if-changed={}", entry.path().display());
    }

    Ok(())
}
