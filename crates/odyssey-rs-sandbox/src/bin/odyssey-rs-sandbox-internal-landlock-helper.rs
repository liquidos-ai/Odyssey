use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

#[cfg(target_os = "linux")]
use std::collections::BTreeMap;

#[cfg(target_os = "linux")]
use std::os::unix::process::CommandExt;

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
    {
        apply_landlock(&policy)?;
        exec_command(&command, args)
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = policy;
        let _ = command;
        let _ = args;
        Err("internal Landlock helper is only supported on Linux".to_string())
    }
}

#[cfg(target_os = "linux")]
fn exec_command(command: &Path, args: Vec<OsString>) -> Result<ExitCode, String> {
    let error = Command::new(command).args(args).exec();
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
    add_roots(&mut rights_by_path, &policy.exec_roots, exec_access());
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
        landlock::RulesetStatus::FullyEnforced => Ok(()),
        landlock::RulesetStatus::PartiallyEnforced => {
            Err("Landlock ruleset was only partially enforced by the kernel".to_string())
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
    AccessFs::ReadFile | AccessFs::ReadDir
}

#[cfg(target_os = "linux")]
fn exec_access() -> BitFlags<AccessFs> {
    read_access() | AccessFs::Execute
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

#[cfg(test)]
mod tests {
    use super::{LauncherPolicy, parse_args, resolve_rule_path};
    use pretty_assertions::assert_eq;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    #[cfg(target_os = "linux")]
    use super::{add_roots, exec_access, read_access, write_access};

    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn parse_args_collects_policy_and_command() {
        let temp = tempdir().expect("tempdir");
        let read = temp.path().join("read");
        let write = temp.path().join("write");
        let exec = temp.path().join("exec");
        std::fs::create_dir_all(&read).expect("create read");
        std::fs::create_dir_all(&write).expect("create write");
        std::fs::create_dir_all(&exec).expect("create exec");

        let args = vec![
            OsString::from("helper"),
            OsString::from("--read"),
            read.as_os_str().to_os_string(),
            OsString::from("--write"),
            write.as_os_str().to_os_string(),
            OsString::from("--exec"),
            exec.as_os_str().to_os_string(),
            OsString::from("--"),
            OsString::from("/bin/echo"),
            OsString::from("hello"),
        ];

        let (policy, command, command_args) = parse_args(args).expect("parse args");

        assert_eq!(
            policy.read_roots,
            vec![read.canonicalize().expect("canonical read")]
        );
        assert_eq!(
            policy.write_roots,
            vec![write.canonicalize().expect("canonical write")]
        );
        assert_eq!(
            policy.exec_roots,
            vec![exec.canonicalize().expect("canonical exec")]
        );
        assert_eq!(command, PathBuf::from("/bin/echo"));
        assert_eq!(command_args, vec![OsString::from("hello")]);
    }

    #[test]
    fn parse_args_rejects_missing_command_after_separator() {
        let error = parse_args([OsString::from("helper"), OsString::from("--")])
            .expect_err("missing command should fail");

        assert_eq!(error, "missing command after '--'");
    }

    #[test]
    fn parse_args_rejects_unsupported_flags() {
        let error = parse_args([
            OsString::from("helper"),
            OsString::from("--unknown"),
            OsString::from("--"),
            OsString::from("/bin/echo"),
        ])
        .expect_err("unsupported flag should fail");

        assert_eq!(error, "unsupported argument: --unknown");
    }

    #[test]
    fn parse_args_requires_absolute_command() {
        let error = parse_args([
            OsString::from("helper"),
            OsString::from("--"),
            OsString::from("echo"),
        ])
        .expect_err("relative command should fail");

        assert_eq!(error, "helper command must be absolute: echo");
    }

    #[cfg(unix)]
    #[test]
    fn parse_args_rejects_non_utf8_flags() {
        let error = parse_args([
            OsString::from("helper"),
            OsString::from_vec(vec![0xff]),
            OsString::from("--"),
            OsString::from("/bin/echo"),
        ])
        .expect_err("non-utf8 flag should fail");

        assert_eq!(error, "helper arguments must be valid UTF-8");
    }

    #[test]
    fn resolve_rule_path_requires_absolute_path() {
        let error = resolve_rule_path(Path::new("relative")).expect_err("relative path rejected");
        assert_eq!(error, "Landlock root must be absolute: relative");
    }

    #[test]
    fn resolve_rule_path_canonicalizes_existing_path() {
        let temp = tempdir().expect("tempdir");
        let child = temp.path().join("child");
        std::fs::create_dir_all(&child).expect("create child");

        let resolved = resolve_rule_path(&temp.path().join("child").join("..").join("child"))
            .expect("resolve path");

        assert_eq!(resolved, child.canonicalize().expect("canonical child"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn add_roots_merges_permissions_for_duplicate_paths() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().join("root");
        std::fs::create_dir_all(&root).expect("create root");

        let mut rights = std::collections::BTreeMap::new();
        add_roots(&mut rights, std::slice::from_ref(&root), read_access());
        add_roots(&mut rights, std::slice::from_ref(&root), write_access());

        let merged = rights.get(&root).expect("merged rights");
        assert!((*merged).contains(read_access()));
        assert!((*merged).contains(write_access()));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn read_and_write_access_do_not_grant_execute() {
        assert!(!read_access().contains(landlock::AccessFs::Execute));
        assert!(!write_access().contains(landlock::AccessFs::Execute));
        assert!(exec_access().contains(landlock::AccessFs::Execute));
    }

    #[test]
    fn launcher_policy_defaults_to_empty_roots() {
        let policy = LauncherPolicy::default();
        assert!(policy.read_roots.is_empty());
        assert!(policy.write_roots.is_empty());
        assert!(policy.exec_roots.is_empty());
    }
}
