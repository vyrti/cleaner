//! macOS cleanup targets.
//!
//! Deliberately excluded, because removing them costs more than it saves:
//! `/private/var/vm` (live swap), `/System/Volumes/Preboot/*/cryptex1` (boot
//! data), `System/Library/AssetsV2` (macOS re-downloads it),
//! `/Library/Developer/CommandLineTools` (the compilers), and
//! `/opt/homebrew/Cellar` (installed programs - that is `brew uninstall`, not a
//! cache).

use super::Cat;
use crate::sysclean::{Group, Tier};

pub(super) fn fill(cat: &mut Cat) {
    dev_caches(cat);
    containers(cat);
    mobile(cat);
    editors(cat);
    browsers(cat);
    chat(cat);
    media(cat);
    games(cat);
    system_junk(cat);
}

fn dev_caches(cat: &mut Cat) {
    use Group::DevCache as D;
    use Tier::{Reclaimable, Safe};

    cat.cmd(
        "brew-cleanup",
        D,
        Safe,
        "Homebrew cache",
        "downloaded bottles and tarballs; re-downloaded on next install",
        "brew",
        &["cleanup", "-s", "--prune=all"],
        &["Library/Caches/Homebrew"],
    );
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
        "yarn-cache",
        D,
        Safe,
        "Yarn cache",
        "package tarballs; refetched on next install",
        "yarn",
        &["cache", "clean"],
        &["Library/Caches/Yarn"],
    );
    cat.cmd(
        "pnpm-store",
        D,
        Safe,
        "pnpm store",
        "prunes packages no longer referenced by any project",
        "pnpm",
        &["store", "prune"],
        &["Library/pnpm/store"],
    );
    cat.cmd(
        "go-cache",
        D,
        Safe,
        "Go build cache",
        "compiled build artifacts; rebuilt on next go build",
        "go",
        &["clean", "-cache"],
        &["Library/Caches/go-build"],
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
    cat.cmd(
        "conda-clean",
        D,
        Safe,
        "Conda package cache",
        "unused packages and tarballs",
        "conda",
        &["clean", "-a", "-y"],
        &[".conda/pkgs", "miniconda3/pkgs", "anaconda3/pkgs"],
    );
    cat.cmd(
        "dotnet-nuget",
        D,
        Reclaimable,
        "NuGet caches",
        "clears all local NuGet caches; restored on next build",
        "dotnet",
        &["nuget", "locals", "all", "--clear"],
        &[".nuget/packages"],
    );
    cat.cmd(
        "nix-gc",
        D,
        Reclaimable,
        "Nix garbage collection",
        "deletes unreachable store paths and old generations",
        "nix-collect-garbage",
        &["-d"],
        &[],
    );

    cat.rm(
        "pip-cache",
        D,
        Safe,
        "pip cache",
        "wheels and downloads; pip refetches as needed",
        &["Library/Caches/pip"],
    );
    cat.rm(
        "uv-cache",
        D,
        Safe,
        "uv cache",
        "Python package cache; refetched on demand",
        &["Library/Caches/uv"],
    );
    cat.rm(
        "poetry-cache",
        D,
        Safe,
        "Poetry cache",
        "Python package cache; refetched on demand",
        &["Library/Caches/pypoetry"],
    );
    cat.rm(
        "bun-cache",
        D,
        Safe,
        "Bun install cache",
        "package cache; refetched on demand",
        &[".bun/install/cache"],
    );
    cat.rm(
        "deno-cache",
        D,
        Safe,
        "Deno cache",
        "remote module cache; refetched on demand",
        &["Library/Caches/deno"],
    );
    cat.rm(
        "swiftpm-cache",
        D,
        Safe,
        "SwiftPM cache",
        "package checkouts; refetched on next resolve",
        &["Library/Caches/org.swift.swiftpm"],
    );
    cat.rm(
        "cocoapods-cache",
        D,
        Safe,
        "CocoaPods cache",
        "pod specs and downloads",
        &["Library/Caches/CocoaPods"],
    );
    cat.rm(
        "carthage-cache",
        D,
        Safe,
        "Carthage cache",
        "prebuilt dependency binaries",
        &["Library/Caches/org.carthage.CarthageKit"],
    );
    cat.rm(
        "composer-cache",
        D,
        Safe,
        "Composer cache",
        "PHP package cache; refetched on demand",
        &[".composer/cache", ".cache/composer"],
    );
    cat.rm(
        "bundler-cache",
        D,
        Safe,
        "Bundler cache",
        "cached gems; refetched on next bundle install",
        &[".bundle/cache"],
    );
    cat.rm(
        "node-gyp-cache",
        D,
        Safe,
        "node-gyp headers",
        "downloaded Node headers; refetched when building native modules",
        &["Library/Caches/node-gyp"],
    );
    cat.rm(
        "sccache",
        D,
        Safe,
        "sccache",
        "shared compilation cache; rebuilds are slower until it refills",
        &["Library/Caches/Mozilla.sccache"],
    );
    cat.rm(
        "cargo-xwin",
        D,
        Safe,
        "cargo-xwin cache",
        "Windows cross-compilation SDK cache",
        &["Library/Caches/cargo-xwin"],
    );
    cat.rm(
        "misc-build-caches",
        D,
        Safe,
        "Assorted build caches",
        "goimports, worker-build, typescript, helm, sherpa-rs",
        &[
            "Library/Caches/goimports",
            "Library/Caches/worker-build",
            "Library/Caches/typescript",
            "Library/Caches/helm",
            "Library/Caches/sherpa-rs",
        ],
    );
    cat.rm(
        "terraform-plugins",
        D,
        Safe,
        "Terraform plugin cache",
        "provider binaries; re-downloaded on terraform init",
        &[".terraform.d/plugin-cache"],
    );

    cat.rm(
        "gradle-caches",
        D,
        Reclaimable,
        "Gradle caches",
        "dependency cache; every project refetches on next build",
        &[".gradle/caches"],
    );
    cat.rm(
        "maven-repo",
        D,
        Reclaimable,
        "Maven repository",
        "dependency cache; every project refetches on next build",
        &[".m2/repository"],
    );
    cat.rm(
        "cargo-registry",
        D,
        Reclaimable,
        "Cargo registry cache",
        "crate sources and downloads; every project refetches on next build",
        &[".cargo/registry/cache", ".cargo/registry/src"],
    );
    cat.rm(
        "playwright",
        D,
        Reclaimable,
        "Playwright browsers",
        "re-downloaded on the next test run",
        &[
            "Library/Caches/ms-playwright",
            "Library/Caches/ms-playwright-go",
        ],
    );
    cat.rm(
        "puppeteer",
        D,
        Reclaimable,
        "Puppeteer browsers",
        "re-downloaded on next use",
        &[".cache/puppeteer", "Library/Caches/puppeteer"],
    );
    cat.rm(
        "cypress",
        D,
        Reclaimable,
        "Cypress binaries",
        "re-downloaded on next run",
        &["Library/Caches/Cypress"],
    );
    cat.rm(
        "electron",
        D,
        Reclaimable,
        "Electron downloads",
        "prebuilt Electron binaries; re-downloaded on next build",
        &["Library/Caches/electron", "Library/Caches/electron-builder"],
    );
    cat.rm(
        "pub-cache",
        D,
        Reclaimable,
        "Dart/Flutter pub cache",
        "packages refetched on next pub get",
        &[".pub-cache"],
    );
    cat.rm(
        "vagrant-boxes",
        D,
        Reclaimable,
        "Vagrant boxes",
        "base box images; re-downloaded on next vagrant up",
        &[".vagrant.d/boxes"],
    );
    cat.rm(
        "minikube",
        D,
        Reclaimable,
        "minikube cache",
        "cached Kubernetes ISO and images",
        &[".minikube/cache"],
    );
}

