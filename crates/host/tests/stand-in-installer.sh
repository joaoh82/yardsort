#!/bin/bash
# A stand-in for a vendor's installer, for `yardsort-host install-agent <harness> --installer`: it
# does what Anthropic's, Cursor's and xAI's do, put a program in ~/.local/bin, and nothing else.
# Used by crates/host/tests/install_agent.rs and by CI's and the smoke test's image checks.
set -eu
program="${STAND_IN_PROGRAM:-claude}"
mkdir -p "$HOME/.local/bin"
printf '%s\n' '#!/bin/sh' "echo \"$program 0.0.0 (stand-in)\"" > "$HOME/.local/bin/$program"
chmod 755 "$HOME/.local/bin/$program"
echo "stand-in installer: $program is in $HOME/.local/bin"
