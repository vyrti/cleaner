//! Windows cleanup targets.
//!
//! Deliberately excluded, because the downside is far worse than the space
//! saved: `vssadmin delete shadows` (destroys every System Restore point),
//! `pagefile.sys` (live paging), and anything under `System Volume
//! Information`.
//!
//! Windows has no `sudo`, so [`Tier::NeedsRoot`] rows go through the UAC path in
//! [`crate::sysclean::elevate`] rather than a password prompt.

use super::Cat;
use crate::sysclean::{Group, Tier};

pub(super) fn fill(cat: &mut Cat) {
    let local = dirs::data_local_dir();
    let roaming = dirs::data_dir();
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
    let system_drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());

    system_junk(cat, &system_root, &system_drive);
    dev_caches(cat, local.as_deref());
    containers(cat, local.as_deref());
    apps(cat, local.as_deref(), roaming.as_deref());
}

/// Join a `%LOCALAPPDATA%`-relative path, returning `None` when the base is
/// unavailable so the caller can skip the entry entirely.
fn under(base: Option<&std::path::Path>, rel: &str) -> Option<String> {
    base.map(|b| b.join(rel).to_string_lossy().into_owned())
}

fn collect(base: Option<&std::path::Path>, rels: &[&str]) -> Vec<String> {
    rels.iter().filter_map(|r| under(base, r)).collect()
}

fn as_refs(items: &[String]) -> Vec<&str> {
    items.iter().map(String::as_str).collect()
}

fn system_junk(cat: &mut Cat, system_root: &str, system_drive: &str) {
    use Tier::{Destructive, Safe};

    let local = dirs::data_local_dir();

    let temp = collect(local.as_deref(), &["Temp"]);
    if !temp.is_empty() {
        cat.abs_rm(
            "win-user-temp",
            Group::SystemJunk,
            Safe,
            "User temp files",
            "%LOCALAPPDATA%\\Temp; recreated on demand",
            &as_refs(&temp),
        );
    }

    let wer = collect(local.as_deref(), &["Microsoft\\Windows\\WER", "CrashDumps"]);
    if !wer.is_empty() {
        cat.abs_rm(
            "win-error-reports",
            Group::SystemJunk,
            Safe,
            "Error reports and crash dumps",
            "queued Windows Error Reporting data",
            &as_refs(&wer),
        );
    }

    let inet = collect(local.as_deref(), &["Microsoft\\Windows\\INetCache"]);
    if !inet.is_empty() {
        cat.abs_rm(
            "win-inetcache",
            Group::SystemJunk,
            Safe,
            "Internet cache",
            "legacy WinINet cache; regenerated on demand",
            &as_refs(&inet),
        );
    }

    let thumbs = collect(
        local.as_deref(),
        &["Microsoft\\Windows\\Explorer\\thumbcache_*.db"],
    );
    if !thumbs.is_empty() {
        cat.abs_glob(
            "win-thumbcache",
            Group::SystemJunk,
            Safe,
            "Explorer thumbnail cache",
            "regenerated as you browse folders",
            &as_refs(&thumbs),
        );
    }

    cat.root_cmd(
        "win-recycle-bin",
        "Recycle Bin",
        "everything you have already thrown away",
        "powershell",
        &["-NoProfile", "-Command", "Clear-RecycleBin -Force"],
        &[],
    );

    let win_temp = format!("{system_root}\\Temp");
    cat.root_empty(
        "win-system-temp",
        "Windows temp files",
        "C:\\Windows\\Temp; recreated on demand",
        &[&win_temp],
    );

    let sd = format!("{system_root}\\SoftwareDistribution\\Download");
    let dosvc = format!("{system_root}\\SoftwareDistribution\\DeliveryOptimization");
    cat.root_empty(
        "win-update-cache",
        "Windows Update cache",
        "downloaded update payloads; re-downloaded if an update is retried",
        &[&sd, &dosvc],
    );

    let prefetch = format!("{system_root}\\Prefetch");
    cat.root_empty(
        "win-prefetch",
        "Prefetch data",
        "app launch is slightly slower until it rebuilds",
        &[&prefetch],
    );

    let cbs = format!("{system_root}\\Logs\\CBS");
    cat.root_empty(
        "win-cbs-logs",
        "Component servicing logs",
        "Windows servicing history",
        &[&cbs],
    );

    let windows_old = format!("{system_drive}\\Windows.old");
    cat.root_rm(
        "win-windows-old",
        Destructive,
        "Windows.old",
        "the previous Windows install; removes your ability to roll back",
        &[&windows_old],
    );

    cat.root_cmd(
        "win-component-cleanup",
        "WinSxS component cleanup",
        "removes superseded component versions; cannot be undone",
        "dism",
        &["/online", "/Cleanup-Image", "/StartComponentCleanup"],
        &[],
    );
    cat.root_cmd(
        "win-hibernation",
        "Disable hibernation file",
        "frees hiberfil.sys but disables hibernate and fast startup",
        "powercfg",
        &["/h", "off"],
        &[],
    );
}