fn containers(cat: &mut Cat) {
    use Group::Container as C;
    use Tier::{Destructive, Safe};

    cat.cmd(
        "docker-prune",
        C,
        Safe,
        "Docker · prune unused",
        "removes dangling images, stopped containers and unused volumes",
        "docker",
        &["system", "prune", "-a", "--volumes", "-f"],
        &[],
    );
    cat.rm(
        "docker-installer",
        C,
        Safe,
        "Docker · stalled installer download",
        "an interrupted Docker Desktop update; safe once Docker is not updating",
        &["Library/Application Support/com.docker.install/in_progress"],
    );
    cat.rm(
        "docker-raw",
        C,
        Destructive,
        "Docker · WIPE disk image",
        "destroys EVERY image, container and volume; quit Docker Desktop first",
        &["Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw"],
    );
    cat.rm(
        "colima-disk",
        C,
        Destructive,
        "colima · WIPE VM disk",
        "destroys the colima VM and everything inside it",
        &[".colima/_lima"],
    );
    cat.rm(
        "podman-machine",
        C,
        Destructive,
        "Podman · WIPE machine disk",
        "destroys the Podman machine and everything inside it",
        &[".local/share/containers/podman/machine"],
    );

    cat.report(
        "vmware-fusion",
        C,
        "VMware Fusion virtual machines",
        "guest operating systems; delete from VMware, never from here",
        &["Virtual Machines.localized"],
    );
    cat.report(
        "parallels-vms",
        C,
        "Parallels virtual machines",
        "guest operating systems; delete from Parallels, never from here",
        &["Parallels"],
    );
    cat.report(
        "virtualbox-vms",
        C,
        "VirtualBox virtual machines",
        "guest operating systems; delete from VirtualBox, never from here",
        &["VirtualBox VMs"],
    );
}

