#!/usr/bin/env bash
# kto installer script
# Usage: curl -fsSL https://raw.githubusercontent.com/devindudeman/kto/main/install.sh | bash
#    or: curl -fsSL https://raw.githubusercontent.com/devindudeman/kto/main/install.sh | bash -s -- --to /custom/path

set -euo pipefail

REPO="devindudeman/kto"
INSTALL_DIR="${HOME}/.local/bin"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --to)
            INSTALL_DIR="$2"
            shift 2
            ;;
        --help|-h)
            echo "Usage: install.sh [--to /custom/path]"
            echo ""
            echo "Options:"
            echo "  --to PATH    Install to custom directory (default: ~/.local/bin)"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Detect OS and architecture
detect_platform() {
    local os arch

    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)
            os="unknown-linux-gnu"
            ;;
        Darwin)
            os="apple-darwin"
            ;;
        *)
            echo "Error: Unsupported operating system: $os"
            exit 1
            ;;
    esac

    case "$arch" in
        x86_64|amd64)
            arch="x86_64"
            ;;
        arm64|aarch64)
            arch="aarch64"
            ;;
        *)
            echo "Error: Unsupported architecture: $arch"
            exit 1
            ;;
    esac

    echo "${arch}-${os}"
}

# Get latest release version
get_latest_version() {
    curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/'
}

# Download and verify
download_and_install() {
    local platform="$1"
    local version="$2"
    local url="https://github.com/${REPO}/releases/download/${version}/kto-${platform}.tar.gz"
    local checksums_url="https://github.com/${REPO}/releases/download/${version}/checksums.txt"
    local tmpdir

    tmpdir="$(mktemp -d)"
    # Clean up on exit, but handle the case where tmpdir might be unset
    trap 'rm -rf "${tmpdir:-}"' EXIT

    echo "Downloading kto ${version} for ${platform}..."
    curl -fsSL "$url" -o "${tmpdir}/kto.tar.gz"

    echo "Verifying checksum..."
    curl -fsSL "$checksums_url" -o "${tmpdir}/checksums.txt"

    local expected_checksum
    expected_checksum="$(grep "kto-${platform}.tar.gz" "${tmpdir}/checksums.txt" | awk '{print $1}')"

    local actual_checksum
    if command -v sha256sum &> /dev/null; then
        actual_checksum="$(sha256sum "${tmpdir}/kto.tar.gz" | awk '{print $1}')"
    elif command -v shasum &> /dev/null; then
        actual_checksum="$(shasum -a 256 "${tmpdir}/kto.tar.gz" | awk '{print $1}')"
    else
        echo "Error: Cannot verify checksum - neither sha256sum nor shasum found"
        echo "Install one of these tools or use 'cargo install kto' instead"
        exit 1
    fi

    if [[ "$expected_checksum" != "$actual_checksum" ]]; then
        echo "Error: Checksum verification failed!"
        echo "Expected: $expected_checksum"
        echo "Got: $actual_checksum"
        exit 1
    fi

    echo "Extracting..."
    # Extract only the kto binary, prevent path traversal
    tar xzf "${tmpdir}/kto.tar.gz" -C "${tmpdir}" --no-same-owner kto

    echo "Installing to ${INSTALL_DIR}..."
    mkdir -p "${INSTALL_DIR}"
    # Verify extracted file exists and is a regular file
    if [[ ! -f "${tmpdir}/kto" ]]; then
        echo "Error: Expected binary 'kto' not found in archive"
        exit 1
    fi
    mv "${tmpdir}/kto" "${INSTALL_DIR}/kto"
    chmod +x "${INSTALL_DIR}/kto"

    echo ""
    echo "kto ${version} installed successfully!"
    echo ""

    # Check if install dir is in PATH
    if [[ ":$PATH:" != *":${INSTALL_DIR}:"* ]]; then
        echo "Note: ${INSTALL_DIR} is not in your PATH."
        echo ""
        echo "Add it to your shell config:"
        echo ""
        echo "  # For bash (~/.bashrc):"
        echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
        echo ""
        echo "  # For zsh (~/.zshrc):"
        echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
        echo ""
        echo "  # For fish (~/.config/fish/config.fish):"
        echo "  set -gx PATH \$HOME/.local/bin \$PATH"
        echo ""
    fi

    echo "Get started with:"
    echo "  kto init        # First-time setup"
    echo "  kto doctor      # Check dependencies"
    echo "  kto --help      # Show all commands"
}

main() {
    echo "kto installer"
    echo ""

    local platform version

    platform="$(detect_platform)"
    echo "Detected platform: ${platform}"

    version="$(get_latest_version)"
    if [[ -z "$version" ]]; then
        echo "Error: Could not determine latest version"
        exit 1
    fi
    echo "Latest version: ${version}"
    echo ""

    download_and_install "$platform" "$version"
}

main
