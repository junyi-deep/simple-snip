//! Prepare beside the installed application, then replace only after it exits.
//! Failed replacement restores the backup. Download verification precedes this.
use anyhow::{Context, Result, ensure};
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

pub struct InstallPlan {
    root: PathBuf,
    target: PathBuf,
    payload: PathBuf,
    result: PathBuf,
    #[cfg(target_os = "macos")]
    reset_screen_capture: bool,
}

#[cfg(target_os = "macos")]
const MACOS_INSTALLER: &str = include_str!("install-macos.sh");

#[cfg(target_os = "macos")]
fn screen_capture_identity_changed(previous: &Path, incoming: &Path) -> bool {
    let Ok(requirement) = command("/usr/bin/codesign")
        .args(["--display", "-r-"])
        .arg(previous)
        .output()
    else {
        return false;
    };
    let Ok(requirement_text) = std::str::from_utf8(&requirement.stdout) else {
        return false;
    };
    let requirement_text = requirement_text.trim();
    let requirement_text = requirement_text
        .strip_prefix("# ")
        .unwrap_or(requirement_text);
    let Some(requirement_text) = requirement_text.strip_prefix("designated => ") else {
        return false;
    };
    if !requirement.status.success() || requirement_text.is_empty() {
        return false;
    }
    // Exit 3 means that valid code did not satisfy the explicit requirement.
    // Treat tool and signature validation failures as inconclusive.
    command("/usr/bin/codesign")
        .arg("--verify")
        .arg(format!("-R={requirement_text}"))
        .arg(incoming)
        .output()
        .is_ok_and(|result| result.status.code() == Some(3))
}

fn command(program: &str) -> Command {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = Command::new(program);
        cmd.creation_flags(0x08000000);
        cmd
    }
    #[cfg(target_os = "macos")]
    {
        Command::new(program)
    }
}
fn checked(cmd: &mut Command) -> Result<()> {
    let output = cmd.output().context("无法启动升级工具")?;
    ensure!(
        output.status.success(),
        "升级准备失败：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
#[cfg(any(target_os = "windows", test))]
fn quote_ps(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub fn prepare_install(package: &Path) -> Result<InstallPlan> {
    let executable = std::env::current_exe()?.canonicalize()?;
    #[cfg(target_os = "windows")]
    let target = executable;
    #[cfg(target_os = "macos")]
    let target = executable
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .context("自动升级需要从 .app 应用包运行")?
        .to_path_buf();
    let result = snip_core::config::Config::path()
        .with_file_name("updates")
        .join("last-install.txt");
    prepare_install_at(package, target, result)
}

fn prepare_install_at(package: &Path, target: PathBuf, result: PathBuf) -> Result<InstallPlan> {
    let parent = target.parent().context("安装目录无效")?;
    let staging = tempfile::Builder::new()
        .prefix(".simple-snip-update-")
        .tempdir_in(parent)
        .context("安装目录不可写，请将应用放到可写目录后升级")?;
    let root = staging.path().to_path_buf();
    std::fs::create_dir_all(result.parent().unwrap())?;
    #[cfg(target_os = "windows")]
    let payload = {
        ensure!(
            package.extension().is_some_and(|e| e == "zip"),
            "升级包必须为 ZIP"
        );
        let payload = root.join("simple-snip.exe");
        let script = format!(
            r#"& {{ param($Archive,$Output)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.IO.Compression,System.IO.Compression.FileSystem
$zip=[IO.Compression.ZipFile]::OpenRead($Archive)
try {{
 $entries=@($zip.Entries | Where-Object {{ $_.FullName -eq 'simple-snip.exe' -or $_.FullName.EndsWith('/simple-snip.exe') }})
 if ($entries.Count -ne 1 -or $entries[0].Length -gt 268435456) {{ throw 'Invalid update executable' }}
 $inputStream=$entries[0].Open()
 $outputStream=[IO.File]::Open($Output,[IO.FileMode]::CreateNew)
 try {{ $inputStream.CopyTo($outputStream) }} finally {{ $inputStream.Dispose(); $outputStream.Dispose() }}
}} finally {{ $zip.Dispose() }}
}} {} {}"#,
            quote_ps(&package.to_string_lossy()),
            quote_ps(&payload.to_string_lossy())
        );
        checked(command("powershell.exe").args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ]))?;
        let bytes = std::fs::read(&payload)?;
        ensure!(
            valid_pe(&bytes),
            "更新程序不是有效的 Windows AMD64 可执行文件"
        );
        payload
    };
    #[cfg(target_os = "macos")]
    let payload = {
        let mount = root.join("mount");
        std::fs::create_dir(&mount)?;
        checked(
            command("hdiutil")
                .args(["attach", "-readonly", "-nobrowse", "-mountpoint"])
                .arg(&mount)
                .arg(package),
        )?;
        let prepared = (|| -> Result<PathBuf> {
            let apps: Vec<_> = std::fs::read_dir(&mount)?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "app"))
                .collect();
            ensure!(apps.len() == 1, "升级镜像必须包含一个应用包");
            let payload = root.join("Simple Snip.app");
            checked(command("ditto").arg(&apps[0]).arg(&payload))?;
            checked(
                command("/usr/bin/codesign")
                    .args([
                        "--verify",
                        "--deep",
                        "--strict",
                        "-R=identifier \"dev.simplesnip.desktop\"",
                    ])
                    .arg(&payload),
            )?;
            let bytes = std::fs::read(payload.join("Contents/MacOS/simple-snip"))?;
            ensure!(
                bytes.starts_with(&[0xcf, 0xfa, 0xed, 0xfe])
                    && bytes.get(4..8) == Some(&[12, 0, 0, 1]),
                "更新程序不是 macOS ARM64 应用"
            );
            Ok(payload)
        })();
        let _ = command("hdiutil").arg("detach").arg(&mount).output();
        prepared?
    };
    #[cfg(target_os = "macos")]
    let reset_screen_capture = screen_capture_identity_changed(&target, &payload);
    let root = staging.keep();
    Ok(InstallPlan {
        root,
        target,
        payload,
        result,
        #[cfg(target_os = "macos")]
        reset_screen_capture,
    })
}

