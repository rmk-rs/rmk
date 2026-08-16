{
  description = "RMK Rust development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    nixpkgs,
    fenix,
    flake-utils,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = nixpkgs.legacyPackages.${system};
        lib = pkgs.lib;
        fenixPkgs = fenix.packages.${system};

        embeddedTargets = [
          "thumbv6m-none-eabi"
          "thumbv7m-none-eabi"
          "thumbv7em-none-eabi"
          "thumbv7em-none-eabihf"
          "thumbv8m.main-none-eabihf"
          "riscv32imc-unknown-none-elf"
          "riscv32imac-unknown-none-elf"
          "wasm32-unknown-unknown"
        ];

        stableToolchain = fenixPkgs.combine (
          [
            fenixPkgs.stable.cargo
            fenixPkgs.stable.clippy
            fenixPkgs.stable.llvm-tools
            fenixPkgs.stable.rust-src
            fenixPkgs.stable.rustc
            fenixPkgs.stable.rustfmt
            fenixPkgs.stable.rust-analyzer
          ]
          ++ map (target: fenixPkgs.targets.${target}.stable.rust-std) embeddedTargets
        );

        # RMK uses nightly for formatting and for a small smoke-check matrix.
        nightlyToolchain = fenixPkgs.combine [
          fenixPkgs.latest.cargo
          fenixPkgs.latest.rustc
          fenixPkgs.latest.rustfmt
        ];

        # The repository's scripts intentionally use rustup-style invocations
        # such as `cargo +stable` and `rustfmt +nightly`. Keep that interface
        # while sourcing those toolchains from Fenix. The Xtensa-only `+esp`
        # toolchain remains managed by espup and is delegated to rustup.
        mkToolchainDispatcher = command:
          pkgs.writeShellScriptBin command ''
            toolchain=${lib.escapeShellArg (toString stableToolchain)}
            case "''${1:-}" in
              +stable)
                shift
                ;;
              +nightly)
                toolchain=${lib.escapeShellArg (toString nightlyToolchain)}
                shift
                ;;
              +esp)
                shift
                exec ${pkgs.rustup}/bin/rustup run esp ${command} "$@"
                ;;
              +*)
                echo "unsupported Rust toolchain: $1" >&2
                exit 2
                ;;
            esac

            export PATH="$toolchain/bin:$PATH"
            export RUSTC="$toolchain/bin/rustc"
            export RUSTDOC="$toolchain/bin/rustdoc"
            exec "$toolchain/bin/${command}" "$@"
          '';

        toolchainDispatchers = pkgs.symlinkJoin {
          name = "rmk-rust-toolchain-dispatchers";
          paths = map mkToolchainDispatcher [
            "cargo"
            "clippy-driver"
            "rustc"
            "rustdoc"
            "rustfmt"
          ];
        };

        # cargo-batch is not packaged in nixpkgs. This implements the subset
        # used by .github/ci/check.sh, preserving its shared target directory
        # while running each `---`-delimited Cargo command in order.
        cargoBatch = pkgs.writeShellScriptBin "cargo-batch" ''
          set -euo pipefail

          if [[ "''${1:-}" == batch ]]; then
            shift
          fi

          common_args=()
          while (( $# > 0 )) && [[ "$1" != --- ]]; do
            common_args+=("$1")
            shift
          done

          while (( $# > 0 )); do
            [[ "$1" == --- ]] || {
              echo "cargo-batch: expected ---, got $1" >&2
              exit 2
            }
            shift
            (( $# > 0 )) || {
              echo "cargo-batch: missing Cargo command after ---" >&2
              exit 2
            }

            cargo_command="$1"
            shift
            command_args=()
            while (( $# > 0 )) && [[ "$1" != --- ]]; do
              command_args+=("$1")
              shift
            done

            ${toolchainDispatchers}/bin/cargo \
              "$cargo_command" \
              "''${common_args[@]}" \
              "''${command_args[@]}"
          done
        '';

        shellPackages = [
          toolchainDispatchers
          stableToolchain
          cargoBatch
          pkgs.cargo-binutils
          pkgs.cargo-expand
          pkgs.cargo-make
          pkgs.cargo-nextest
          pkgs.dbus
          pkgs.espflash
          pkgs.espup
          pkgs.flip-link
          pkgs.just
          pkgs.llvmPackages.libclang
          pkgs.nodejs
          pkgs.pkg-config
          pkgs.probe-rs-tools
          pkgs.wasm-pack
        ];
      in {
        formatter = pkgs.alejandra;

        checks.toolchains =
          pkgs.runCommand "rmk-toolchains-check" {
            nativeBuildInputs = shellPackages;
          } ''
            ${toolchainDispatchers}/bin/cargo +stable --version
            ${toolchainDispatchers}/bin/cargo +nightly --version
            ${toolchainDispatchers}/bin/rustfmt +nightly --version
            cargo-nextest --version
            cargo-expand --version
            touch "$out"
          '';

        devShells.default = pkgs.mkShell {
          packages = shellPackages;

          env = {
            CARGO_NET_GIT_FETCH_WITH_CLI = "true";
            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
            RUST_MIN_STACK = "67108864";
            RUST_SRC_PATH = "${stableToolchain}/lib/rustlib/src/rust/library";
          };

          shellHook = ''
            export PATH="${toolchainDispatchers}/bin:$PATH"
          '';
        };
      }
    );
}
