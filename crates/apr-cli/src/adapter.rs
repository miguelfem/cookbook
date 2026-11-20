//! Owner-reviewable, stdlib-only adapter preparation. No grants or credentials.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use apr_connectors::mcp::{ToolContract, pin_executable};
use apr_core::validate_service_schema;
use apr_runtime::{Result, RuntimeError};
use clap::Subcommand;
use serde_json::{Value, json};

const ADAPTER: &str = include_str!("../../../integrations/adapters/adapter.py");
const BOOTSTRAP: &str = include_str!("../../../integrations/adapters/bootstrap.py");
const MAX_SPEC_BYTES: u64 = 64 * 1024;
// The MCP request/result line also carries JSON-RPC metadata and headers.
const MAX_PAYLOAD_BYTES: u64 = 60 * 1024;

#[derive(Subcommand)]
pub enum AdapterCommand {
    /// Freeze one fixed JSON-in/JSON-out CLI as a reviewable stdio MCP adapter.
    Prepare {
        #[arg(long)]
        spec: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        python: PathBuf,
    },
    /// Check local files and hashes without invoking the CLI or creating a grant.
    Check {
        #[arg(long)]
        directory: PathBuf,
    },
}

pub fn execute(command: AdapterCommand) -> Result<Value> {
    match command {
        AdapterCommand::Prepare {
            spec,
            output,
            python,
        } => prepare(&spec, &output, &python),
        AdapterCommand::Check { directory } => check(&directory),
    }
}

