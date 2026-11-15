#!/usr/bin/env bash
# Check the host's install script as root, as a new cloud server's first boot runs it: in a
# throwaway Debian container, against a stand-in release in a folder. What it must get right is
# the new user's way in: which SSH keys it is given, and that a run which failed part-way does
# not cost it them. crates/host/tests/install_script.rs covers the script run as an ordinary user.
#
# usage: scripts/host-install-check.sh
# Used by CI's "Host package" job. Needs docker.
set -euo pipefail
cd "$(dirname "$0")/.."

# Debian 12 with curl and nothing more: the oldest glibc the host runs on.
image="buildpack-deps:bookworm-curl"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
case "$(uname -m)" in
  x86_64 | amd64) arch=x86_64 ;;
  aarch64 | arm64) arch=aarch64 ;;
esac
name="yardsort-host-9.9.9-linux-$arch"
mkdir -p "$work/stage/$name/bin" "$work/stage/$name/systemd" "$work/releases/download/v9.9.9"
for program in yardsort-host ys; do
  printf '#!/bin/sh\necho "%s 9.9.9"\n' "$program" > "$work/stage/$name/bin/$program"
  chmod 755 "$work/stage/$name/bin/$program"
done
cp crates/host/packaging/yardsort-host.service "$work/stage/$name/systemd/"
tar -czf "$work/releases/download/v9.9.9/$name.tar.gz" -C "$work/stage" "$name"
chmod -R a+rX "$work"

# Inside the container, as root. Each case is a user of its own.
cat > "$work/cases.sh" <<'CASES'
set -eu
export YARDSORT_HOST_RELEASES=file:///releases
install_for() { sh /install.sh --user "$@"; }
fail() { echo "FAILED: $*" >&2; exit 1; }
keys_of() { cat "/home/$1/.ssh/authorized_keys" 2>/dev/null || true; }

plain='ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIPlainPlainPlainPlainPlainPlainPlainPlainPlain you@laptop'
other='ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOtherOtherOtherOtherOtherOtherOtherOtherOther admin@laptop'
# What cloud-init leaves on root where root logins are off, which is its default.
denied='no-port-forwarding,no-agent-forwarding,no-X11-forwarding,command="echo '"'"'Please login as the user \"admin\" rather than the user \"root\".'"'"';echo;sleep 10;exit 142" '"$plain"
mkdir -p /root/.ssh && chmod 700 /root/.ssh

echo "--- a download that fails makes no user, and the next run gives the new user its keys"
printf '%s\n' "$plain" > /root/.ssh/authorized_keys
if install_for retry --version 9.9.8 2>/dev/null; then fail "a release that is not there installed"; fi
if id retry >/dev/null 2>&1; then fail "a failed download left the user behind"; fi
install_for retry --version 9.9.9
[ "$(keys_of retry)" = "$plain" ] || fail "the retried user has no keys: $(keys_of retry)"
[ "$(stat -c '%U %a' /home/retry/.ssh/authorized_keys)" = "retry 600" ] || fail "keys' owner or mode"
[ "$(stat -c '%U %a' /home/retry/.ssh)" = "retry 700" ] || fail ".ssh's owner or mode"
[ "$(runuser -l retry -c 'yardsort-host')" = "yardsort-host 9.9.9" ] || fail "the user cannot run it"

echo "--- cloud-init's root-login denial is not handed on"
printf '%s\n' "$denied" > /root/.ssh/authorized_keys
out="$(install_for denied --version 9.9.9)"
[ ! -e /home/denied/.ssh/authorized_keys ] || fail "the denial was copied: $(keys_of denied)"
printf '%s\n' "$out" | grep -q "denied has no SSH key yet" || fail "it did not say there is no key: $out"

echo "--- of root's keys, only those with no options are given"
printf '%s\n' "# a comment" "$denied" "$plain" 'from="10.0.0.1" '"$other" > /root/.ssh/authorized_keys
install_for mixed --version 9.9.9 >/dev/null
[ "$(keys_of mixed)" = "$plain" ] || fail "expected the plain key alone: $(keys_of mixed)"

echo "--- under sudo, the keys are those of the user who called it"
useradd --create-home admin
install -d -m 700 -o admin -g admin /home/admin/.ssh
printf '%s\n' "$other" > /home/admin/.ssh/authorized_keys
printf '%s\n' "$denied" > /root/.ssh/authorized_keys
SUDO_USER=admin install_for viasudo --version 9.9.9 >/dev/null
[ "$(keys_of viasudo)" = "$other" ] || fail "expected admin's key: $(keys_of viasudo)"

echo "--- a user who was already there keeps what it has"
useradd --create-home already
printf '%s\n' "$plain" > /root/.ssh/authorized_keys
install_for already --version 9.9.9 >/dev/null
[ ! -e /home/already/.ssh ] || fail "an existing user was given keys"
[ -x /home/already/.local/bin/ys ] || fail "nothing was installed for the existing user"

echo "all passed"
CASES

docker run --rm \
  -v "$PWD/crates/host/packaging/install.sh:/install.sh:ro" \
  -v "$work/releases:/releases:ro" \
  -v "$work/cases.sh:/cases.sh:ro" \
  "$image" sh /cases.sh
