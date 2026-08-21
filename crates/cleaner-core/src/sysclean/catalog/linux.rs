//! Linux cleanup targets.
//!
//! Smaller than the macOS and Windows tables by design - it covers the XDG
//! cache layout and the common package managers, and leaves distro-specific
//! surgery alone. It is also the table the ubuntu CI job exercises, so it must
//! stay non-empty.

use super::Cat;
use crate::sysclean::{Group, Tier};

pub(super) fn fill(cat: &mut Cat) {
    dev_caches(cat);
    apps(cat);
    system_junk(cat);
}

fn dev_caches(cat: &mut Cat) {
    use Group::DevCache as D;
    use Tier::{Reclaimable, Safe};

    cat.cmd(
        "npm-cache",
        D,
        Safe,
        "npm cache",
        "package tarballs; npm refetches as needed",
        "npm",
        &["cache", "clean", "--force"],
        &[".npm/_cacache"],
    );
    cat.cmd(
        "go-cache",
        D,
        Safe,
        "Go build cache",
        "compiled build artifacts; rebuilt on next go build",
        "go",
        &["clean", "-cache"],
        &[".cache/go-build"],
    );
    cat.cmd(
        "go-modcache",
        D,
        Reclaimable,
        "Go module cache",
        "downloaded modules; every project refetches on next build",
        "go",
        &["clean", "-modcache"],
        &["go/pkg/mod"],
    );

    cat.rm(
        "xdg-tool-caches",
        D,
        Safe,
        "Language tool caches",
        "pip, uv, yarn, pnpm and composer caches; refetched on demand",
        &[
            ".cache/pip",
            ".cache/uv",
            ".cache/yarn",
            ".cache/pnpm",
            ".cache/composer",
            ".cache/node-gyp",
        ],
    );
    cat.rm(
        "package-manager-caches",
        D,
        Reclaimable,
        "Package manager caches",
        "Gradle, Maven and Cargo caches; every project refetches",
        &[
            ".gradle/caches",
            ".m2/repository",
            ".cargo/registry/cache",
            ".cargo/registry/src",
        ],
    );
    cat.rm(
        "playwright",
        D,
        Reclaimable,
        "Playwright browsers",
        "re-downloaded on the next test run",
        &[".cache/ms-playwright"],
    );
}

fn apps(cat: &mut Cat) {
    use Group::{Browser, Container, Editor};
    use Tier::Safe;

    cat.rm(
        "browser-caches",
        Browser,
        Safe,
        "Browser caches",
        "page and asset cache; cookies, history and passwords are untouched",
        &[
            ".cache/mozilla",
            ".cache/google-chrome",
            ".cache/chromium",
            ".cache/BraveSoftware",
        ],
    );
    cat.rm(
        "thumbnail-cache",
        Editor,
        Safe,
        "Thumbnail cache",
        "regenerated as you browse folders",
        &[".cache/thumbnails"],
    );
    cat.rm(
        "vscode-caches",
        Editor,
        Safe,
        "VS Code caches",
        "cached extension packages and data; regenerated on next launch",
        &[
            ".config/Code/Cache",
            ".config/Code/CachedData",
            ".config/Code/CachedExtensionVSIXs",
            ".config/Code/logs",
        ],
    );
    cat.cmd(
        "docker-prune",
        Container,
        Safe,
        "Docker · prune unused",
        "removes dangling images, stopped containers and unused volumes",
        "docker",
        &["system", "prune", "-a", "--volumes", "-f"],
        &[],
    );
    cat.cmd(
        "flatpak-unused",
        Container,
        Safe,
        "Unused Flatpak runtimes",
        "runtimes no longer required by any installed application",
        "flatpak",
        &["uninstall", "--unused", "-y"],
        &[],
    );
}

fn system_junk(cat: &mut Cat) {
    use Group::SystemJunk as S;
    use Tier::Reclaimable;

    cat.rm(
        "trash",
        S,
        Reclaimable,
        "Trash",
        "everything you have already thrown away",
        &[".local/share/Trash"],
    );

    cat.root_cmd(
        "journal-vacuum",
        "Vacuum systemd journal",
        "trims the journal to 200M; older system logs are lost",
        "journalctl",
        &["--vacuum-size=200M"],
        &["/var/log/journal"],
    );
    cat.root_empty(
        "apt-cache",
        "APT package cache",
        "downloaded .deb archives; re-downloaded if reinstalled",
        &["/var/cache/apt/archives"],
    );
    cat.root_empty(
        "dnf-cache",
        "DNF package cache",
        "downloaded packages and metadata",
        &["/var/cache/dnf"],
    );
}