fn mobile(cat: &mut Cat) {
    use Group::Mobile as M;
    use Tier::{Reclaimable, Safe};

    cat.empty(
        "xcode-derived-data",
        M,
        Safe,
        "Xcode DerivedData",
        "build intermediates and indexes; rebuilt on next build",
        &["Library/Developer/Xcode/DerivedData"],
    );
    cat.empty(
        "xcode-coding-assistant",
        M,
        Safe,
        "Xcode coding assistant cache",
        "regenerated on demand",
        &["Library/Developer/Xcode/CodingAssistant"],
    );
    cat.rm(
        "xcode-simulator-caches",
        M,
        Safe,
        "CoreSimulator caches",
        "simulator runtime caches; regenerated on demand",
        &["Library/Developer/CoreSimulator/Caches"],
    );
    cat.cmd(
        "simctl-unavailable",
        M,
        Safe,
        "Unavailable iOS simulators",
        "simulators whose runtime is no longer installed",
        "xcrun",
        &["simctl", "delete", "unavailable"],
        &["Library/Developer/CoreSimulator/Devices"],
    );
    cat.rm(
        "xcode-device-support",
        M,
        Reclaimable,
        "Xcode device support",
        "debug symbols for old iOS/watchOS/tvOS versions; re-downloaded on connect",
        &[
            "Library/Developer/Xcode/iOS DeviceSupport",
            "Library/Developer/Xcode/watchOS DeviceSupport",
            "Library/Developer/Xcode/tvOS DeviceSupport",
        ],
    );
    cat.rm(
        "xcode-archives",
        M,
        Reclaimable,
        "Xcode archives",
        "shippable build outputs; not recoverable once deleted",
        &[
            "Library/Developer/Xcode/Archives",
            "Library/Developer/Xcode/Products",
        ],
    );
    cat.rm(
        "ios-software-updates",
        M,
        Safe,
        "iOS software update downloads",
        "cached iOS installers; re-downloaded if needed",
        &["Library/iTunes/iPhone Software Updates"],
    );
    cat.rm(
        "ios-backups",
        M,
        Reclaimable,
        "iOS device backups",
        "REAL device backups; irreplaceable unless you have another copy",
        &["Library/Application Support/MobileSync/Backup"],
    );
    cat.rm(
        "android-avd",
        M,
        Reclaimable,
        "Android emulators",
        "emulator disk images; recreated from Android Studio",
        &[".android/avd"],
    );
    cat.rm(
        "android-system-images",
        M,
        Reclaimable,
        "Android SDK system images",
        "re-downloaded via the SDK manager",
        &["Library/Android/sdk/system-images"],
    );
    cat.glob(
        "android-studio-caches",
        M,
        Safe,
        "Android Studio caches",
        "index and build caches; rebuilt on next open",
        &["Library/Caches/Google/AndroidStudio*"],
    );
}