fn invalid(message: &str) -> RuntimeError {
    RuntimeError::InvalidInput(message.into())
}
fn digest(path: &Path) -> Result<String> {
    pin_executable(path).map_err(|_| invalid("file must be a readable regular file under 512 MiB"))
}
fn executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}
fn read_json(path: &Path) -> Result<Value> {
    if !path.is_absolute() {
        return Err(invalid("path must be absolute"));
    }
    let metadata = fs::metadata(path).map_err(|_| invalid("file is unavailable"))?;
    if !metadata.is_file() || metadata.len() > MAX_SPEC_BYTES {
        return Err(invalid("JSON file is not a regular file under 64 KiB"));
    }
    let bytes = fs::read(path).map_err(|_| invalid("file is unreadable"))?;
    serde_json::from_slice(&bytes).map_err(|_| invalid("invalid JSON file"))
}
fn write_json(path: &Path, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| invalid("invalid adapter JSON"))?;
    fs::write(path, bytes).map_err(|_| invalid("cannot write adapter file"))
}
fn validated_spec(input: &Value) -> Result<Value> {
    let object = input
        .as_object()
        .ok_or_else(|| invalid("spec must be an object"))?;
    let allowed = [
        "schema_version",
        "name",
        "cli_executable",
        "cli_args",
        "dependency_files",
        "credential_env",
        "input_schema",
        "output_schema",
        "timeout_ms",
        "max_input_bytes",
        "max_output_bytes",
    ];
    if object.keys().any(|key| !allowed.contains(&key.as_str())) || input["schema_version"] != 1 {
        return Err(invalid("unknown spec field or schema_version"));
    }
    let name = input["name"]
        .as_str()
        .ok_or_else(|| invalid("name is required"))?;
    if name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.-/".contains(&c))
    {
        return Err(invalid("name must be a bounded MCP tool name"));
    }
    let executable = input["cli_executable"]
        .as_str()
        .ok_or_else(|| invalid("cli_executable is required"))?;
    if !Path::new(executable).is_absolute() || executable.len() > 4096 {
        return Err(invalid("cli_executable must be absolute"));
    }
    let args = input["cli_args"]
        .as_array()
        .ok_or_else(|| invalid("cli_args must be an array"))?;
    if args.len() > 32
        || !args.iter().all(|a| {
            a.as_str()
                .is_some_and(|s| s.len() <= 4096 && !s.contains('\0'))
        })
    {
        return Err(invalid("cli_args are too large or invalid"));
    }
    let deps = input["dependency_files"]
        .as_array()
        .ok_or_else(|| invalid("dependency_files must be an array"))?;
    if deps.len() > 64
        || !deps.iter().all(|d| {
            d.as_str()
                .is_some_and(|s| Path::new(s).is_absolute() && s.len() <= 4096)
        })
    {
        return Err(invalid(
            "dependency_files must contain at most 64 absolute paths",
        ));
    }
    let mut unique = BTreeSet::new();
    for dependency in deps {
        if !unique.insert(dependency.as_str().unwrap()) {
            return Err(invalid("duplicate dependency_file"));
        }
    }
    for arg in args {
        let arg = arg.as_str().unwrap();
        if Path::new(arg).is_absolute()
            && Path::new(arg).is_file()
            && !unique.contains(arg)
            && arg != executable
        {
            return Err(invalid(
                "absolute file argument must be listed in dependency_files",
            ));
        }
    }
    let credential_env = input["credential_env"].as_str();
    if input
        .get("credential_env")
        .is_some_and(|v| !v.is_null() && !v.is_string())
        || credential_env.is_some_and(|s| {
            s.is_empty()
                || s.len() > 80
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                || s.starts_with("LD_")
                || s.starts_with("DYLD_")
                || matches!(
                    s,
                    "PATH"
                        | "HOME"
                        | "SHELL"
                        | "ENV"
                        | "BASH_ENV"
                        | "PYTHONPATH"
                        | "PYTHONHOME"
                        | "NODE_OPTIONS"
                        | "RUBYOPT"
                )
        })
    {
        return Err(invalid(
            "credential_env must be one safe uppercase variable",
        ));
    }
    for key in ["input_schema", "output_schema"] {
        let schema = input
            .get(key)
            .ok_or_else(|| invalid("input_schema and output_schema are required"))?;
        validate_service_schema(schema).map_err(|_| invalid("invalid JSON schema"))?;
    }
    ToolContract {
        name: name.to_owned(),
        input_schema: input["input_schema"].clone(),
        output_schema: Some(input["output_schema"].clone()),
        ui_resource_uri: None,
    }
    .validate()
    .map_err(|_| invalid("tool schema exceeds connector bounds"))?;
    let timeout = input["timeout_ms"]
        .as_u64()
        .ok_or_else(|| invalid("timeout_ms is required"))?;
    let max_input = input["max_input_bytes"]
        .as_u64()
        .ok_or_else(|| invalid("max_input_bytes is required"))?;
    let max_output = input["max_output_bytes"]
        .as_u64()
        .ok_or_else(|| invalid("max_output_bytes is required"))?;
    if !(100..=120_000).contains(&timeout)
        || !(1..=MAX_PAYLOAD_BYTES).contains(&max_input)
        || !(1..=MAX_PAYLOAD_BYTES).contains(&max_output)
    {
        return Err(invalid("timeout or byte bound is outside supported range"));
    }
    let mut frozen = input.clone();
    frozen["cli_sha256"] = json!(digest(Path::new(executable))?);
    frozen["dependency_sha256"] = Value::Array(
        deps.iter()
            .map(|v| {
                digest(Path::new(v.as_str().unwrap())).map(|sha| json!({"path":v,"sha256":sha}))
            })
            .collect::<Result<Vec<_>>>()?,
    );
    if serde_json::to_vec_pretty(&frozen)
        .map_err(|_| invalid("invalid adapter spec"))?
        .len() as u64
        > MAX_SPEC_BYTES
    {
        return Err(invalid("frozen adapter spec exceeds 64 KiB"));
    }
    Ok(frozen)
}

