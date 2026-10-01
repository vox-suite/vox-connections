//! Authoring and deployment tooling use the same validators as the platform.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, env, fs, io::Read, path::Path};
use vox_connections::{
    packages::{PackageMetadata, package_bytes},
    remote_extensions::{InstallExtensionRequest, RemoteExtensionService},
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Package {
    version: i32,
    manifest: InstallExtensionRequest,
    metadata: PackageMetadata,
}

const MAX_RETAINED_REPORT_BYTES: u64 = 256 * 1024;

/// Bind the local publication evidence to its exact retained bytes. This is
/// an integrity check, not independent verification of provider behavior.
fn validate_retained_report(path: &Path, review: &serde_json::Value) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "report.json must contain the retained independent operator report")?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_RETAINED_REPORT_BYTES {
        return Err("report.json must be a regular file of at most 256 KiB".into());
    }
    let file = fs::File::open(path).map_err(|_| "Cannot read report.json")?;
    let mut bytes = Vec::new();
    file.take(MAX_RETAINED_REPORT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read report.json")?;
    if bytes.is_empty()
        || bytes.iter().all(u8::is_ascii_whitespace)
        || bytes.len() as u64 > MAX_RETAINED_REPORT_BYTES
    {
        return Err("report.json must contain a nonempty report of at most 256 KiB".into());
    }
    let digest = hex::encode(Sha256::digest(&bytes));
    if review
        .pointer("/evidence/report_digest")
        .and_then(serde_json::Value::as_str)
        != Some(digest.as_str())
    {
        return Err("report.json bytes do not match review.json evidence.report_digest".into());
    }
    Ok(())
}

fn read_skill(root: &Path) -> Result<vox_connections::skills::PublishSkillRequest, String> {
    fn walk(
        root: &Path,
        current: &Path,
        files: &mut BTreeMap<String, String>,
    ) -> Result<(), String> {
        for entry in fs::read_dir(current).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() {
                return Err("Skill symlinks are unsupported".into());
            }
            if kind.is_dir() {
                if current != root
                    || !matches!(entry.file_name().to_str(), Some("references" | "assets"))
                {
                    return Err("Only bounded references and assets are supported; scripts are not executable skills".into());
                }
                walk(root, &path, files)?;
            } else if kind.is_file() {
                if files.len() >= 32 || entry.metadata().map_err(|e| e.to_string())?.len() > 16_384
                {
                    return Err("Skill file limits exceeded".into());
                }
                let name = path
                    .strip_prefix(root)
                    .map_err(|e| e.to_string())?
                    .to_str()
                    .ok_or("Invalid skill path")?
                    .replace('\\', "/");
                files.insert(name, fs::read_to_string(path).map_err(|e| e.to_string())?);
            } else {
                return Err("Special files are unsupported".into());
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    walk(root, root, &mut files)?;
    vox_connections::skill_format::import(&files).map_err(|e| e.to_string())
}

fn check(root: &Path) -> Result<Package, String> {
    let path = root.join("vox-package.json");
    if fs::symlink_metadata(&path)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
        || fs::metadata(&path).map_err(|e| e.to_string())?.len() > 256 * 1024
    {
        return Err("Invalid package file".into());
    }
    let package: Package = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if package.version < 1 {
        return Err("Version must be positive".into());
    }
    package.metadata.validate().map_err(|e| e.to_string())?;
    RemoteExtensionService::validate_install_request(&package.manifest, false)
        .map_err(|e| e.to_string())?;
    for pinned in &package.metadata.skills {
        let skill = read_skill(&root.join("skills").join(&pinned.external_key))?;
        if skill.external_key != pinned.external_key
            || vox_connections::skills::content_digest(&skill).map_err(|e| e.to_string())?
                != pinned.digest
        {
            return Err("Bundled skill digest mismatch".into());
        }
    }
    Ok(package)
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1)
    }
}
async fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["defaults","publish",deployment]=>{
            let url=env::var("DATABASE_URL").map_err(|_|"DATABASE_URL is required")?;
            let pool=sqlx::PgPool::connect(&url).await.map_err(|e|e.to_string())?;
            let count=vox_connections::defaults::publish(pool,deployment).await.map_err(|e|e.to_string())?;
            println!("Published {count} curated skills; no user installation or enablement changed.");
        }
        ["skill","check",directory]=>{
            let skill=read_skill(Path::new(directory))?;
            println!("{}",serde_json::to_string_pretty(&serde_json::json!({"skill":skill,"digest":vox_connections::skills::content_digest(&skill).map_err(|e|e.to_string())?})).map_err(|e|e.to_string())?);
        }
        ["package","init",directory] | ["package","init",directory,"--mcp"]=>{
            let root=Path::new(directory); fs::create_dir(root).map_err(|e|e.to_string())?;
            let manifest:InstallExtensionRequest=serde_json::from_str(include_str!("../../examples/mcp/read-only-manifest.json")).map_err(|e|e.to_string())?;
            fs::write(root.join("vox-package.json"),serde_json::to_vec_pretty(&Package {version:1,manifest,metadata:PackageMetadata::oauth()}).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            println!("Created {}. Declare the real endpoint, tools, schemas and recipients before review.",root.display());
        }
        ["package","check",directory]=>{
            let p=check(Path::new(directory))?;
            println!("sha256:{}",hex::encode(Sha256::digest(package_bytes(&p.manifest,&p.metadata).map_err(|e|e.to_string())?)));
        }
        ["package","test",directory,"--sandbox"]=>{
            let package=check(Path::new(directory))?;
            let token=env::var("VOX_TEST_ACCESS_TOKEN").unwrap_or_default();
            let (info,tools)=vox_connections::connected_apps::mcp::McpDiscoveryClient::default()
                .discover(&package.manifest.endpoint_url,&token,std::time::Duration::from_secs(15),true)
                .await.map_err(|e|format!("MCP discovery failed: {e}"))?;
            if info.get("vox_protocol_version").and_then(serde_json::Value::as_str)!=Some(&package.metadata.protocol_version) {return Err("Negotiated MCP protocol differs from reviewed package".into())}
            for capability in &package.manifest.capabilities {
                if !tools.iter().any(|tool|tool.get("name").and_then(serde_json::Value::as_str)==Some(&capability.external_key)
                    && tool.get("inputSchema")==Some(&capability.input_schema)) {
                    return Err(format!("Tool or input schema drift: {}",capability.external_key));
                }
            }
            println!("{}",serde_json::json!({"declaration_and_discovery":"passed","reported_tools":tools.len(),"protocol":package.metadata.protocol_version,"behavior_certified":false}));
        }
        ["package","publish",directory,endpoint,deployment]=>{
            let p=check(Path::new(directory))?;
            let review_path=Path::new(directory).join("review.json");
            let review:serde_json::Value=serde_json::from_slice(&fs::read(review_path).map_err(|_|"review.json must contain independent operator evidence")?).map_err(|e|e.to_string())?;
            let digest=hex::encode(Sha256::digest(package_bytes(&p.manifest,&p.metadata).map_err(|e|e.to_string())?));
            vox_connections::packages::validate_publish_review(&review,&digest,p.version,&p.metadata,&p.manifest)
                .map_err(|_|"review.json must attest this exact package version, digest, protocol, live inventory and behavior (including read effects), with a retained report digest")?;
            validate_retained_report(&Path::new(directory).join("report.json"), &review)?;
            let deployment_id=deployment.parse().map_err(|_|"Deployment must be a UUID")?;
            let token=env::var("VOX_OPERATOR_TOKEN").map_err(|_|"VOX_OPERATOR_TOKEN is required in the environment")?;
            let url=url::Url::parse(endpoint).map_err(|e|e.to_string())?;
            if url.scheme()!="https" || !url.username().is_empty() || url.password().is_some() { return Err("Publish to an authenticated HTTPS Core endpoint".into()) }
            let client=reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(std::time::Duration::from_secs(30)).build().map_err(|e|e.to_string())?;
            let response=client.post(format!("{}/v1/connector-packages/publish",endpoint.trim_end_matches('/'))).bearer_auth(token).json(&vox_connections::packages::PublishPackage {deployment_id,version:p.version,manifest:p.manifest,metadata:p.metadata,review}).send().await.map_err(|_|"Publication transport failed")?;
            if !response.status().is_success() { return Err(format!("Publication rejected: HTTP {}",response.status())) }
            println!("Published the immutable reviewed package.");
        }
        _=>return Err("Usage: vox defaults publish <deployment-key> | skill check <directory> | package init <directory> [--mcp] | package check <directory> | package test <directory> --sandbox | package publish <directory> <https-core-url> <deployment-uuid>".into()),
    }
    Ok(())
}

