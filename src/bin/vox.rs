//! Authoring and deployment tooling use the same validators as the platform.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, env, fs, path::Path};
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
