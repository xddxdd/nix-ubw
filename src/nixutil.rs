use std::fs;

use nix::unistd::Pid;

/// Read /proc/<pid>/cmdline and return the arguments as a Vec<String>.
/// The first argument (argv[0]) is automatically resolved to its unwrapped
/// basename via `resolve_basename`.
pub fn read_cmdline(pid: Pid) -> Option<Vec<String>> {
    let path = format!("/proc/{}/cmdline", pid);
    let data = fs::read(&path).ok()?;
    let mut args: Vec<String> = data
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    if let Some(first) = args.first_mut() {
        *first = resolve_basename(first).to_owned();
    }
    Some(args)
}

/// Unwrap a NixOS-wrapped executable name by stripping matched pairs of
/// leading `.` and trailing `-wrapped`.
///
/// NixOS wraps executables by prepending `.` and appending `-wrapped` for each
/// layer of wrapping:
/// - `.gcc-wrapped` → `gcc`
/// - `..gcc-wrapped-wrapped` → `gcc`
///
/// Names without matching pairs are left unchanged:
/// - `.hidden-file` → `.hidden-file`
/// - `gcc-wrapped` (no leading `.`) → `gcc-wrapped`
fn unwrap_nix_name(name: &str) -> &str {
    let mut name = name;
    while let Some(stripped) = name.strip_suffix("-wrapped") {
        if let Some(stripped) = stripped.strip_prefix('.') {
            name = stripped;
        } else {
            break;
        }
    }
    name
}

/// Extract the basename from a path and unwrap NixOS wrapper names.
fn resolve_basename(path: &str) -> &str {
    let basename = path.rsplit('/').next().unwrap_or(path);
    unwrap_nix_name(basename)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unwrap_single_wrap() {
        assert_eq!(unwrap_nix_name(".gcc-wrapped"), "gcc");
        assert_eq!(unwrap_nix_name(".sleep-wrapped"), "sleep");
    }

    #[test]
    fn test_unwrap_double_wrap() {
        assert_eq!(unwrap_nix_name("..gcc-wrapped-wrapped"), "gcc");
    }

    #[test]
    fn test_unwrap_triple_wrap() {
        assert_eq!(unwrap_nix_name("...gcc-wrapped-wrapped-wrapped"), "gcc");
    }

    #[test]
    fn test_no_wrap() {
        assert_eq!(unwrap_nix_name("gcc"), "gcc");
        assert_eq!(unwrap_nix_name("sleep"), "sleep");
    }

    #[test]
    fn test_hidden_file_not_stripped() {
        assert_eq!(unwrap_nix_name(".hidden-file"), ".hidden-file");
    }

    #[test]
    fn test_no_leading_dot_not_stripped() {
        assert_eq!(unwrap_nix_name("gcc-wrapped"), "gcc-wrapped");
    }

    #[test]
    fn test_mismatched_pairs_partial() {
        // One leading dot but two -wrapped suffixes: only one pair stripped
        assert_eq!(unwrap_nix_name(".gcc-wrapped-wrapped"), "gcc-wrapped");
    }

    #[test]
    fn test_resolve_basename_full_path() {
        assert_eq!(
            resolve_basename("/nix/store/abc-gcc/bin/.gcc-wrapped"),
            "gcc"
        );
    }

    #[test]
    fn test_resolve_basename_plain() {
        assert_eq!(resolve_basename("/usr/bin/sleep"), "sleep");
    }

    #[test]
    fn test_resolve_basename_no_path() {
        assert_eq!(resolve_basename("gcc"), "gcc");
    }
}