#[cfg(test)]
mod retained_report_tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    struct ReportFixture(PathBuf);

    impl ReportFixture {
        fn new() -> Self {
            let path = env::temp_dir().join(format!("vox-report-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn report(&self) -> PathBuf {
            self.0.join("report.json")
        }
    }

    impl Drop for ReportFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn review_for(bytes: &[u8]) -> serde_json::Value {
        json!({"evidence": {"report_digest": hex::encode(Sha256::digest(bytes))}})
    }

    #[test]
    fn publication_evidence_matches_exact_retained_bytes() {
        let fixture = ReportFixture::new();
        let bytes = b"{\"independent_review\":\"retained evidence\"}\n";
        fs::write(fixture.report(), bytes).unwrap();
        let review = review_for(bytes);
        assert!(validate_retained_report(&fixture.report(), &review).is_ok());
        fs::write(fixture.report(), &bytes[..bytes.len() - 1]).unwrap();
        assert!(validate_retained_report(&fixture.report(), &review).is_err());
        assert!(validate_retained_report(&fixture.report(), &json!({})).is_err());
    }

    #[test]
    fn missing_empty_oversized_and_non_file_reports_are_rejected() {
        let fixture = ReportFixture::new();
        assert!(validate_retained_report(&fixture.report(), &review_for(b"evidence")).is_err());
        for bytes in [
            vec![],
            b" \n\t".to_vec(),
            vec![b'x'; MAX_RETAINED_REPORT_BYTES as usize + 1],
        ] {
            fs::write(fixture.report(), &bytes).unwrap();
            assert!(validate_retained_report(&fixture.report(), &review_for(&bytes)).is_err());
        }
        fs::remove_file(fixture.report()).unwrap();
        fs::create_dir(fixture.report()).unwrap();
        assert!(validate_retained_report(&fixture.report(), &review_for(b"evidence")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_reports_are_rejected() {
        let fixture = ReportFixture::new();
        let bytes = b"retained evidence";
        let target = fixture.0.join("target.json");
        fs::write(&target, bytes).unwrap();
        std::os::unix::fs::symlink(target, fixture.report()).unwrap();
        assert!(validate_retained_report(&fixture.report(), &review_for(bytes)).is_err());
    }
}
