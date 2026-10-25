$ErrorActionPreference = "Stop"
Set-Location "$PSScriptRoot/../.."
if ($env:PROCESSOR_ARCHITECTURE -ne "AMD64") { throw "win-amd64 native runner required" }
$version = (Select-String -Path Cargo.toml -Pattern '^version = "([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$folder = "dist/SimpleSnip_${version}_win-amd64_unsigned"
New-Item -ItemType Directory -Force $folder | Out-Null
Copy-Item "target/x86_64-pc-windows-msvc/release/simple-snip.exe" $folder
Copy-Item README.md, LICENSE $folder
Copy-Item dist/THIRD_PARTY_NOTICES.txt $folder
Compress-Archive -Path "$folder/*" -DestinationPath "$folder.zip" -Force
# Unsigned local test package. Sign with an organization-owned certificate
# before any public release; never fabricate a signing identity.
