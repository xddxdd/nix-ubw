# Nix-UBW (Unlimited Build Works)

> **EXPERIMENTAL**. Might not have performance benefits due to ptrace overhead.

Orchestrates resource heavy processes from your Nix builds, so you can run **unlimited(*)** large **builds** in parallel, and they just **work** without resource contention and/or out-of-memory errors.

> (*) Within reason for Linux's scheduler to handle.

# How does it work

TODO: expand

Ptrace on execve from `nix-daemon` and all subprocesses. When it detects resource heavy processes (compilers, linkers, compressors, etc), uses global counter to limit the number of parallel processes, and pause these processes on trace point until resource frees up.

# Usage

TODO: complete rest of README, add Nix development shell, etc.

# Future Improvements

- [ ] Read rules from external file
- [ ] Builtin rules for more kinds of processes

# AI usage disclaimer

This project is completed with the help of LLM. I have reviewed LLM generated code line by line, so you can consider them the same quality as if I wrote these code by hand.

# License

GPLv3 or later.