fn editors(cat: &mut Cat) {
    use Group::Editor as E;
    use Tier::Safe;

    cat.rm(
        "cursor-caches",
        E,
        Safe,
        "Cursor caches and logs",
        "regenerated on next launch",
        &[
            "Library/Application Support/Cursor/logs",
            "Library/Application Support/Cursor/CachedData",
            "Library/Application Support/Cursor/Cache",
            "Library/Application Support/Cursor/Code Cache",
            "Library/Application Support/Cursor/GPUCache",
        ],
    );
    cat.rm(
        "vscode-caches",
        E,
        Safe,
        "VS Code caches",
        "cached extension packages and data; regenerated on next launch",
        &[
            "Library/Application Support/Code/CachedExtensionVSIXs",
            "Library/Application Support/Code/CachedData",
            "Library/Application Support/Code/Cache",
            "Library/Application Support/Code/Code Cache",
            "Library/Application Support/Code/GPUCache",
            "Library/Application Support/Code/logs",
        ],
    );
    cat.rm(
        "vscodium-caches",
        E,
        Safe,
        "VSCodium caches",
        "regenerated on next launch",
        &[
            "Library/Application Support/VSCodium/CachedExtensionVSIXs",
            "Library/Application Support/VSCodium/CachedData",
            "Library/Application Support/VSCodium/Cache",
            "Library/Application Support/VSCodium/logs",
        ],
    );
    cat.rm(
        "jetbrains-caches",
        E,
        Safe,
        "JetBrains caches and logs",
        "project indexes; rebuilt on next open",
        &["Library/Caches/JetBrains", "Library/Logs/JetBrains"],
    );
    cat.rm(
        "zed-logs",
        E,
        Safe,
        "Zed logs",
        "regenerated on next launch",
        &["Library/Logs/Zed"],
    );
    cat.rm(
        "sublime-cache",
        E,
        Safe,
        "Sublime Text cache",
        "regenerated on next launch",
        &[
            "Library/Caches/com.sublimetext.4",
            "Library/Caches/com.sublimetext.3",
        ],
    );
    cat.rm(
        "claude-caches",
        E,
        Safe,
        "Claude desktop caches",
        "regenerated on next launch",
        &[
            "Library/Application Support/Claude/Cache",
            "Library/Application Support/Claude/Code Cache",
            "Library/Application Support/Claude/GPUCache",
        ],
    );
    cat.rm(
        "antigravity-updater",
        E,
        Safe,
        "Antigravity updater cache",
        "downloaded update payloads",
        &["Library/Caches/antigravity-updater"],
    );
    cat.rm(
        "app-logs",
        E,
        Safe,
        "Application logs",
        "user-level application logs",
        &[
            "Library/Logs/DiagnosticReports",
            "Library/Logs/CoreSimulator",
        ],
    );
}