fn dev_caches(cat: &mut Cat, local: Option<&std::path::Path>) {
    use Group::DevCache as D;
    use Tier::{Reclaimable, Safe};

    let npm = collect(local, &["npm-cache"]);
    if !npm.is_empty() {
        cat.abs_rm(
            "win-npm-cache",
            D,
            Safe,
            "npm cache",
            "package tarballs; npm refetches as needed",
            &as_refs(&npm),
        );
    }

    let yarn = collect(local, &["Yarn\\Cache"]);
    if !yarn.is_empty() {
        cat.abs_rm(
            "win-yarn-cache",
            D,
            Safe,
            "Yarn cache",
            "package tarballs; refetched on next install",
            &as_refs(&yarn),
        );
    }

    let pip = collect(local, &["pip\\Cache"]);
    if !pip.is_empty() {
        cat.abs_rm(
            "win-pip-cache",
            D,
            Safe,
            "pip cache",
            "wheels and downloads; pip refetches as needed",
            &as_refs(&pip),
        );
    }

    let gobuild = collect(local, &["go-build"]);
    if !gobuild.is_empty() {
        cat.abs_rm(
            "win-go-cache",
            D,
            Safe,
            "Go build cache",
            "compiled build artifacts; rebuilt on next go build",
            &as_refs(&gobuild),
        );
    }

    let nuget_http = collect(local, &["NuGet\\v3-cache", "NuGet\\plugins-cache"]);
    if !nuget_http.is_empty() {
        cat.abs_rm(
            "win-nuget-http",
            D,
            Safe,
            "NuGet HTTP cache",
            "refetched on next restore",
            &as_refs(&nuget_http),
        );
    }

    let vs = collect(local, &["Microsoft\\VisualStudio\\*\\ComponentModelCache"]);
    if !vs.is_empty() {
        cat.abs_glob(
            "win-vs-componentcache",
            D,
            Safe,
            "Visual Studio component cache",
            "rebuilt on next Visual Studio launch",
            &as_refs(&vs),
        );
    }

    cat.rm(
        "win-home-caches",
        D,
        Reclaimable,
        "Package manager caches",
        "NuGet, Gradle, Maven and Cargo caches; every project refetches",
        &[
            ".nuget\\packages",
            ".gradle\\caches",
            ".m2\\repository",
            ".cargo\\registry\\cache",
            ".cargo\\registry\\src",
        ],
    );
    cat.rm(
        "win-go-modcache",
        D,
        Reclaimable,
        "Go module cache",
        "downloaded modules; every project refetches on next build",
        &["go\\pkg\\mod"],
    );

    cat.cmd(
        "win-dotnet-nuget",
        D,
        Reclaimable,
        "NuGet caches",
        "clears all local NuGet caches; restored on next build",
        "dotnet",
        &["nuget", "locals", "all", "--clear"],
        &[],
    );

    let vs_packages = "C:\\ProgramData\\Microsoft\\VisualStudio\\Packages";
    cat.root_rm(
        "win-vs-installer-cache",
        Reclaimable,
        "Visual Studio installer cache",
        "downloaded installer payloads; re-downloaded if you modify the install",
        &[vs_packages],
    );
}

fn containers(cat: &mut Cat, local: Option<&std::path::Path>) {
    use Group::Container as C;
    use Tier::{Destructive, Safe};

    cat.cmd(
        "win-docker-prune",
        C,
        Safe,
        "Docker · prune unused",
        "removes dangling images, stopped containers and unused volumes",
        "docker",
        &["system", "prune", "-a", "--volumes", "-f"],
        &[],
    );

    // Docker Desktop moved the WSL2 data disk between layouts; offer both.
    let disks = collect(
        local,
        &[
            "Docker\\wsl\\data\\ext4.vhdx",
            "Docker\\wsl\\disk\\docker_data.vhdx",
        ],
    );
    if !disks.is_empty() {
        cat.abs_rm(
            "win-docker-vhdx",
            C,
            Destructive,
            "Docker · WIPE data disk",
            "destroys EVERY image, container and volume; quit Docker Desktop first",
            &as_refs(&disks),
        );
    }

    let wsl = collect(local, &["Packages\\*\\LocalState\\ext4.vhdx"]);
    if !wsl.is_empty() {
        cat.abs_report(
            "win-wsl-distros",
            C,
            "WSL distributions",
            "entire Linux installs; remove with `wsl --unregister`, never from here",
            &as_refs(&wsl),
        );
    }
}

