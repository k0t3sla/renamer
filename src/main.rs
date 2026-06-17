use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::Deserialize;
use walkdir::WalkDir;

#[derive(Debug, Deserialize)]
struct BarcodeEntry {
    art: String,
    barcode: Option<String>,
}

async fn fetch_barcodes() -> Result<Vec<BarcodeEntry>> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://shop.citilux.ru/api/sale/getBarcodes/")
        .send()
        .await
        .context("Failed to connect to API")?
        .json::<Vec<BarcodeEntry>>()
        .await
        .context("Failed to parse API response")?;
    Ok(resp)
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

fn rename_group(prefix: &str, barcode: &str, files: &[PathBuf]) -> Result<()> {
    for (i, path) in files.iter().enumerate() {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let new_name = format!("{}_{}_{}.{}", prefix, barcode, i + 1, ext);
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

fn print_errors(errors: &[String]) {
    if errors.is_empty() {
        return;
    }
    println!("\n========== ERRORS ==========");
    for err in errors {
        println!("{}", err);
    }
    println!("============================");
    println!("\nClosing in 10 seconds...");
    thread::sleep(Duration::from_secs(10));
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

    println!("\nFetching barcodes from API...");
    let barcodes = match fetch_barcodes().await {
        Ok(b) => b,
        Err(e) => {
            bail!("API request failed: {}", e);
        }
    };

    let barcode_map: HashMap<String, String> = barcodes
        .into_iter()
        .map(|e| (e.art, e.barcode.unwrap_or_default()))
        .collect();

    let mut errors: Vec<String> = Vec::new();
    let mut rename_map: HashMap<String, String> = HashMap::new();

    for prefix in groups.keys() {
        match barcode_map.get(prefix.as_str()) {
            Some(barcode) if !barcode.is_empty() => {
                rename_map.insert(prefix.clone(), barcode.clone());
                println!("  {} → {}", prefix, barcode);
            }
            Some(_barcode) => {
                let msg = format!(
                    "SKIP: '{}' — barcode is empty in API response",
                    prefix
                );
                println!("  {}", msg);
                errors.push(msg);
            }
            None => {
                let msg = format!(
                    "SKIP: '{}' — article not found in API response",
                    prefix
                );
                println!("  {}", msg);
                errors.push(msg);
            }
        }
    }

    println!("\nRenaming:");
    for (prefix, paths) in &groups {
        if let Some(barcode) = rename_map.get(prefix.as_str()) {
            rename_group(prefix, barcode, paths)?;
        }
    }

    println!("\nDone.");
    print_errors(&errors);
    Ok(())
}
