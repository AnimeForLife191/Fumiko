use dotenvy::from_filename_iter;
use std::path::PathBuf;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR should always be set by cargo");

    let env_path = PathBuf::from(&manifest_dir)
        .join("..")
        .join("..")
        .join(".env");

    println!("cargo:warning=Looking for .env at {}", env_path.display());
    println!("cargo:rerun-if-changed={}", env_path.display());

    match from_filename_iter(&env_path) {
        Ok(iter) => {
            let mut found_any = false;
            for item in iter {
                let Ok((key, value)) = item else { continue };
                println!("cargo:warning=Loaded env var: {key}");
                println!("cargo:rustc-env={key}={value}");
                found_any = true;
            }
            if !found_any {
                println!("cargo:warning=.env was found but contained no valid entries");
            }
        }
        Err(e) => {
            println!("cargo:warning=Failed to load .env: {e}");
        }
    }
}
