use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use regex::Regex;
use walkdir::WalkDir;

/// Fake API: возвращает детерминированный ID на основе хеша префикса.
/// В будущем заменяется на реальный HTTP-запрос через reqwest.
async fn fetch_id(prefix: &str) -> Result<u64> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    prefix.hash(&mut hasher);
    let id = hasher.finish() % 10_000_000;
    Ok(id)
}

fn find_matching_files(root: &Path) -> Result<Vec<PathBuf>> {
    let re = Regex::new(r"^[A-Za-z0-9]+_\d{1,2}\.(jpg|png)$")
        .context("Failed to compile regex")?;

    let mut found = Vec::new();

    for entry in WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if re.is_match(name) {
                found.push(path.to_path_buf());
            }
        }
    }

    Ok(found)
}

fn group_files(files: Vec<PathBuf>) -> HashMap<String, Vec<PathBuf>> {
    let sep_re = Regex::new(r"^([A-Za-z0-9]+)_\d{1,2}\.\w+$").unwrap();

    let mut groups: HashMap<String, Vec<PathBuf>> = HashMap::new();

    for path in files {
        let name = path.file_name().unwrap().to_str().unwrap();
        if let Some(caps) = sep_re.captures(name) {
            let prefix = caps[1].to_string();
            groups.entry(prefix).or_default().push(path);
        }
    }

    for paths in groups.values_mut() {
        paths.sort_by(|a, b| {
            let name_a = a.file_stem().unwrap().to_str().unwrap();
            let name_b = b.file_stem().unwrap().to_str().unwrap();
            name_a.cmp(name_b)
        });
    }

    groups
}

fn rename_group(prefix: &str, api_id: u64, files: &[PathBuf]) -> Result<()> {
    for (i, path) in files.iter().enumerate() {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let new_name = format!("{}_{}_{}.{}", prefix, api_id, i + 1, ext);
        let new_path = path.parent().unwrap().join(&new_name);

        if new_path.exists() {
            bail!(
                "Conflict: '{}' already exists — skipping rename of '{}'",
                new_path.display(),
                path.display()
            );
        }

        fs::rename(path, &new_path).with_context(|| {
            format!(
                "Failed to rename '{}' → '{}'",
                path.display(),
                new_path.display()
            )
        })?;

        println!("  {} → {}", path.display(), new_path.display());
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    if !root.is_dir() {
        bail!("'{}' is not a directory", root.display());
    }

    println!("Scanning: {}", root.display());

    let files = find_matching_files(&root)?;

    if files.is_empty() {
        println!("No matching files found.");
        return Ok(());
    }

    println!("Found {} matching file(s):", files.len());
    for f in &files {
        println!("  {}", f.display());
    }

    let groups = group_files(files);

    println!("\nGroups:");
    for (prefix, paths) in &groups {
        println!(
            "  [{}] → {} file(s)",
            prefix,
            paths.len()
        );
        for p in paths {
            println!("    {}", p.display());
        }
    }

    println!("\nFetching IDs from API...");
    let mut id_map: HashMap<String, u64> = HashMap::new();
    for prefix in groups.keys() {
        let id = fetch_id(prefix).await?;
        println!("  {} → {}", prefix, id);
        id_map.insert(prefix.clone(), id);
    }

    println!("\nRenaming:");
    for (prefix, paths) in &groups {
        let id = *id_map.get(prefix.as_str()).unwrap();
        rename_group(prefix, id, paths)?;
    }

    println!("\nDone.");
    Ok(())
}
