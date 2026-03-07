use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

#[cfg(target_os = "linux")]
use landlock::{
    ABI, Access, AccessFs, BitFlags, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr,
};

#[cfg(target_os = "linux")]
const TARGET_ABI: ABI = ABI::V5;

#[derive(Debug, Default)]
struct LauncherPolicy {
    read_roots: Vec<PathBuf>,
    write_roots: Vec<PathBuf>,
    exec_roots: Vec<PathBuf>,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("odyssey-rs-sandbox-internal-landlock-helper: {error}");
            ExitCode::from(126)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let (policy, command, args) = parse_args(std::env::args_os())?;
    #[cfg(target_os = "linux")]
    apply_landlock(&policy)?;
    #[cfg(not(target_os = "linux"))]
    {
        let _ = policy;
        return Err("internal Landlock helper is only supported on Linux".to_string());
    }

    let error = Command::new(&command).args(args).exec();
    Err(format!("failed to exec {}: {error}", command.display()))
}

fn parse_args(
    args: impl IntoIterator<Item = OsString>,
) -> Result<(LauncherPolicy, PathBuf, Vec<OsString>), String> {
    let mut iter = args.into_iter();
    let _program_name = iter.next();

    let mut policy = LauncherPolicy::default();
    let mut command = None;
    let mut command_args = Vec::new();
    let mut in_command = false;

    while let Some(arg) = iter.next() {
        if in_command {
            command_args.push(arg);
            continue;
        }

        match arg.to_str() {
            Some("--") => {
                command = iter.next().map(PathBuf::from);
                if command.is_none() {
                    return Err("missing command after '--'".to_string());
                }
                in_command = true;
            }
            Some("--read") => {
                let value = iter
                    .next()
                    .ok_or_else(|| "missing value for --read".to_string())?;
                policy
                    .read_roots
                    .push(resolve_rule_path(Path::new(&value))?);
            }
            Some("--write") => {
                let value = iter
                    .next()
                    .ok_or_else(|| "missing value for --write".to_string())?;
                policy
                    .write_roots
                    .push(resolve_rule_path(Path::new(&value))?);
            }
            Some("--exec") => {
                let value = iter
                    .next()
                    .ok_or_else(|| "missing value for --exec".to_string())?;
                policy
                    .exec_roots
                    .push(resolve_rule_path(Path::new(&value))?);
            }
            Some(flag) => {
                return Err(format!("unsupported argument: {flag}"));
            }
            None => {
                return Err("helper arguments must be valid UTF-8".to_string());
            }
        }
    }

    let command = command.ok_or_else(|| "missing command to execute".to_string())?;
    if !command.is_absolute() {
        return Err(format!(
            "helper command must be absolute: {}",
            command.display()
        ));
    }

    Ok((policy, command, command_args))
}

fn resolve_rule_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!(
            "Landlock root must be absolute: {}",
            path.display()
        ));
    }
    path.canonicalize()
        .map_err(|error| format!("failed to resolve {}: {error}", path.display()))
}

#[cfg(target_os = "linux")]
fn apply_landlock(policy: &LauncherPolicy) -> Result<(), String> {
    let mut rights_by_path: BTreeMap<PathBuf, BitFlags<AccessFs>> = BTreeMap::new();
    add_roots(&mut rights_by_path, &policy.read_roots, read_access());
    add_roots(&mut rights_by_path, &policy.exec_roots, read_access());
    add_roots(
        &mut rights_by_path,
        &policy.write_roots,
        read_access() | write_access(),
    );

    if rights_by_path.is_empty() {
        return Err("refusing to apply an empty Landlock policy".to_string());
    }

    let mut ruleset = Ruleset::default()
        .handle_access(AccessFs::from_all(TARGET_ABI))
        .map_err(|error| format!("failed to handle Landlock access: {error}"))?
        .create()
        .map_err(|error| format!("failed to create Landlock ruleset: {error}"))?;

    for (path, access) in rights_by_path {
        let path_fd = PathFd::new(&path)
            .map_err(|error| format!("failed to open Landlock path {}: {error}", path.display()))?;
        ruleset = ruleset
            .add_rule(PathBeneath::new(path_fd, access))
            .map_err(|error| {
                format!(
                    "failed to add Landlock rule for {}: {error}",
                    path.display()
                )
            })?;
    }

    let status = ruleset
        .restrict_self()
        .map_err(|error| format!("failed to restrict process with Landlock: {error}"))?;

    match status.ruleset {
        landlock::RulesetStatus::FullyEnforced | landlock::RulesetStatus::PartiallyEnforced => {
            Ok(())
        }
        landlock::RulesetStatus::NotEnforced => {
            Err("Landlock ruleset was not enforced by the kernel".to_string())
        }
    }
}

#[cfg(target_os = "linux")]
fn add_roots(
    rights_by_path: &mut BTreeMap<PathBuf, BitFlags<AccessFs>>,
    paths: &[PathBuf],
    access: BitFlags<AccessFs>,
) {
    for path in paths {
        rights_by_path
            .entry(path.clone())
            .and_modify(|existing| *existing |= access)
            .or_insert(access);
    }
}

#[cfg(target_os = "linux")]
fn read_access() -> BitFlags<AccessFs> {
    AccessFs::ReadFile | AccessFs::ReadDir | AccessFs::Execute
}

#[cfg(target_os = "linux")]
fn write_access() -> BitFlags<AccessFs> {
    AccessFs::WriteFile
        | AccessFs::MakeChar
        | AccessFs::MakeDir
        | AccessFs::MakeReg
        | AccessFs::MakeSock
        | AccessFs::MakeFifo
        | AccessFs::MakeBlock
        | AccessFs::MakeSym
        | AccessFs::RemoveFile
        | AccessFs::RemoveDir
        | AccessFs::Refer
        | AccessFs::Truncate
}
