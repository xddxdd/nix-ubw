use crate::resources::resource_profile::ResourceProfile;

/// Look up the resource profile for a process given its resolved argv.
/// `args[0]` is expected to already be the resolved basename (as returned
/// by `read_cmdline`).
///
/// Returns `None` if the process has no specific profile and should not be
/// throttled.
pub fn profile_for(args: &[String]) -> Option<ResourceProfile> {
    let name = args.first().map(|s| s.as_str())?;

    let profile = match name {
        // --- C / C++ compilers ---
        "cc" | "gcc" | "g++" | "c++" | "clang" | "clang++" => ResourceProfile::new(1, 1),

        // --- Rust compiler (parallel codegen, memory-hungry) ---
        "rustc" => ResourceProfile::new(4, 4),

        // --- LLVM backend / linker ---
        "llc" | "lld" | "ld.lld" => ResourceProfile::new(1, 2),

        // --- GNU linker / gold ---
        "ld" | "gold" => ResourceProfile::new(1, 1),

        // --- Go compiler ---
        "go" => ResourceProfile::new(1, 1),

        // --- Haskell (GHC is very memory hungry) ---
        "ghc" => ResourceProfile::new(1, 4),

        // --- JVM-based compilers ---
        "java" | "javac" | "scalac" | "kotlinc" => ResourceProfile::new(1, 2),

        // --- CUDA toolchain (GPU compile, 1 CPU but lots of RAM) ---
        "nvcc" | "ptxas" | "cicc" | "cudafe++" | "fatbinary" => ResourceProfile::new(1, 4),

        // Everything else (orchestrators, wrappers, etc.) is not throttled.
        _ => return None,
    };

    Some(profile)
}