fn apps(cat: &mut Cat, local: Option<&std::path::Path>, roaming: Option<&std::path::Path>) {
    use Group::{Browser, Chat, Editor, Games};
    use Tier::Safe;

    for (id, label, rel) in [
        (
            "chrome",
            "Google Chrome",
            "Google\\Chrome\\User Data\\*\\Cache",
        ),
        (
            "edge",
            "Microsoft Edge",
            "Microsoft\\Edge\\User Data\\*\\Cache",
        ),
        (
            "brave",
            "Brave",
            "BraveSoftware\\Brave-Browser\\User Data\\*\\Cache",
        ),
        ("vivaldi", "Vivaldi", "Vivaldi\\User Data\\*\\Cache"),
        ("chromium", "Chromium", "Chromium\\User Data\\*\\Cache"),
    ] {
        let paths = collect(local, &[rel]);
        if paths.is_empty() {
            continue;
        }
        cat.abs_glob(
            &format!("win-browser-{id}"),
            Browser,
            Safe,
            &format!("{label} cache"),
            "page and asset cache; cookies, history and passwords are untouched",
            &as_refs(&paths),
        );
    }

    let firefox = collect(local, &["Mozilla\\Firefox\\Profiles\\*\\cache2"]);
    if !firefox.is_empty() {
        cat.abs_glob(
            "win-browser-firefox",
            Browser,
            Safe,
            "Firefox cache",
            "page and asset cache; cookies, history and passwords are untouched",
            &as_refs(&firefox),
        );
    }

    for (id, label, rels) in [
        (
            "slack",
            "Slack",
            vec!["Slack\\Cache", "Slack\\Code Cache", "Slack\\GPUCache"],
        ),
        (
            "discord",
            "Discord",
            vec!["discord\\Cache", "discord\\Code Cache", "discord\\GPUCache"],
        ),
        (
            "teams",
            "Microsoft Teams",
            vec![
                "Microsoft\\Teams\\Cache",
                "Microsoft\\Teams\\Code Cache",
                "Microsoft\\Teams\\GPUCache",
            ],
        ),
        (
            "telegram",
            "Telegram",
            vec!["Telegram Desktop\\tdata\\user_data\\cache"],
        ),
        (
            "signal",
            "Signal",
            vec!["Signal\\Cache", "Signal\\Code Cache", "Signal\\GPUCache"],
        ),
    ] {
        let paths = collect(roaming, &rels);
        if paths.is_empty() {
            continue;
        }
        cat.abs_rm(
            &format!("win-chat-{id}"),
            Chat,
            Safe,
            &format!("{label} cache"),
            "message cache and preview data; chat history is untouched",
            &as_refs(&paths),
        );
    }

    let code = collect(
        roaming,
        &[
            "Code\\Cache",
            "Code\\CachedData",
            "Code\\CachedExtensionVSIXs",
            "Code\\Code Cache",
            "Code\\GPUCache",
            "Code\\logs",
        ],
    );
    if !code.is_empty() {
        cat.abs_rm(
            "win-vscode-caches",
            Editor,
            Safe,
            "VS Code caches",
            "cached extension packages and data; regenerated on next launch",
            &as_refs(&code),
        );
    }

    let jetbrains = collect(local, &["JetBrains"]);
    if !jetbrains.is_empty() {
        cat.abs_rm(
            "win-jetbrains-caches",
            Editor,
            Safe,
            "JetBrains caches",
            "project indexes; rebuilt on next open",
            &as_refs(&jetbrains),
        );
    }

    let onedrive = collect(local, &["Microsoft\\OneDrive\\setup\\logs"]);
    if !onedrive.is_empty() {
        cat.abs_rm(
            "win-onedrive-logs",
            Editor,
            Safe,
            "OneDrive setup logs",
            "install and sync logs",
            &as_refs(&onedrive),
        );
    }

    let steam = collect(local, &["Steam\\htmlcache"]);
    if !steam.is_empty() {
        cat.abs_rm(
            "win-steam-cache",
            Games,
            Safe,
            "Steam web cache",
            "regenerated on next launch; installed games are untouched",
            &as_refs(&steam),
        );
    }

    let epic = collect(local, &["EpicGamesLauncher\\Saved\\webcache"]);
    if !epic.is_empty() {
        cat.abs_rm(
            "win-epic-webcache",
            Games,
            Safe,
            "Epic Games web cache",
            "regenerated on next launch",
            &as_refs(&epic),
        );
    }

    let battlenet = collect(roaming, &["Battle.net\\Cache"]);
    if !battlenet.is_empty() {
        cat.abs_rm(
            "win-battlenet-cache",
            Games,
            Safe,
            "Battle.net cache",
            "regenerated on next launch",
            &as_refs(&battlenet),
        );
    }
}
