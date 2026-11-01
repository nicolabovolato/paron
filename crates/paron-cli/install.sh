#!/bin/sh
# Installs the newest release of Paron.
#
#   curl -fsSL https://paron.bovolato.dev/install.sh | sh
#
# PARON_VERSION       install this version, for example 0.1.0
# PARON_INSTALL_DIR   install into this directory, the default is /usr/local/bin

set -eu

repository="nicolabovolato/paron"
directory="${PARON_INSTALL_DIR:-/usr/local/bin}"

fail() {
  printf 'error: %s\n' "$1" >&2
  exit 1
}

case "$(uname -s)" in
  Linux) system="unknown-linux-musl" ;;
  Darwin) system="apple-darwin" ;;
  *) fail "Paron has no release for $(uname -s)" ;;
esac

case "$(uname -m)" in
  x86_64 | amd64) processor="x86_64" ;;
  aarch64 | arm64) processor="aarch64" ;;
  *) fail "Paron has no release for $(uname -m)" ;;
esac

if command -v curl >/dev/null 2>&1; then
  print_url() { curl -fsSL "$1"; }
  save_url() { curl -fsSL -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
  print_url() { wget -qO- "$1"; }
  save_url() { wget -qO "$2" "$1"; }
else
  fail "curl or wget is necessary"
fi

if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d ' ' -f 1; }
elif command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
else
  fail "sha256sum or shasum is necessary"
fi

releases=$(print_url "https://api.github.com/repos/$repository/releases?per_page=100") ||
  fail "could not get the list of releases from GitHub"

version="${PARON_VERSION:-}"
if [ -z "$version" ]; then
  version=$(printf '%s\n' "$releases" |
    sed -n 's/^ *"tag_name": "paron-cli-v\(.*\)",$/\1/p' |
    head -n 1)
  [ -n "$version" ] || fail "GitHub lists no release of Paron"
fi

archive="paron-$version-$processor-$system.tar.gz"

# GitHub records a SHA-256 digest for each release file.
expected=$(printf '%s\n' "$releases" |
  awk -v name="\"name\": \"$archive\"," '
    index($0, name) { found = 1 }
    found && /"digest": "sha256:/ {
      sub(/.*sha256:/, "")
      sub(/".*/, "")
      print
      exit
    }')
[ -n "$expected" ] || fail "GitHub lists no file $archive"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

printf 'downloading %s\n' "$archive"
save_url "https://github.com/$repository/releases/download/paron-cli-v$version/$archive" "$work/$archive" ||
  fail "could not download $archive"

actual=$(sha256 "$work/$archive")
[ "$actual" = "$expected" ] ||
  fail "the SHA-256 of $archive is $actual, and GitHub records $expected"

tar -xzf "$work/$archive" -C "$work"

if mkdir -p "$directory" 2>/dev/null && [ -w "$directory" ]; then
  install -m 755 "$work/paron" "$directory/paron"
else
  if command -v sudo >/dev/null 2>&1; then
    as_root="sudo"
  elif command -v doas >/dev/null 2>&1; then
    as_root="doas"
  else
    fail "cannot write to $directory. Set PARON_INSTALL_DIR to a directory that you can write to"
  fi

  printf '%s needs root access. These commands run as root:\n' "$directory" >&2
  set -x
  "$as_root" mkdir -p "$directory"
  "$as_root" install -m 755 "$work/paron" "$directory/paron"
  { set +x; } 2>/dev/null
fi

printf 'installed %s\n' "$("$directory/paron" --version)"
printf 'at %s\n' "$directory/paron"

case ":$PATH:" in
  *":$directory:"*) ;;
  *) printf 'note: %s is not in your PATH\n' "$directory" ;;
esac
