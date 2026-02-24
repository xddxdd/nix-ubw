{
  description = "nix-ubw development shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      perSystem =
        { pkgs, ... }:
        {
          devShells.default = pkgs.mkShell {
            nativeBuildInputs = with pkgs; [
              cargo
              pkg-config
              rustc
              rustfmt
            ];

            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
          };

          packages.parallel-test =
            let
              value = "test2";
            in
            pkgs.runCommand "parallel" { nativeBuildInputs = [ pkgs.parallel ]; } ''
              mkdir -p $out

              for i in $(seq 1 100); do
                echo "sleep 5; echo ${value} > $out/$i.txt" >> job.txt
              done

              cat job.txt | parallel -j50
            '';
        };
    };
}
