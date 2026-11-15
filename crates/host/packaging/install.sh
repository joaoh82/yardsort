#!/bin/sh
# Installs yardsort-host and ys from a release's tarball, for one user, or brings them up to date.
# See docs/guide/headless-host.md.
#
#   curl -fsSL https://raw.githubusercontent.com/joaoh82/yardsort/main/crates/host/packaging/install.sh | sh
#
# As the user that will run the host, it puts the two programs in ~/.local/bin and the systemd user
# service in ~/.config/systemd/user, and keeps that user's services running after logout where it
# can. As root, which is how a new cloud server's first boot runs things, name the user:
#
#   curl -fsSL …/install.sh | sh -s -- --user yardsort
#
# makes that user if there is none, installs git if apt can, and installs the same things for that
# user. A user it made gets the SSH keys of whoever ran it: yours under sudo, else root's, and only
# keys with no options before them, so never cloud-init's "log in as another user" line. The host
# itself never runs as root.
#
#   --version X.Y.Z   that release rather than the newest
#   --user NAME       as root only: the user to install for
#
# It starts nothing and signs in to nothing: it prints what is left, each a sign-in of yours.
# YARDSORT_HOST_RELEASES names another place to take releases from, laid out as GitHub's are
# (<it>/download/v<version>/<tarball>); the tests use it.
set -eu

releases="${YARDSORT_HOST_RELEASES:-https://github.com/joaoh82/yardsort/releases}"
guide="https://yardsort.sh/docs/guide/headless-host"

say() { printf '%s\n' "$*"; }
die() {
  printf '%s\n' "$*" >&2
  exit 1
}

usage() {
  die "usage: install.sh [--version X.Y.Z] [--user NAME]"
}

# Runs a command as the user the host is installed for.
as_user() {
  if [ "$as_root" = 1 ]; then
    runuser -u "$user" -- "$@"
  else
    "$@"
  fi
}

# Whether that user's login shell finds the programs, which is how agents find `ys`.
login_shell_finds_them() {
  if [ "$as_root" = 1 ]; then
    runuser -l "$user" -c 'command -v yardsort-host' </dev/null >/dev/null 2>&1
  else
    "${SHELL:-/bin/sh}" -lc 'command -v yardsort-host' </dev/null >/dev/null 2>&1
  fi
}

lingers() {
  [ "$(loginctl show-user "$user" --property=Linger --value 2>/dev/null)" = yes ]
}

