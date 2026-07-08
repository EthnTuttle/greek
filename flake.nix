{
  description = "Greek learning app — Rust core + Android (Jetpack Compose) + Tauri desktop";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
          targets = [
            "aarch64-linux-android"
            "armv7-linux-androideabi"
            "x86_64-linux-android"
            "i686-linux-android"
          ];
        };
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            rustToolchain
            cargo-ndk

            # Tauri desktop deps
            pkg-config
            openssl
            webkitgtk_4_1
            gtk3
            libsoup_3
            glib
            cairo
            pango
            gdk-pixbuf
            atk

            # Build tools
            cmake
            protobuf
          ];

          ANDROID_HOME = "/home/ethan/Android/Sdk";
          ANDROID_NDK_HOME = "/home/ethan/Android/Sdk/ndk/27.0.12077973";
          JAVA_HOME = "${pkgs.jdk17}";

          shellHook = ''
            export PATH="/home/ethan/Android/Sdk/platform-tools:$PATH"
            echo "Greek App dev shell ready"
            echo "  Rust: $(rustc --version)"
            echo "  Android SDK: $ANDROID_HOME"
            echo "  NDK: $ANDROID_NDK_HOME"
            echo "  Java: $(java -version 2>&1 | head -1)"
          '';
        };
      });
}
