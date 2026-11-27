//! Explicit, per-user Hex setup. Constructing state and detecting files never use the network.
use serde::Deserialize;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::windows::{fs::OpenOptionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const DOWNLOAD_PAGE: &str = "https://github.com/anomalyco/hex/releases/latest";
const RELEASE_API: &str = "https://api.github.com/repos/anomalyco/hex/releases/latest";
const MAX_INSTALLER: u64 = 512 * 1024 * 1024;
type Result<T> = std::result::Result<T, &'static str>;

pub use crate::hex_install_state::{Installation, State};

fn valid_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 128
        && version.bytes().any(|c| c.is_ascii_digit())
        && version
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-_".contains(&c))
        && version != "."
        && version != ".."
}

pub fn detect_in(root: &Path) -> Installation {
    let pointer = root.join("current-version");
    if let Ok(version) = fs::read_to_string(&pointer) {
        let version = version.trim();
        if valid_version(version) {
            let exe = root.join("versions").join(version).join("hex.exe");
            if exe.is_file() {
                return Installation::Installed {
                    version: version.into(),
                    exe,
                };
            }
        }
        return Installation::Incomplete;
    }
    // An unreadable pointer or legacy files must not be mistaken for permission to overwrite Hex.
    if !matches!(pointer.try_exists(), Ok(false))
        || !matches!(root.join("versions").try_exists(), Ok(false))
        || !matches!(root.join("bin").try_exists(), Ok(false))
        || !matches!(root.join("hex.exe").try_exists(), Ok(false))
    {
        Installation::Incomplete
    } else {
        Installation::Missing
    }
}

pub fn detect() -> Installation {
    std::env::var_os("APPDATA")
        .map(|p| detect_in(&PathBuf::from(p).join("hex")))
        .unwrap_or(Installation::Incomplete)
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Clone, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

fn select_asset(release: Release, arch: &str) -> Result<Asset> {
    let version = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name);
    if release.draft || release.prerelease || !valid_version(version) {
        return Err("Could not find a stable Hex release. Use the download page.");
    }
    let name = format!("hex-setup-{version}-windows-{arch}.exe");
    let expected_url = format!(
        "https://github.com/anomalyco/hex/releases/download/{}/{name}",
        release.tag_name
    );
    let asset = release
        .assets
        .into_iter()
        .find(|a| a.name == name && a.browser_download_url == expected_url)
        .ok_or("No Hex installer is available for this PC. Use the download page.")?;
    let digest = asset
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"));
    if asset.size == 0
        || asset.size > MAX_INSTALLER
        || !digest.is_some_and(|d| d.len() == 64 && d.bytes().all(|c| c.is_ascii_hexdigit()))
    {
        return Err(
            "The Hex release has no valid download size or SHA-256. Use the download page.",
        );
    }
    Ok(asset)
}

trait Backend {
    fn detect(&self) -> Installation;
    fn check_allowed(&self) -> Result<()>;
    fn latest(&mut self) -> Result<Asset>;
    fn download(
        &mut self,
        asset: &Asset,
        out: &mut File,
        report: &mut dyn FnMut(State),
    ) -> Result<()>;
    fn verify_signer(&mut self, path: &Path) -> Result<()>;
    fn install(&mut self, path: &Path) -> Result<()>;
}