main() {
  version=""
  user=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --version)
        [ $# -ge 2 ] || usage
        version="${2#v}"
        shift 2
        ;;
      --user)
        [ $# -ge 2 ] || usage
        user="$2"
        shift 2
        ;;
      *) usage ;;
    esac
  done

  [ "$(uname -s)" = Linux ] || die "yardsort-host runs on Linux. See $guide."
  case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) die "There is no yardsort-host for $(uname -m): x86_64 and aarch64 only." ;;
  esac
  # The tarballs are built on Ubuntu 22.04, and need its glibc or a newer one.
  libc="$(getconf GNU_LIBC_VERSION 2>/dev/null | sed -n 's/^glibc //p')"
  case "$libc" in
    [0-9]*.[0-9]*)
      if [ "${libc%%.*}" -lt 2 ] || { [ "${libc%%.*}" -eq 2 ] && [ "${libc#*.}" -lt 35 ]; }; then
        die "yardsort-host needs glibc 2.35 or later (Ubuntu 22.04, Debian 12); this computer has $libc."
      fi
      ;;
    *) die "yardsort-host needs glibc 2.35 or later (Ubuntu 22.04, Debian 12), and found none here." ;;
  esac
  for tool in curl tar gzip install; do
    command -v "$tool" >/dev/null 2>&1 || die "Needs $tool."
  done

  made_user=0
  if [ "$(id -u)" = 0 ]; then
    as_root=1
    [ -n "$user" ] || die "The host never runs as root. Run this as the user that will run it, or as root with --user NAME."
    [ "$user" != root ] || die "The host never runs as root: name an ordinary user."
    command -v runuser >/dev/null 2>&1 || die "Needs runuser."
    id "$user" >/dev/null 2>&1 || command -v useradd >/dev/null 2>&1 || die "Needs useradd, to make $user."
  else
    as_root=0
    [ -z "$user" ] || [ "$user" = "$(id -un)" ] || die "--user is for root. Run this as $user instead."
    user="$(id -un)"
  fi
  if [ -z "$version" ]; then
    # The newest release's page redirects to its tag: no API, so no rate limit.
    latest="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$releases/latest")" ||
      die "Could not reach $releases/latest."
    version="${latest##*/tag/v}"
    case "$version" in
      [0-9]*.[0-9]*.[0-9]*) ;;
      *) die "Could not tell the newest release from $latest. Name one with --version." ;;
    esac
  fi

  name="yardsort-host-$version-linux-$arch"
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  say "Downloading $name.tar.gz"
  curl -fsSL -o "$tmp/$name.tar.gz" "$releases/download/v$version/$name.tar.gz" ||
    die "Could not download $releases/download/v$version/$name.tar.gz. The host's tarballs start at 0.23.0."
  tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
  from="$tmp/$name"
  [ -x "$from/bin/yardsort-host" ] && [ -x "$from/bin/ys" ] || die "$name.tar.gz is not a host's tarball."
  "$from/bin/yardsort-host" --version >/dev/null || die "This computer cannot run $name."
  # The user reads it from here; nothing in it is secret.
  chmod -R a+rX "$tmp"

  # Only now, with a release in hand, is anything made: a download that fails leaves no user
  # behind, for the next run to find already there and give no keys to.
  no_keys=0
  if [ "$as_root" = 1 ]; then
    if ! id "$user" >/dev/null 2>&1; then
      useradd --create-home --shell /bin/bash "$user"
      made_user=1
      say "Made the user $user."
    fi
    home="$(getent passwd "$user" | cut -d: -f6)"
  else
    home="$HOME"
  fi
  [ -n "$home" ] && [ -d "$home" ] || die "$user has no home folder."

  if [ "$as_root" = 1 ]; then
    if [ "$made_user" = 1 ]; then
      # A new user has no way in. Give it the keys of whoever is running this: the user who
      # called sudo, else root. Only keys with nothing before them are taken. A key with options
      # is restricted, and on cloud-init images root's is a forced command that says to log in
      # as another user and hangs up: copied, it would lock the new user out the same way.
      giver="${SUDO_USER:-root}"
      giver_home="$(getent passwd "$giver" | cut -d: -f6)"
      if [ -n "$giver_home" ] && [ -r "$giver_home/.ssh/authorized_keys" ]; then
        grep -E '^[[:space:]]*(ssh-|ecdsa-|sk-)[^[:space:]]+[[:space:]]+[A-Za-z0-9+/]' \
          "$giver_home/.ssh/authorized_keys" >"$tmp/keys" 2>/dev/null || true
      fi
      if [ -s "$tmp/keys" ]; then
        install -d -m 700 -o "$user" -g "$(id -gn "$user")" "$home/.ssh"
        install -m 600 -o "$user" -g "$(id -gn "$user")" "$tmp/keys" "$home/.ssh/authorized_keys"
        say "Gave $user $giver's SSH keys, so whoever logs in as $giver can log in as $user."
      else
        no_keys=1
      fi
    fi
    if ! command -v git >/dev/null 2>&1 && command -v apt-get >/dev/null 2>&1; then
      say "Installing git"
      apt-get update -qq </dev/null >/dev/null
      DEBIAN_FRONTEND=noninteractive apt-get install -y -qq git </dev/null >/dev/null
    fi
  fi

  bin="$home/.local/bin"
  unit="$home/.config/systemd/user/yardsort-host.service"
  upgrade=0
  [ ! -e "$bin/yardsort-host" ] || upgrade=1
  # `install` replaces a file rather than writing into it, so a running `serve` is left alone.
  as_user install -Dm755 "$from/bin/yardsort-host" "$from/bin/ys" -t "$bin/"
  kept_unit=0
  if [ -e "$unit" ] && ! cmp -s "$unit" "$from/systemd/yardsort-host.service"; then
    kept_unit=1
  else
    as_user install -Dm644 "$from/systemd/yardsort-host.service" -t "${unit%/*}/"
  fi

  systemd=0
  [ ! -d /run/systemd/system ] || systemd=1
  if [ "$systemd" = 1 ] && ! lingers; then
    # Without it the host stops when its user's last login ends.
    if [ "$as_root" = 1 ]; then
      loginctl enable-linger "$user" || true
    else
      loginctl enable-linger "$user" </dev/null >/dev/null 2>&1 ||
        sudo -n loginctl enable-linger "$user" </dev/null >/dev/null 2>&1 || true
    fi
  fi
  if [ "$systemd" = 1 ] && [ "$as_root" = 0 ]; then
    systemctl --user daemon-reload </dev/null >/dev/null 2>&1 || true
  fi

  say ""
  say "yardsort-host $version and ys are in $bin."
  [ "$kept_unit" = 0 ] || say "Kept $unit, which is not the release's."
  command -v git >/dev/null 2>&1 || say "There is no git here yet: install it before adding a project."
  if ! login_shell_finds_them; then
    say ""
    say "$bin is not on $user's PATH in its login shell, where agents look for ys. Add this to"
    say "$home/.profile, then log in again:"
    say "  export PATH=\"\$HOME/.local/bin:\$PATH\""
  elif [ "$as_root" = 0 ] && ! command -v yardsort-host >/dev/null 2>&1; then
    say "Log out and in again, so that this shell finds them."
  fi

  say ""
  if [ "$upgrade" = 1 ]; then
    if [ "$systemd" = 1 ]; then
      if [ "$as_root" = 1 ]; then
        say "To run the new one, as $user: systemctl --user restart yardsort-host"
      else
        say "To run the new one: systemctl --user restart yardsort-host"
      fi
    else
      say "To run the new one, stop yardsort-host serve and start it again."
    fi
    say "The agents keep running."
    return
  fi
  if [ "$no_keys" = 1 ]; then
    say "$user has no SSH key yet: $giver has none it could be given. Before logging in as $user,"
    say "put your public key in $home/.ssh/authorized_keys (folder 700, file 600, both $user's)."
    say ""
  fi
  if [ "$as_root" = 1 ]; then
    say "What is left, logged in as $user (over SSH, not su or sudo), each a sign-in of yours:"
  else
    say "What is left, each a sign-in of yours:"
  fi
  say "  yardsort-host sign-in <your server's address>"
  if [ "$systemd" = 1 ]; then
    say "  systemctl --user enable --now yardsort-host"
    lingers || say "  sudo loginctl enable-linger $user     # or it stops when you log out"
  else
    say "  yardsort-host serve                   # under tmux: there is no systemd here"
  fi
  say "  yardsort-host install-agent claude    # or cursor, grok, codex, opencode, pi, omp; then sign it in"
  say "  yardsort-host pair                    # for your phone"
  say "Git and GitHub: $guide#git-and-github"
}

main "$@"
