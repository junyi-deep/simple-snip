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
                command("codesign")
                    .args(["--verify", "--deep", "--strict"])
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
    let root = staging.keep();
    Ok(InstallPlan {
        root,
        target,
        payload,
        result,
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
    let mut helper = command("/bin/sh").args(["-c", r#"
pid=$1; target=$2; payload=$3; backup=$4; ready=$5; result=$6
printf ready > "$ready"
count=0
while kill -0 "$pid" 2>/dev/null; do count=$((count+1)); [ "$count" -lt 60 ] || exit 1; sleep 1; done
if mv "$target" "$backup" && mv "$payload" "$target"; then
 printf ok > "$result"
 open "$target" || { mv "$target" "$payload.failed"; mv "$backup" "$target"; printf failed > "$result"; open "$target"; }
else
 [ ! -e "$backup" ] || mv "$backup" "$target"
 printf failed > "$result"
 open "$target"
fi
"#, "simple-snip-updater"]).arg(std::process::id().to_string()).arg(&plan.target).arg(&plan.payload).arg(&backup).arg(&ready).arg(&plan.result).stdout(log.try_clone()?).stderr(log).spawn()?;
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
