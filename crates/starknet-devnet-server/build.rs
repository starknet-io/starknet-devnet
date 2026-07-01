use std::error::Error;
use std::fs;
use std::path::{Component, Path, PathBuf};

const UI_ASSETS_DIR: &str = "assets/ui";
const GENERATED_ASSETS_FILE: &str = "ui_assets.rs";

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let assets_dir = manifest_dir.join(UI_ASSETS_DIR);
    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let generated_assets = out_dir.join(GENERATED_ASSETS_FILE);

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", assets_dir.display());

    if !assets_dir.join("index.html").is_file() {
        return Err(format!(
            "missing built web UI at {}; run `npm --prefix web-ui ci` and `npm --prefix web-ui \
             run build:devnet` before building or publishing",
            assets_dir.display()
        )
        .into());
    }

    let mut assets = Vec::new();
    collect_assets(&assets_dir, &assets_dir, &mut assets)?;
    assets.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    for asset in &assets {
        println!("cargo:rerun-if-changed={}", asset.absolute_path.display());
    }

    let mut generated = String::from("const UI_ASSETS: &[Asset] = &[\n");
    for asset in assets {
        generated.push_str("    Asset { path: ");
        generated.push_str(&rust_string_literal(&asset.relative_path));
        generated.push_str(", bytes: include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/");
        generated.push_str(UI_ASSETS_DIR);
        generated.push_str("/\", ");
        generated.push_str(&rust_string_literal(&asset.relative_path));
        generated.push_str(")) },\n");
    }
    generated.push_str("];\n");

    fs::write(generated_assets, generated)?;

    Ok(())
}

struct AssetSource {
    relative_path: String,
    absolute_path: PathBuf,
}

fn collect_assets(
    root: &Path,
    current_dir: &Path,
    assets: &mut Vec<AssetSource>,
) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(current_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            collect_assets(root, &path, assets)?;
        } else if path.is_file() {
            let relative_path = slash_separated_path(path.strip_prefix(root)?);
            assets.push(AssetSource { relative_path, absolute_path: path });
        }
    }

    Ok(())
}

fn slash_separated_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn rust_string_literal(value: &str) -> String {
    format!("{value:?}")
}