fn prepare(spec: &Path, output: &Path, python: &Path) -> Result<Value> {
    if !output.is_absolute() || !python.is_absolute() || output.exists() {
        return Err(invalid(
            "output must be a new absolute directory and python must be absolute",
        ));
    }
    let frozen = validated_spec(&read_json(spec)?)?;
    let python_sha256 = digest(python)?;
    if !executable(python) || !executable(Path::new(frozen["cli_executable"].as_str().unwrap())) {
        return Err(invalid("python and target CLI must be executable"));
    }
    fs::create_dir(output).map_err(|_| invalid("cannot create new output directory"))?;
    let source = output.join("adapter.py");
    let config = output.join("spec.json");
    fs::write(&source, ADAPTER).map_err(|_| invalid("cannot write adapter source"))?;
    write_json(&config, &frozen)?;
    let source_sha256 = digest(&source)?;
    let spec_sha256 = digest(&config)?;
    let args = json!([
        "-I",
        "-S",
        "-u",
        "-c",
        BOOTSTRAP,
        config,
        source,
        spec_sha256,
        source_sha256
    ]);
    let manifest = json!({"schema_version":1,"python":python,"python_sha256":python_sha256,
        "spec_sha256":spec_sha256,"source_sha256":source_sha256,"args":args});
    write_json(&output.join("manifest.json"), &manifest)?;
    check(output)
}

fn check(directory: &Path) -> Result<Value> {
    if !directory.is_absolute() {
        return Err(invalid("directory must be absolute"));
    }
    let manifest = read_json(&directory.join("manifest.json"))?;
    if manifest["schema_version"] != 1 {
        return Err(invalid("unsupported adapter manifest"));
    }
    let spec = read_json(&directory.join("spec.json"))?;
    if spec["schema_version"] != 1
        || !spec["name"].is_string()
        || !spec["dependency_sha256"].is_array()
    {
        return Err(invalid("invalid frozen adapter spec"));
    }
    let spec_path = directory.join("spec.json");
    let source_path = directory.join("adapter.py");
    let expected_args = json!([
        "-I",
        "-S",
        "-u",
        "-c",
        BOOTSTRAP,
        spec_path,
        source_path,
        manifest["spec_sha256"],
        manifest["source_sha256"]
    ]);
    let paths = [
        ("python", manifest["python"].as_str()),
        ("spec", spec_path.to_str()),
        ("source", source_path.to_str()),
        ("cli", spec["cli_executable"].as_str()),
    ];
    let expected = [
        manifest["python_sha256"].as_str(),
        manifest["spec_sha256"].as_str(),
        manifest["source_sha256"].as_str(),
        spec["cli_sha256"].as_str(),
    ];
    let mut issues = Vec::new();
    if manifest["args"] != expected_args {
        issues.push("launcher args changed".to_owned());
    }
    for ((label, path), hash) in paths.into_iter().zip(expected) {
        let Some(path) = path else {
            return Err(invalid("invalid manifest path"));
        };
        let Some(hash) = hash else {
            return Err(invalid("invalid manifest digest"));
        };
        if digest(Path::new(path)).ok().as_deref() != Some(hash) {
            issues.push(format!("{label} missing or changed"));
        }
    }
    for (label, path) in [
        ("python", manifest["python"].as_str()),
        ("cli", spec["cli_executable"].as_str()),
    ] {
        if !path.is_some_and(|path| executable(Path::new(path))) {
            issues.push(format!("{label} is not executable"));
        }
    }
    let dependencies = spec["dependency_sha256"]
        .as_array()
        .ok_or_else(|| invalid("invalid dependency pins"))?;
    for dep in dependencies {
        let path = dep["path"]
            .as_str()
            .ok_or_else(|| invalid("invalid dependency path"))?;
        let hash = dep["sha256"]
            .as_str()
            .ok_or_else(|| invalid("invalid dependency digest"))?;
        if digest(Path::new(path)).ok().as_deref() != Some(hash) {
            issues.push(format!("dependency missing or changed: {path}"));
        }
    }
    let binding = if issues.is_empty() {
        json!({"protocol":"2026-07-28","transport":{"kind":"stdio","executable":manifest["python"],"sha256":manifest["python_sha256"],"args":expected_args,"credential_env":spec["credential_env"]},
            "tool":{"name":spec["name"],"input_schema":spec["input_schema"],"output_schema":spec["output_schema"]}})
    } else {
        Value::Null
    };
    Ok(
        json!({"ready":issues.is_empty(),"issues":issues,"grant_created":false,
        "credential_required":spec["credential_env"].is_string(),"binding_proposal":binding,
        "review":"The adapter and target CLI run with the owner's OS privileges; review target behavior, destinations and all transitive dependencies before approval."}),
    )
}