fn browsers(cat: &mut Cat) {
    use Group::Browser as B;
    use Tier::Safe;

    // Caches only. Cookies, history, passwords and profile data are never
    // touched - those live outside the Caches tree and outside these globs.
    for (id, label, cache) in [
        ("chrome", "Google Chrome", "Library/Caches/Google/Chrome"),
        (
            "chrome-canary",
            "Chrome Canary",
            "Library/Caches/Google/Chrome Canary",
        ),
        ("chromium", "Chromium", "Library/Caches/Chromium"),
        (
            "brave",
            "Brave",
            "Library/Caches/BraveSoftware/Brave-Browser",
        ),
        ("edge", "Microsoft Edge", "Library/Caches/Microsoft Edge"),
        ("vivaldi", "Vivaldi", "Library/Caches/Vivaldi"),
        ("opera", "Opera", "Library/Caches/com.operasoftware.Opera"),
        ("arc", "Arc", "Library/Caches/company.thebrowser.Browser"),
        ("firefox", "Firefox", "Library/Caches/Firefox"),
        ("safari", "Safari", "Library/Caches/com.apple.Safari"),
    ] {
        cat.empty(
            &format!("browser-{id}"),
            B,
            Safe,
            &format!("{label} cache"),
            "page and asset cache; cookies, history and passwords are untouched",
            &[cache],
        );
    }

    cat.glob(
        "chromium-profile-caches",
        B,
        Safe,
        "Chromium profile service-worker caches",
        "per-profile service worker caches; logins are untouched",
        &[
            "Library/Application Support/Google/Chrome/*/Service Worker/CacheStorage",
            "Library/Application Support/BraveSoftware/Brave-Browser/*/Service Worker/CacheStorage",
            "Library/Application Support/Microsoft Edge/*/Service Worker/CacheStorage",
        ],
    );
}

fn chat(cat: &mut Cat) {
    use Group::Chat as C;
    use Tier::{Reclaimable, Safe};

    for (id, label, dirs) in [
        (
            "slack",
            "Slack",
            vec![
                "Library/Application Support/Slack/Cache",
                "Library/Application Support/Slack/Code Cache",
                "Library/Application Support/Slack/GPUCache",
                "Library/Application Support/Slack/Service Worker/CacheStorage",
            ],
        ),
        (
            "discord",
            "Discord",
            vec![
                "Library/Application Support/discord/Cache",
                "Library/Application Support/discord/Code Cache",
                "Library/Application Support/discord/GPUCache",
                "Library/Application Support/discordcanary/Cache",
                "Library/Application Support/discordptb/Cache",
            ],
        ),
        (
            "teams",
            "Microsoft Teams",
            vec![
                "Library/Containers/com.microsoft.teams2/Data/Library/Caches",
                "Library/Application Support/Microsoft/Teams/Cache",
                "Library/Application Support/Microsoft/Teams/Code Cache",
                "Library/Application Support/Microsoft/Teams/GPUCache",
            ],
        ),
        (
            "telegram",
            "Telegram",
            vec!["Library/Application Support/Telegram Desktop/tdata/user_data/cache"],
        ),
        (
            "signal",
            "Signal",
            vec![
                "Library/Application Support/Signal/Cache",
                "Library/Application Support/Signal/Code Cache",
                "Library/Application Support/Signal/GPUCache",
            ],
        ),
        (
            "whatsapp",
            "WhatsApp",
            vec!["Library/Application Support/WhatsApp/Cache"],
        ),
        ("zoom", "Zoom", vec!["Library/Caches/us.zoom.xos"]),
        (
            "skype",
            "Skype",
            vec!["Library/Application Support/Skype/Cache"],
        ),
    ] {
        cat.rm(
            &format!("chat-{id}"),
            C,
            Safe,
            &format!("{label} cache"),
            "message cache and attachments preview; chat history is untouched",
            &dirs,
        );
    }

    cat.rm(
        "zoom-recordings",
        C,
        Reclaimable,
        "Zoom local recordings",
        "YOUR recorded meetings; not recoverable once deleted",
        &["Documents/Zoom"],
    );
}

fn media(cat: &mut Cat) {
    use Group::Media as M;
    use Tier::Safe;

    cat.rm(
        "spotify-cache",
        M,
        Safe,
        "Spotify cache",
        "streamed audio cache; re-downloaded on demand",
        &[
            "Library/Caches/com.spotify.client",
            "Library/Application Support/Spotify/PersistentCache",
        ],
    );
    cat.rm(
        "adobe-media-cache",
        M,
        Safe,
        "Adobe media cache",
        "conformed audio and peak files; regenerated when a project reopens",
        &[
            "Library/Application Support/Adobe/Common/Media Cache Files",
            "Library/Application Support/Adobe/Common/Media Cache",
            "Library/Caches/Adobe",
        ],
    );
    cat.rm(
        "vlc-cache",
        M,
        Safe,
        "VLC cache",
        "regenerated on demand",
        &["Library/Caches/org.videolan.vlc"],
    );
    cat.rm(
        "iina-cache",
        M,
        Safe,
        "IINA cache",
        "regenerated on demand",
        &["Library/Caches/com.colliderli.iina"],
    );
}

