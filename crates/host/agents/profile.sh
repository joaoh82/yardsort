# The full yardsort-host image's tools, for login shells: Debian's /etc/profile sets PATH afresh,
# and agents are started from the login shell's environment. ~/.local/bin comes first, so that a
# harness `yardsort-host install-agent` installed or updated is the one found.
PATH="$HOME/.local/bin:/usr/local/cargo/bin:/usr/local/go/bin:$HOME/go/bin:$PATH"
export PATH