fn try_install(backend: &mut impl Backend, report: &mut dyn FnMut(State)) -> Result<()> {
    if backend.detect() != Installation::Missing {
        return Err(
            "Hex is already installed or has an incomplete installation. Use the download page.",
        );
    }
    backend.check_allowed()?;
    let asset = backend.latest()?;
    let dir = tempfile::Builder::new()
        .prefix("sidenote-hex-")
        .tempdir()
        .map_err(|_| "Could not create a temporary download folder.")?;
    let path = dir.path().join("hex-setup.exe");
    {
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(&path)
            .map_err(|_| "Could not create the Hex download.")?;
        backend.download(&asset, &mut out, report)?;
        out.sync_all()
            .map_err(|_| "Could not save the Hex download.")?;
    }
    // Hold a read-only handle denying writes/deletion through signature verification and execution.
    let mut pinned = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .map_err(|_| "Could not lock the Hex download for verification.")?;
    report(State::Verifying);
    if pinned
        .metadata()
        .map_err(|_| "Could not read the Hex download.")?
        .len()
        != asset.size
        || !crate::updates::verify_sha256(
            &mut pinned,
            asset
                .digest
                .as_deref()
                .and_then(|d| d.strip_prefix("sha256:"))
                .unwrap_or(""),
        )
        .map_err(|_| "Could not read the Hex download.")?
    {
        return Err("The Hex download failed its SHA-256 check. It was not run. Try again.");
    }
    backend.verify_signer(&path)?;
    // Re-check after the download in case another installer or Hex started meanwhile.
    if backend.detect() != Installation::Missing {
        return Err("Hex appeared during setup. It was not replaced. Check again.");
    }
    backend.check_allowed()?;
    report(State::Installing);
    backend.install(&path)?;
    drop(pinned);
    dir.close()
        .map_err(|_| "Hex setup finished, but its temporary download could not be removed.")?;
    if !matches!(backend.detect(), Installation::Installed { .. }) {
        return Err("Hex setup finished, but Sidenote could not find Hex. Use the download page.");
    }
    Ok(())
}

/// Must only be called by the UI worker started by an explicit Install Hex click.
pub fn install_after_click(mut report: impl FnMut(State)) {
    let _ = run(&mut WindowsBackend::new(), &mut report);
}

fn run(backend: &mut impl Backend, report: &mut dyn FnMut(State)) -> Result<()> {
    let result = try_install(backend, report);
    report(match result {
        Ok(()) => State::Complete,
        Err(message) => State::Failed(message),
    });
    result
}

struct WindowsBackend {
    agent: ureq::Agent,
}
impl WindowsBackend {
    fn new() -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .https_only(true)
                .timeout_connect(Some(Duration::from_secs(10)))
                .timeout_global(Some(Duration::from_secs(300)))
                .build()
                .into(),
        }
    }
}

fn unelevated() -> Result<()> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    // SAFETY: the token belongs to this process; the buffer matches TokenElevation and is sized.
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err("Could not check Windows account permissions. Use the download page.");
        }
        let mut elevation: TOKEN_ELEVATION = std::mem::zeroed();
        let mut size = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            std::mem::size_of_val(&elevation) as u32,
            &mut size,
        );
        CloseHandle(token);
        if ok == 0 || elevation.TokenIsElevated != 0 {
            return Err(
                "Close Sidenote and open it without administrator permissions to install Hex.",
            );
        }
    }
    Ok(())
}

fn wait_hidden(command: &mut Command, timeout: Duration) -> Result<i32> {
    let mut child = command
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(
            |_| "Could not start Hex setup verification or installation. Use the download page.",
        )?;
    let job = crate::platform::ProcessJob::attach(&child).map_err(|_| {
        let _ = child.kill();
        let _ = child.wait();
        "Could not monitor Hex setup. Try again."
    })?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.code().unwrap_or(-1)),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                drop(job);
                let _ = child.kill();
                let _ = child.wait();
                return Err(
                    "Hex setup did not finish in time. Try again or use the download page.",
                );
            }
        }
    }
}

