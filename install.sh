#!/bin/sh
# Voussoir installer for Linux, macOS, and the BSDs.
#
# Installs the toolchain dependencies (system webview, Git, Bun, Rust), builds
# the CLI from source, and puts `voussoir` on PATH. Run it from a checkout, or
# pipe it to sh from anywhere:
#
#   sh install.sh [--prefix DIR] [--repo URL] [--no-deps]
#
# Every dependency step is skipped when the tool is already present, so
# re-running on a provisioned machine touches nothing but the build.

set -eu

PREFIX="${VOUSSOIR_PREFIX:-$HOME/.local}"
REPO="${VOUSSOIR_REPO:-https://github.com/smegasberla/voussoir.git}"
WITH_DEPS=1

log() { printf 'voussoir-install: %s\n' "$*"; }
warn() { printf 'voussoir-install: warning: %s\n' "$*" >&2; }
die() { printf 'voussoir-install: error: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

usage() {
  cat <<'EOF'
usage: install.sh [--prefix DIR] [--repo URL] [--no-deps] [--help]

  --prefix DIR   install location (default: $HOME/.local; the binary lands in
                 DIR/bin/voussoir)
  --repo URL     git URL to clone when not run from a checkout
  --no-deps      skip system package installation
  --help         this text

  Environment: VOUSSOIR_PREFIX, VOUSSOIR_REPO do the same as the flags.
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --prefix)
      [ $# -ge 2 ] || die "--prefix needs a value"
      PREFIX=$2; shift 2 ;;
    --repo)
      [ $# -ge 2 ] || die "--repo needs a value"
      REPO=$2; shift 2 ;;
    --no-deps) WITH_DEPS=0; shift ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; die "unknown argument: $1" ;;
  esac
done

# Installers put tools here; pick them up for the rest of this run.
PATH="$HOME/.cargo/bin:$HOME/.bun/bin:$PATH"
export PATH

OS=$(uname -s)
PM=""
SUDO=""
ROOT=0
[ "$(id -u)" -eq 0 ] && ROOT=1
if [ "$ROOT" -eq 0 ] && have sudo; then SUDO="sudo"; fi

detect_pm() {
  case "$OS" in
    Linux)
      for m in apt-get dnf pacman zypper apk; do
        if have "$m"; then PM=$m; return 0; fi
      done ;;
    FreeBSD|DragonFly)
      if have pkg; then PM=pkg; return 0; fi ;;
    NetBSD)
      if have pkgin; then PM=pkgin; return 0; fi
      if have pkg_add; then PM=pkg_add; return 0; fi ;;
    OpenBSD)
      if have pkg_add; then PM=pkg_add; return 0; fi ;;
  esac
  return 0
}

# Attempts the package install and returns its real status; call sites decide
# what a failure means (warn-and-continue, or the next name in a fallback
# chain). Never aborts on its own, even under set -e.
try_pkgs() {
  if [ -z "$PM" ]; then
    log "no package manager known for $OS"
    return 1
  fi
  if [ "$ROOT" -eq 0 ] && [ -z "$SUDO" ]; then
    log "no root access, not attempting: $*"
    return 1
  fi
  log "installing system packages: $*"
  case "$PM" in
    apt-get)
      $SUDO apt-get update -qq || true
      $SUDO apt-get install -y --no-install-recommends "$@" ;;
    dnf) $SUDO dnf install -y "$@" ;;
    pacman) $SUDO pacman -S --noconfirm --needed "$@" ;;
    zypper) $SUDO zypper --non-interactive install "$@" ;;
    apk) $SUDO apk add "$@" ;;
    pkg)
      if [ "$OS" = "FreeBSD" ] || [ "$OS" = "DragonFly" ]; then
        $SUDO pkg update -f || true
      fi
      $SUDO pkg install -y "$@" ;;
    pkgin) $SUDO pkgin -y install "$@" ;;
    pkg_add) $SUDO pkg_add -I "$@" ;;
    *) return 1 ;;
  esac
}

gtk_platform() {
  case "$OS" in
    Linux|FreeBSD|NetBSD|OpenBSD|DragonFly) return 0 ;;
    *) return 1 ;;
  esac
}

