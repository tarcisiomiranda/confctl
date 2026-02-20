#!/bin/bash
set -e

REPO="tarcisiomiranda/confctl"
INSTALL_DIR="/usr/local/bin"
BINARY_NAME="confctl"

OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$OS" in
    linux)
        case "$ARCH" in
            x86_64|amd64)
                BINARY="confctl-linux-amd64"
                ;;
            *)
                echo "Error: Unsupported Linux architecture: $ARCH"
                exit 1
                ;;
        esac
        ;;
    darwin)
        case "$ARCH" in
            arm64|aarch64)
                BINARY="confctl-darwin-arm64"
                ;;
            x86_64|amd64)
                BINARY="confctl-darwin-amd64"
                ;;
            *)
                echo "Error: Unsupported macOS architecture: $ARCH"
                exit 1
                ;;
        esac
        ;;
    *)
        echo "Error: Unsupported OS: $OS"
        exit 1
        ;;
esac

VERSION="${1:-latest}"

if [ "$VERSION" = "latest" ]; then
    URL="https://github.com/$REPO/releases/latest/download/$BINARY"
else
    URL="https://github.com/$REPO/releases/download/$VERSION/$BINARY"
fi

echo "Installing confctl..."
echo "  OS: $OS"
echo "  Arch: $ARCH"
echo "  Binary: $BINARY"
echo "  Version: $VERSION"
echo ""

TMP_FILE=$(mktemp)
trap "rm -f $TMP_FILE" EXIT

echo "Downloading from: $URL"
if command -v curl &> /dev/null; then
    curl -fsSL "$URL" -o "$TMP_FILE"
elif command -v wget &> /dev/null; then
    wget -q "$URL" -O "$TMP_FILE"
else
    echo "Error: curl or wget is required"
    exit 1
fi

chmod +x "$TMP_FILE"

if [ -w "$INSTALL_DIR" ]; then
    mv "$TMP_FILE" "$INSTALL_DIR/$BINARY_NAME"
else
    echo "Installing to $INSTALL_DIR (requires sudo)..."
    sudo mv "$TMP_FILE" "$INSTALL_DIR/$BINARY_NAME"
fi

echo ""
echo "✓ confctl installed successfully!"
echo "  Location: $INSTALL_DIR/$BINARY_NAME"
echo "  Version: $(confctl --version 2>/dev/null || echo 'installed')"
