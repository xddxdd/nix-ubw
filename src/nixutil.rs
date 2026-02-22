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
pub fn unwrap_nix_name(name: &str) -> &str {
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
pub fn resolve_basename(path: &str) -> &str {
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