impl Backend for WindowsBackend {
    fn detect(&self) -> Installation {
        detect()
    }
    fn check_allowed(&self) -> Result<()> {
        unelevated()?;
        if crate::platform::hex_running()
            .map_err(|_| "Could not check whether Hex is running. Close Hex and try again.")?
        {
            return Err("Close Hex before installing it. Sidenote will not stop a recording.");
        }
        Ok(())
    }
    fn latest(&mut self) -> Result<Asset> {
        let json = self.agent.get(RELEASE_API)
            .header("User-Agent", "Sidenote-Hex-Setup")
            .header("Accept", "application/vnd.github+json")
            .config().timeout_global(Some(Duration::from_secs(30))).build()
            .call().map_err(|_| "Could not check the Hex release. Check your connection or use the download page.")?
            .body_mut().with_config().limit(2 * 1024 * 1024).read_to_string()
            .map_err(|_| "Could not read the Hex release. Use the download page.")?;
        let release = serde_json::from_str(&json)
            .map_err(|_| "Could not read the Hex release. Use the download page.")?;
        let arch = match std::env::consts::ARCH {
            "x86_64" => "x64",
            "aarch64" => "arm64",
            _ => return Err("This PC needs a different Hex installer. Use the download page."),
        };
        select_asset(release, arch)
    }
    fn download(
        &mut self,
        asset: &Asset,
        out: &mut File,
        report: &mut dyn FnMut(State),
    ) -> Result<()> {
        let mut response = self
            .agent
            .get(&asset.browser_download_url)
            .call()
            .map_err(|_| "Could not download Hex. Check your connection and try again.")?;
        let mut reader = response.body_mut().as_reader();
        let mut bytes = 0;
        let mut last_percent = u64::MAX;
        let mut buffer = [0; 128 * 1024];
        report(State::Downloading {
            bytes,
            total: asset.size,
        });
        loop {
            let count = reader
                .read(&mut buffer)
                .map_err(|_| "The Hex download stopped. Check your connection and try again.")?;
            if count == 0 {
                break;
            }
            bytes += count as u64;
            if bytes > asset.size {
                return Err("The Hex download is larger than expected. It was not run.");
            }
            out.write_all(&buffer[..count])
                .map_err(|_| "Could not save Hex. Check free disk space and try again.")?;
            let percent = bytes * 100 / asset.size;
            if percent != last_percent {
                report(State::Downloading {
                    bytes,
                    total: asset.size,
                });
                last_percent = percent;
            }
        }
        Ok(())
    }
    fn verify_signer(&mut self, path: &Path) -> Result<()> {
        let system = std::env::var_os("SystemRoot")
            .ok_or("Could not find Windows signature verification.")?;
        let mut command = crate::platform::windows_powershell_at(
            PathBuf::from(system).join("System32/WindowsPowerShell/v1.0/powershell.exe"),
        );
        command.args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command",
            "$ErrorActionPreference='Stop'; $s=Get-AuthenticodeSignature -LiteralPath $env:SIDENOTE_HEX_INSTALLER; if ($s.Status -eq 'Valid' -and $null -ne $s.SignerCertificate -and $s.SignerCertificate.GetNameInfo([System.Security.Cryptography.X509Certificates.X509NameType]::SimpleName,$false) -ceq 'Anomaly Innovations, Inc https://anoma.ly/') { exit 0 }; exit 1"])
            .env("SIDENOTE_HEX_INSTALLER", path);
        if wait_hidden(&mut command, Duration::from_secs(60))? != 0 {
            return Err(
                "Could not verify the Anomaly Innovations signature. The installer was not run. Use the download page.",
            );
        }
        Ok(())
    }
    fn install(&mut self, path: &Path) -> Result<()> {
        unelevated()?;
        let code = wait_hidden(
            Command::new(path).args([
                "/VERYSILENT",
                "/SUPPRESSMSGBOXES",
                "/NORESTART",
                "/SP-",
                "/CURRENTUSER",
            ]),
            Duration::from_secs(300),
        )?;
        if code != 0 {
            return Err("Hex setup did not complete. Try again or use the download page.");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const ABC_HASH: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn installed() -> Installation {
        Installation::Installed {
            version: "3.4.0".into(),
            exe: "mock/hex.exe".into(),
        }
    }

    #[test]
    fn detection_uses_current_pointer_and_never_directory_time() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert_eq!(detect_in(root), Installation::Missing);
        let current = root.join("versions/3.4.0");
        fs::create_dir_all(&current).unwrap();
        fs::write(current.join("hex.exe"), b"mock").unwrap();
        assert_eq!(detect_in(root), Installation::Incomplete);
        fs::write(root.join("current-version"), "3.4.0\n").unwrap();
        assert_eq!(
            detect_in(root),
            Installation::Installed {
                version: "3.4.0".into(),
                exe: current.join("hex.exe")
            }
        );
        fs::create_dir_all(root.join("versions/9.9.9")).unwrap();
        fs::write(root.join("versions/9.9.9/hex.exe"), b"mock").unwrap();
        assert_eq!(
            detect_in(root),
            Installation::Installed {
                version: "3.4.0".into(),
                exe: current.join("hex.exe")
            }
        );
        for pointer in [
            "../outside",
            "..",
            "C:\\Hex",
            "3.0.0",
            "",
            "3.4.0/../../escape",
        ] {
            fs::write(root.join("current-version"), pointer).unwrap();
            assert_eq!(detect_in(root), Installation::Incomplete, "{pointer}");
        }
    }

    #[test]
    fn legacy_and_unreadable_installations_are_not_missing() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("hex.exe"), b"mock").unwrap();
        assert_eq!(detect_in(dir.path()), Installation::Incomplete);
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("current-version")).unwrap();
        assert_eq!(detect_in(dir.path()), Installation::Incomplete);
    }

    #[test]
    fn sha256_accepts_only_matching_well_formed_digest() {
        for hash in [ABC_HASH.to_string(), ABC_HASH.to_uppercase()] {
            assert!(crate::updates::verify_sha256(&mut &b"abc"[..], &hash).unwrap());
        }
        for hash in ["", "abc", &"0".repeat(64), &"z".repeat(64)] {
            assert!(!crate::updates::verify_sha256(&mut &b"abc"[..], hash).unwrap());
        }
        assert!(!crate::updates::verify_sha256(&mut &b"abd"[..], ABC_HASH).unwrap());
    }

    fn release() -> Release {
        Release {
            tag_name: "v3.4.0".into(), draft: false, prerelease: false,
            assets: vec![Asset {
                name: "hex-setup-3.4.0-windows-x64.exe".into(),
                browser_download_url: "https://github.com/anomalyco/hex/releases/download/v3.4.0/hex-setup-3.4.0-windows-x64.exe".into(),
                size: 3, digest: Some(format!("sha256:{ABC_HASH}")),
            }],
        }
    }

    #[test]
    fn metadata_requires_stable_official_architecture_size_and_digest() {
        assert!(select_asset(release(), "x64").is_ok());
        assert!(select_asset(release(), "arm64").is_err());
        for change in 0..9 {
            let mut release = release();
            match change {
                0 => release.draft = true,
                1 => release.prerelease = true,
                2 => {
                    release.assets[0].browser_download_url = "https://example.com/setup.exe".into()
                }
                3 => release.assets[0].digest = None,
                4 => release.assets[0].digest = Some("sha256:bad".into()),
                5 => release.assets[0].size = MAX_INSTALLER + 1,
                6 => release.assets[0].size = 0,
                7 => release.assets[0].name = "hex-3.4.0-windows-x64.exe".into(),
                _ => release.tag_name = "../../other".into(),
            }
            assert!(select_asset(release, "x64").is_err(), "case {change}");
        }
    }

    struct Mock {
        fail: &'static str,
        calls: Vec<&'static str>,
        detects: Cell<u32>,
        installed: bool,
        path: Option<PathBuf>,
    }
    impl Mock {
        fn new(fail: &'static str) -> Self {
            Self {
                fail,
                calls: vec![],
                detects: Cell::new(0),
                installed: false,
                path: None,
            }
        }
    }
    impl Backend for Mock {
        fn detect(&self) -> Installation {
            let count = self.detects.get();
            self.detects.set(count + 1);
            if (self.installed && self.fail != "redetect")
                || self.fail == "existing"
                || (self.fail == "race" && count > 0)
            {
                installed()
            } else {
                Installation::Missing
            }
        }
        fn check_allowed(&self) -> Result<()> {
            if self.fail == "busy" {
                Err("mock busy or elevated")
            } else {
                Ok(())
            }
        }
        fn latest(&mut self) -> Result<Asset> {
            self.calls.push("metadata");
            if self.fail == "network" {
                return Err("mock offline");
            }
            select_asset(release(), "x64")
        }
        fn download(
            &mut self,
            _: &Asset,
            out: &mut File,
            report: &mut dyn FnMut(State),
        ) -> Result<()> {
            self.calls.push("download");
            report(State::Downloading { bytes: 0, total: 3 });
            let data: &[u8] = match self.fail {
                "hash" => b"abd",
                "truncated" => b"a",
                _ => b"abc",
            };
            out.write_all(data).unwrap();
            if self.fail == "download" {
                return Err("mock interrupted");
            }
            Ok(())
        }
        fn verify_signer(&mut self, path: &Path) -> Result<()> {
            self.calls.push("signature");
            self.path = Some(path.into());
            assert!(
                OpenOptions::new().write(true).open(path).is_err(),
                "verified file must stay pinned"
            );
            if self.fail == "signer" {
                Err("mock wrong publisher or invalid chain")
            } else {
                Ok(())
            }
        }
        fn install(&mut self, path: &Path) -> Result<()> {
            self.calls.push("install");
            assert_eq!(self.path.as_deref(), Some(path));
            assert!(
                fs::remove_file(path).is_err(),
                "installer must remain pinned through execution"
            );
            if self.fail == "install" {
                return Err("mock installer cancellation");
            }
            self.installed = true;
            Ok(())
        }
    }

    #[test]
    fn state_waits_for_click_and_rejects_duplicate_or_installed_clicks() {
        let mut mock = Mock::new("");
        let mut state = State::default();
        assert!(!state.busy());
        assert!(mock.calls.is_empty());
        assert!(!state.begin(&installed()));
        assert!(!state.begin(&Installation::Incomplete));
        assert!(state.begin(&Installation::Missing));
        assert!(!state.begin(&Installation::Missing));
        let mut states = vec![state];
        run(&mut mock, &mut |state| states.push(state)).unwrap();
        assert_eq!(
            states,
            [
                State::Checking,
                State::Downloading { bytes: 0, total: 3 },
                State::Verifying,
                State::Installing,
                State::Complete
            ]
        );
        assert_eq!(mock.calls, ["metadata", "download", "signature", "install"]);
        assert!(!mock.path.unwrap().exists());
    }

    #[test]
    fn failures_stop_before_installer_and_cleanup_verified_downloads() {
        for failure in [
            "existing",
            "busy",
            "network",
            "download",
            "truncated",
            "hash",
            "signer",
            "race",
            "install",
            "redetect",
        ] {
            let mut mock = Mock::new(failure);
            let mut states = vec![];
            let result = run(&mut mock, &mut |state| states.push(state));
            assert!(result.is_err(), "{failure}");
            assert!(matches!(states.last(), Some(State::Failed(_))), "{failure}");
            if !matches!(failure, "install" | "redetect") {
                assert!(!mock.calls.contains(&"install"), "{failure}");
            }
            if matches!(failure, "hash" | "truncated") {
                assert!(!mock.calls.contains(&"signature"), "{failure}");
            }
            if let Some(path) = mock.path {
                assert!(!path.exists(), "{failure}");
            }
            let mut state = State::Failed(result.unwrap_err());
            assert!(state.begin(&Installation::Missing));
        }
    }
}
