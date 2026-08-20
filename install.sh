#!/bin/bash
set -euo pipefail

REPO="${CONFCTL_REPOSITORY:-tarcisiomiranda/confctl}"
INSTALL_DIR="${CONFCTL_INSTALL_DIR:-/usr/local/bin}"
BINARY_NAME="confctl"
# Off by default. Set to 1/true/yes/on to detect AI agents and install SKILL.md.
INSTALL_SKILLS="${CONFCTL_INSTALL_SKILLS:-0}"

fail() {
	printf 'confctl installer: %s\n' "$*" >&2
	exit 1
}

command_exists() {
	command -v "$1" >/dev/null 2>&1
}

# Returns 0 when value is a common truthy flag (1, true, yes, on).
env_truthy() {
	value=$(printf '%s' "${1:-}" | tr '[:upper:]' '[:lower:]')
	case "$value" in
		1 | true | yes | on) return 0 ;;
		*) return 1 ;;
	esac
}

# Copy skill into dest_dir/SKILL.md when the agent appears installed.
install_skill_for_agent() {
	skill_src=$1
	dest_rel=$2
	shift 2
	# Remaining args: path markers relative to $HOME and/or binary names (bin:name).
	found=0
	for marker in "$@"; do
		case "$marker" in
			bin:*)
				if command_exists "${marker#bin:}"; then
					found=1
					break
				fi
				;;
			*)
				if [ -e "${HOME}/${marker}" ]; then
					found=1
					break
				fi
				;;
		esac
	done
	if [ "$found" -eq 0 ]; then
		return 0
	fi
	dest_dir="${HOME}/${dest_rel}"
	mkdir -p "$dest_dir"
	install -m 0644 "$skill_src" "${dest_dir}/SKILL.md"
	printf '  skill → %s\n' "${dest_dir}/SKILL.md"
}

install_agent_skills() {
	if [ -z "${HOME:-}" ]; then
		printf 'Skipping agent skills: HOME is not set.\n'
		return 0
	fi

	if [ "$VERSION" = latest ]; then
		skill_ref=main
	else
		skill_ref=$VERSION
	fi
	skill_url="https://raw.githubusercontent.com/${REPO}/${skill_ref}/skills/confctl/SKILL.md"
	skill_path="${TMP_DIR}/SKILL.md"

	printf 'Installing AI agent skills (CONFCTL_INSTALL_SKILLS is enabled)...\n'
	if ! curl --proto '=https' --tlsv1.2 -fsSL "$skill_url" -o "$skill_path"; then
		printf 'warning: could not download skill from %s\n' "$skill_url" >&2
		return 0
	fi
	if [ ! -s "$skill_path" ]; then
		printf 'warning: downloaded skill file is empty; skipping\n' >&2
		return 0
	fi

	# Detect agents via config dirs and PATH binaries; install only when found.
	install_skill_for_agent "$skill_path" ".claude/skills/confctl" \
		".claude" "bin:claude"
	install_skill_for_agent "$skill_path" ".codex/skills/confctl" \
		".codex" "bin:codex"
	install_skill_for_agent "$skill_path" ".config/opencode/skills/confctl" \
		".config/opencode" ".opencode" ".local/share/opencode" "bin:opencode"
	install_skill_for_agent "$skill_path" ".cursor/skills/confctl" \
		".cursor" "bin:cursor" "bin:cursor-agent"
	install_skill_for_agent "$skill_path" ".grok/skills/confctl" \
		".grok" "bin:grok"
	install_skill_for_agent "$skill_path" ".agents/skills/confctl" \
		".agents"
	install_skill_for_agent "$skill_path" ".gemini/skills/confctl" \
		".gemini" "bin:gemini"
	install_skill_for_agent "$skill_path" ".continue/skills/confctl" \
		".continue"
	install_skill_for_agent "$skill_path" ".codeium/windsurf/skills/confctl" \
		".codeium/windsurf" ".windsurf" "bin:windsurf"
	install_skill_for_agent "$skill_path" ".kiro/skills/confctl" \
		".kiro" "bin:kiro"
	install_skill_for_agent "$skill_path" ".hermes/skills/confctl" \
		".hermes" "bin:hermes"

	printf 'Agent skill install finished.\n'
}

command_exists curl || fail "curl is required"
command_exists install || fail "install is required"

OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$OS" in
	linux)
		case "$ARCH" in
			x86_64|amd64) BINARY="confctl-linux-amd64" ;;
			*) fail "unsupported Linux architecture: $ARCH" ;;
		esac
		;;
	darwin)
		case "$ARCH" in
			arm64|aarch64) BINARY="confctl-darwin-arm64" ;;
			x86_64|amd64) BINARY="confctl-darwin-amd64" ;;
			*) fail "unsupported macOS architecture: $ARCH" ;;
		esac
		;;
	*) fail "unsupported OS: $OS" ;;
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

TMP_DIR=$(mktemp -d 2>/dev/null || mktemp -d -t confctl-install)
trap 'rm -rf "$TMP_DIR"' EXIT
TMP_FILE="${TMP_DIR}/${BINARY}"

echo "Downloading from: $URL"
curl -fsSL "$URL" -o "$TMP_FILE"
chmod +x "$TMP_FILE"

if mkdir -p "$INSTALL_DIR" 2>/dev/null && [ -w "$INSTALL_DIR" ]; then
	install -m 0755 "$TMP_FILE" "$INSTALL_DIR/$BINARY_NAME"
elif command_exists sudo; then
	echo "Installing to $INSTALL_DIR (requires sudo)..."
	sudo mkdir -p "$INSTALL_DIR"
	sudo install -m 0755 "$TMP_FILE" "$INSTALL_DIR/$BINARY_NAME"
else
	# Fall back to ~/.local/bin when default dir is not writable.
	INSTALL_DIR="${HOME}/.local/bin"
	mkdir -p "$INSTALL_DIR"
	install -m 0755 "$TMP_FILE" "$INSTALL_DIR/$BINARY_NAME"
fi

echo ""
echo "✓ confctl installed successfully!"
echo "  Location: $INSTALL_DIR/$BINARY_NAME"
echo "  Version: $($INSTALL_DIR/$BINARY_NAME --version 2>/dev/null || echo 'installed')"

case ":${PATH}:" in
	*":${INSTALL_DIR}:"*) ;;
	*) printf 'Add %s to PATH before running confctl.\n' "$INSTALL_DIR" ;;
esac

if env_truthy "$INSTALL_SKILLS"; then
	install_agent_skills
else
	printf 'AI agent skills were not installed (default). Enable with CONFCTL_INSTALL_SKILLS=1.\n'
fi
