use sha2::{Digest, Sha256};
use std::{env, fs, process};
use vox_connections::remote_extensions::{InstallExtensionRequest, RemoteExtensionService};

fn main() {
    let mut args = env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: vox-connector-check <manifest.json>");
        process::exit(2);
    };
    if args.next().is_some() {
        eprintln!("usage: vox-connector-check <manifest.json>");
        process::exit(2);
    }
    let result = (|| {
        let bytes = fs::read(&path).map_err(|error| format!("read manifest: {error}"))?;
        let manifest: InstallExtensionRequest =
            serde_json::from_slice(&bytes).map_err(|error| format!("parse manifest: {error}"))?;
        RemoteExtensionService::validate_install_request(&manifest, false)
            .map_err(|error| format!("invalid manifest: {error}"))?;
        let canonical = serde_json::to_vec(&manifest)
            .map_err(|error| format!("serialize manifest: {error}"))?;
        Ok::<_, String>((
            manifest.external_key,
            hex::encode(Sha256::digest(canonical)),
        ))
    })();
    match result {
        Ok((key, digest)) => println!("valid connector {key}\nsha256:{digest}"),
        Err(error) => {
            eprintln!("{error}");
            process::exit(1);
        }
    }
}