webkit_present() {
  have pkg-config && pkg-config --exists webkit2gtk-4.1 2>/dev/null
}

# Called bare (set -e active): every fallible statement is guarded.
install_deps() {
  case "$OS" in
    Darwin)
      if ! xcode-select -p >/dev/null 2>&1; then
        log "Xcode Command Line Tools missing; triggering installer, re-run when it finishes"
        xcode-select --install || true
        die "install the Command Line Tools, then re-run this script"
      fi
      if ! have git || ! have curl; then
        if have brew; then brew install git curl || warn "brew git/curl install failed"; else
          warn "git/curl missing and Homebrew not found; install them first"
        fi
      fi
      ;;
    Linux)
      detect_pm
      if [ -n "$PM" ] && webkit_present && have git && have curl && have pkg-config; then
        log "system build dependencies already present"
        return 0
      fi
      case "$PM" in
        apt-get)
          try_pkgs build-essential pkg-config curl git \
            libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev \
            libayatana-appindicator3-dev librsvg2-dev ||
            warn "apt install failed; install webkit2gtk-4.1 dev packages manually" ;;
        dnf)
          try_pkgs gcc make pkgconf-pkg-config curl git \
            webkit2gtk4.1-devel gtk3-devel librsvg2-devel ||
            warn "dnf install failed; install webkit2gtk-4.1 devel packages manually"
          try_pkgs libayatana-appindicator-gtk3-devel || true ;;
        pacman)
          try_pkgs base-devel curl git webkit2gtk gtk3 ||
            warn "pacman install failed; install webkit2gtk manually" ;;
        zypper)
          try_pkgs gcc make pkg-config curl git \
            libwebkit2gtk-4_1-devel gtk3-devel ||
            warn "zypper install failed; install libwebkit2gtk-4_1-devel manually" ;;
        apk)
          try_pkgs build-base curl git webkit2gtk-dev gtk3-dev ||
            warn "apk install failed; install webkit2gtk-dev manually" ;;
        *) warn "no known package manager; install webkit2gtk-4.1 dev files manually" ;;
      esac
      ;;
    FreeBSD|DragonFly)
      detect_pm
      have git || try_pkgs git || warn "git missing; needed only to clone"
      have curl || try_pkgs curl || warn "curl missing"
      have pkg-config || try_pkgs pkgconf || warn "pkgconf missing"
      if gtk_platform && ! webkit_present; then
        try_pkgs gtk3 || warn "gtk3 install failed"
        # The port's name moved across releases; CI tries the same chain.
        try_pkgs webkit2-gtk_41 || try_pkgs webkit2-gtk41 ||
          try_pkgs webkit2-gtk || try_pkgs webkit2-gtk4 ||
          warn "no webkit2gtk package found; install it with pkg before voussoir build"
      fi
      ;;
    NetBSD|OpenBSD)
      detect_pm
      have git || try_pkgs git || warn "git missing; needed only to clone"
      have curl || try_pkgs curl || warn "curl missing"
      if gtk_platform && ! webkit_present; then
        try_pkgs gtk3 || warn "gtk3 install failed"
        try_pkgs webkit2gtk ||
          warn "no webkit2gtk package found on $OS; the runtime cannot link until you provide one"
      fi
      ;;
  esac
}

# Called from a condition (have bun || ...), so a failure inside falls through
# to the final `have bun` status.
install_bun() {
  log "installing Bun"
  case "$OS" in
    Darwin)
      if have brew; then brew install bun || true; fi ;;
    FreeBSD|DragonFly)
      try_pkgs bun || true ;;
  esac
  if ! have bun; then
    if have curl; then
      curl -fsSL https://bun.sh/install | bash || warn "the bun.sh installer failed"
    elif have fetch; then
      fetch -o "${TMPDIR:-/tmp}/voussoir-bun-install.sh" https://bun.sh/install &&
        sh "${TMPDIR:-/tmp}/voussoir-bun-install.sh" || warn "the bun.sh installer failed"
    else
      warn "no curl/fetch to reach bun.sh"
    fi
    PATH="$HOME/.bun/bin:$PATH"
    export PATH
  fi
  have bun
}