#[cfg(any(target_os = "windows", test))]
fn valid_pe(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"MZ") {
        return false;
    }
    let Some(offset) = bytes
        .get(0x3c..0x40)
        .and_then(|b| <[u8; 4]>::try_from(b).ok())
        .map(u32::from_le_bytes)
    else {
        return false;
    };
    bytes.get(offset as usize..offset as usize + 6) == Some(&[b'P', b'E', 0, 0, 0x64, 0x86])
}

pub fn launch_install(plan: InstallPlan) -> Result<()> {
    let ready = plan.root.join("ready");
    let backup = plan.root.join(if cfg!(target_os = "windows") {
        "previous.exe"
    } else {
        "Previous.app"
    });
    let log = std::fs::File::create(plan.root.join("install.log"))?;
    #[cfg(target_os = "windows")]
    let mut helper = {
        let script = format!(
            r#"& {{ param($ParentId,$Target,$Payload,$Backup,$Ready,$ResultFile)
$ErrorActionPreference='Stop'
[IO.File]::WriteAllText($Ready,'ready')
try {{
 $process=Get-Process -Id $ParentId -ErrorAction SilentlyContinue
 if ($process) {{ Wait-Process -Id $ParentId -Timeout 60 -ErrorAction Stop }}
 $moved=$false
 for ($attempt=0; $attempt -lt 50; $attempt++) {{
  try {{ Move-Item -LiteralPath $Target -Destination $Backup -ErrorAction Stop; $moved=$true; break }} catch {{ Start-Sleep -Milliseconds 200 }}
 }}
 if (-not $moved) {{ throw 'Cannot replace installed application' }}
 Move-Item -LiteralPath $Payload -Destination $Target -ErrorAction Stop
 Start-Process -FilePath $Target -WindowStyle Hidden -ErrorAction Stop
 [IO.File]::WriteAllText($ResultFile,'ok')
}} catch {{
 $_ | Out-String | Write-Output
 if (Test-Path -LiteralPath $Backup) {{
  if (Test-Path -LiteralPath $Target) {{ Move-Item -LiteralPath $Target -Destination ($Payload+'.failed') -ErrorAction SilentlyContinue }}
  Move-Item -LiteralPath $Backup -Destination $Target -ErrorAction SilentlyContinue
 }}
 [IO.File]::WriteAllText($ResultFile,'failed')
 if (Test-Path -LiteralPath $Target) {{ Start-Process -FilePath $Target -WindowStyle Hidden }}
}}
}} {} {} {} {} {} {}"#,
            std::process::id(),
            quote_ps(&plan.target.to_string_lossy()),
            quote_ps(&plan.payload.to_string_lossy()),
            quote_ps(&backup.to_string_lossy()),
            quote_ps(&ready.to_string_lossy()),
            quote_ps(&plan.result.to_string_lossy())
        );
        command("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()?
    };
    #[cfg(target_os = "macos")]
    let mut helper = command("/bin/sh")
        .args(["-c", MACOS_INSTALLER, "simple-snip-updater"])
        .arg(std::process::id().to_string())
        .arg(&plan.target)
        .arg(&plan.payload)
        .arg(&backup)
        .arg(&ready)
        .arg(&plan.result)
        .arg(if plan.reset_screen_capture {
            "yes"
        } else {
            "no"
        })
        .args(["/usr/bin/tccutil", "/usr/bin/open"])
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        ensure!(
            helper.try_wait()?.is_none(),
            "升级助手启动失败，当前应用未退出；请查看安装日志"
        );
        ensure!(Instant::now() < deadline, "升级助手未就绪，当前应用未退出");
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_upgrade_checks_the_previous_signing_requirement() {
        use super::*;

        let temp = tempfile::tempdir().unwrap();
        let sign = |version| {
            let app = temp.path().join(format!("Signed {version}.app"));
            std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
            std::fs::copy("/usr/bin/true", app.join("Contents/MacOS/test")).unwrap();
            std::fs::write(
                app.join("Contents/Info.plist"),
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>test</string>
<key>CFBundleIdentifier</key><string>dev.simplesnip.signature-test</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>{version}</string>
</dict></plist>"#
                ),
            )
            .unwrap();
            checked(
                command("/usr/bin/codesign")
                    .args(["--force", "--sign", "-"])
                    .arg(&app),
            )
            .unwrap();
            app
        };
        let previous = sign(1);
        let incoming = sign(2);
        assert!(screen_capture_identity_changed(&previous, &incoming));
        assert!(!screen_capture_identity_changed(&previous, &previous));
        assert!(!screen_capture_identity_changed(
            &temp.path().join("Missing.app"),
            &incoming,
        ));
        // System apps emit the explicit designated requirement without the
        // comment marker that appears in ad-hoc signatures.
        let text_edit = Path::new("/System/Applications/TextEdit.app");
        let calculator = Path::new("/System/Applications/Calculator.app");
        assert!(!screen_capture_identity_changed(text_edit, text_edit));
        assert!(screen_capture_identity_changed(text_edit, calculator));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_install_resets_only_after_replacement_and_reports_failure_without_rollback() {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        for (reset, reset_exit, has_payload, expected, launch_arg) in [
            ("no", "0", true, "ok", ""),
            (
                "yes",
                "0",
                true,
                "ok-screen-capture-reset",
                "--screen-capture-reset=done",
            ),
            (
                "yes",
                "1",
                true,
                "ok-screen-capture-reset-failed",
                "--screen-capture-reset=failed",
            ),
            ("yes", "0", false, "failed", ""),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let path = |name| temp.path().join(name);
            let target = path("Installed App");
            let payload = path("Incoming App");
            let result = path("result");
            let log = path("commands");
            std::fs::write(&target, "old").unwrap();
            if has_payload {
                std::fs::write(&payload, "new").unwrap();
            }
            let tool = |name, script| {
                let file = path(name);
                std::fs::write(&file, script).unwrap();
                std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)).unwrap();
                file
            };
            let tccutil = tool(
                "fake-tccutil",
                r#"#!/bin/sh
[ "$(cat "$TEST_TARGET")" = new ] || exit 2
[ "$*" = 'reset ScreenCapture dev.simplesnip.desktop' ] || exit 3
printf 'reset\n' >> "$TEST_LOG"
exit "$TEST_RESET_EXIT"
"#,
            );
            let open = tool(
                "fake-open",
                r#"#!/bin/sh
[ "$1" = "$TEST_TARGET" ] || exit 2
printf 'open %s %s\n' "$(cat "$1")" "$3" >> "$TEST_LOG"
"#,
            );
            let mut stopped = command("/usr/bin/true").spawn().unwrap();
            let previous_pid = stopped.id();
            assert!(stopped.wait().unwrap().success());
            let output = command("/bin/sh")
                .args(["-c", MACOS_INSTALLER, "updater"])
                .arg(previous_pid.to_string())
                .arg(&target)
                .arg(&payload)
                .arg(path("Backup"))
                .arg(path("ready"))
                .arg(&result)
                .arg(reset)
                .arg(tccutil)
                .arg(open)
                .env("TEST_TARGET", &target)
                .env("TEST_LOG", &log)
                .env("TEST_RESET_EXIT", reset_exit)
                .output()
                .unwrap();
            assert!(output.status.success(), "{:?}", output);
            assert_eq!(std::fs::read_to_string(result).unwrap(), expected);
            let expected_reset = if reset == "yes" && has_payload {
                "reset\n"
            } else {
                ""
            };
            let version = if has_payload { "new" } else { "old" };
            assert_eq!(
                std::fs::read_to_string(log).unwrap(),
                format!("{expected_reset}open {version} {launch_arg}\n"),
            );
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn prepares_only_payload_and_rejects_ambiguous_archive_without_replacing_installation() {
        use super::*;
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("with ' quote");
        std::fs::create_dir(&dir).unwrap();
        let target = dir.join("installed.exe");
        std::fs::write(&target, b"unchanged installed executable").unwrap();
        let source = dir.join("source.exe");
        let mut bytes = vec![0u8; 128];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c] = 64;
        bytes[64..70].copy_from_slice(&[b'P', b'E', 0, 0, 0x64, 0x86]);
        std::fs::write(&source, &bytes).unwrap();
        for duplicate in [false, true] {
            let archive = dir.join(if duplicate {
                "ambiguous.zip"
            } else {
                "valid.zip"
            });
            let script = format!(
                "Add-Type -AssemblyName System.IO.Compression,System.IO.Compression.FileSystem; $z=[IO.Compression.ZipFile]::Open({},[IO.Compression.ZipArchiveMode]::Create); try {{ [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($z,{},'app/simple-snip.exe') | Out-Null; {} }} finally {{ $z.Dispose() }}",
                quote_ps(&archive.to_string_lossy()),
                quote_ps(&source.to_string_lossy()),
                if duplicate {
                    format!(
                        "[IO.Compression.ZipFileExtensions]::CreateEntryFromFile($z,{},'other/simple-snip.exe') | Out-Null",
                        quote_ps(&source.to_string_lossy())
                    )
                } else {
                    String::new()
                }
            );
            checked(command("powershell.exe").args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &script,
            ]))
            .unwrap();
            let result = prepare_install_at(&archive, target.clone(), dir.join("state/result.txt"));
            assert_eq!(
                std::fs::read(&target).unwrap(),
                b"unchanged installed executable"
            );
            if duplicate {
                assert!(result.is_err());
            } else {
                let plan = result.unwrap();
                assert_eq!(std::fs::read(&plan.payload).unwrap(), bytes);
                assert!(!plan.root.join("other").exists());
            }
        }
    }

    #[test]
    fn validates_executable_and_quotes_installer_arguments() {
        assert!(!super::valid_pe(b"MZ"));
        let mut bytes = vec![0; 128];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c] = 64;
        bytes[64..70].copy_from_slice(&[b'P', b'E', 0, 0, 0x64, 0x86]);
        assert!(super::valid_pe(&bytes));
        bytes[69] = 0;
        assert!(!super::valid_pe(&bytes));
        assert_eq!(super::quote_ps("C:/a'b/$file.exe"), "'C:/a''b/$file.exe'");
    }
}