fn games(cat: &mut Cat) {
    use Group::Games as G;
    use Tier::Safe;

    cat.rm(
        "steam-shadercache",
        G,
        Safe,
        "Steam shader cache",
        "recompiled on next launch; installed games are untouched",
        &[
            "Library/Application Support/Steam/steamapps/shadercache",
            "Library/Application Support/Steam/steamapps/downloading",
            "Library/Caches/Steam",
        ],
    );
    cat.rm(
        "battlenet-cache",
        G,
        Safe,
        "Battle.net cache",
        "regenerated on next launch",
        &["Library/Application Support/Battle.net/Cache"],
    );
    cat.rm(
        "epic-webcache",
        G,
        Safe,
        "Epic Games web cache",
        "regenerated on next launch",
        &["Library/Application Support/Epic/EpicGamesLauncher/Saved/webcache"],
    );
    cat.rm(
        "unity-cache",
        G,
        Safe,
        "Unity cache",
        "asset and package cache; rebuilt on next open",
        &[
            "Library/Unity/cache",
            "Library/Caches/com.unity3d.UnityEditor",
        ],
    );
}

fn system_junk(cat: &mut Cat) {
    use Group::{Downloads, SystemJunk as S};
    use Tier::{Reclaimable, Safe};

    cat.rm(
        "trash",
        S,
        Reclaimable,
        "Trash",
        "everything you have already thrown away",
        &[".Trash"],
    );
    cat.glob(
        "partial-downloads",
        Downloads,
        Safe,
        "Stale partial downloads",
        "interrupted downloads that can never resume",
        &[
            "Downloads/*.part",
            "Downloads/*.crdownload",
            "Downloads/*.download",
        ],
    );
    cat.rm(
        "saved-app-state",
        S,
        Safe,
        "Saved application state",
        "window positions and restored documents; apps reopen fresh",
        &["Library/Saved Application State"],
    );
    cat.rm(
        "quicklook-cache",
        S,
        Safe,
        "QuickLook thumbnail cache",
        "regenerated on demand",
        &["Library/Caches/com.apple.QuickLook.thumbnailcache"],
    );
    cat.rm(
        "mail-downloads",
        S,
        Reclaimable,
        "Mail attachment downloads",
        "downloaded attachments; the mail store itself is untouched",
        &["Library/Containers/com.apple.mail/Data/Library/Mail Downloads"],
    );

    cat.root_empty(
        "macos-install-data",
        "macOS Install Data",
        "a leftover macOS installer bundle from a completed update",
        &["/System/Volumes/Data/macOS Install Data"],
    );
    cat.root_empty(
        "unified-logs",
        "Unified log archives",
        "system diagnostic history; `log show` loses older data",
        &["/private/var/db/diagnostics", "/private/var/db/uuidtext"],
    );
    cat.root_empty(
        "system-caches",
        "System caches and logs",
        "/Library/Caches and /Library/Logs; regenerated on demand",
        &["/Library/Caches", "/Library/Logs"],
    );
    cat.root_cmd(
        "tm-snapshots",
        "Time Machine local snapshots",
        "local restore points; you lose the ability to roll back to them",
        "tmutil",
        &["thinlocalsnapshots", "/", "999999999999", "4"],
        &[],
    );
    cat.root_cmd(
        "spotlight-rebuild",
        "Rebuild Spotlight index",
        "search is degraded for hours while it re-indexes",
        "mdutil",
        &["-E", "/"],
        &[],
    );
    cat.root_cmd(
        "font-cache",
        "Reset font caches",
        "regenerated on next login; requires a restart to take effect",
        "atsutil",
        &["databases", "-remove"],
        &[],
    );
}