install_rust() {
  log "installing Rust"
  case "$OS" in
    Darwin)
      if have brew; then
        brew install rust || brew install rustup-init || warn "brew rust install failed"
        if have rustup-init && ! have cargo; then
          rustup-init -y --profile minimal || warn "rustup-init failed"
        fi
      fi ;;
    FreeBSD|DragonFly|NetBSD|OpenBSD)
      try_pkgs rust || warn "system rust package failed" ;;
    Linux)
      detect_pm
      case "$PM" in
        pacman) try_pkgs rust || warn "pacman rust failed" ;;
        apt-get|dnf|zypper|apk) try_pkgs cargo || warn "distro cargo package failed" ;;
        *) : ;;
      esac ;;
  esac
  PATH="$HOME/.cargo/bin:$PATH"
  export PATH
  have cargo
}

# --- dependencies -----------------------------------------------------------

if [ "$WITH_DEPS" -eq 1 ]; then
  install_deps
else
  log "skipping system packages (--no-deps)"
fi

if gtk_platform && ! webkit_present; then
  warn "webkit2gtk not found via pkg-config; 'voussoir build' needs it (voussoir doctor names the package for this OS)"
fi

have bun || install_bun || die "Bun is required (CLI build and dev server); install it from bun.sh and re-run"
have cargo || install_rust || die "Rust is required; install it (rustup recommended) and re-run"

# --- source -----------------------------------------------------------------

case "$0" in
  */*) SCRIPT_DIR=$(CDPATH= cd "$(dirname "$0")" 2>/dev/null && pwd) || SCRIPT_DIR="" ;;
  # Piped to sh, or invoked by bare name: the current directory decides —
  # it holds a checkout when it does, and a random one clones instead.
  *) SCRIPT_DIR=$(pwd) || SCRIPT_DIR="" ;;
esac

if [ -n "$SCRIPT_DIR" ] && [ -f "$SCRIPT_DIR/cli/package.json" ] && [ -f "$SCRIPT_DIR/Cargo.toml" ]; then
  SRC=$SCRIPT_DIR
  log "building from the checkout at $SRC"
else
  SRC=${XDG_DATA_HOME:-$HOME/.local/share}/voussoir/src
  if [ -d "$SRC/.git" ]; then
    log "updating $SRC"
    git -C "$SRC" pull --ff-only || warn "could not update; building what is there"
  else
    have git || die "git is required to clone; install it and re-run"
    log "cloning $REPO to $SRC"
    mkdir -p "$(dirname "$SRC")"
    git clone --depth 1 "$REPO" "$SRC" || die "clone failed; pass --repo with a reachable URL"
  fi
fi

# --- build ------------------------------------------------------------------

log "building the CLI"
cd "$SRC/cli"
bun install
bun run build
[ -f dist/voussoir ] || die "build finished but dist/voussoir is missing"

# --- install ----------------------------------------------------------------

mkdir -p "$PREFIX/bin"
cp dist/voussoir "$PREFIX/bin/voussoir"
chmod 755 "$PREFIX/bin/voussoir"
log "installed $PREFIX/bin/voussoir"

# Record the checkout this binary was built from. `voussoir update` rebuilds
# that source rather than downloading a release, so the installed copy has to be
# told where it lives; guessing would walk away from a checkout right here.
SHARE=${XDG_DATA_HOME:-$HOME/.local/share}/voussoir
mkdir -p "$SHARE"
printf '%s\n' "$SRC" > "$SHARE/source"
log "recorded the source checkout in $SHARE/source"

case ":$PATH:" in
  *":$PREFIX/bin:"*) ;;
  *) log "add it to PATH:  export PATH=\"$PREFIX/bin:\$PATH\"" ;;
esac

# --- verify -----------------------------------------------------------------

log "verifying the installed binary"
"$PREFIX/bin/voussoir" help >/dev/null || die "the installed voussoir failed to run"

log "done. next: voussoir init myapp && cd myapp && voussoir dev   (voussoir doctor checks a project)"
