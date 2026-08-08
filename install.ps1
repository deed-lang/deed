# Install the `deed` binary on Windows.
#
#   irm https://raw.githubusercontent.com/deed-lang/deed/main/install.ps1 | iex
#
# The same three steps as install.sh: work out which release asset fits this
# machine, refuse it if its hash is not the one the release published, and put
# one file in your own profile. It never needs elevation, because it never
# writes outside your profile.
#
# What the hash does and does not buy: the checksums come from the same release
# as the binary, so this catches a truncated or corrupted download and does not
# catch a compromised release.
#
# `DEED_VERSION` pins a release, `DEED_INSTALL_DIR` says where the file goes,
# and `DEED_DOWNLOAD_BASE` points at a mirror of the release assets.
#
# `crates/deed-driver/tests/install.rs` holds the platform this knows against
# the ones `.github/workflows/release.yml` actually builds.

$ErrorActionPreference = 'Stop'

$repo = 'deed-lang/deed'
$installDir = if ($env:DEED_INSTALL_DIR) {
    $env:DEED_INSTALL_DIR
} else {
    Join-Path $env:LOCALAPPDATA 'Programs\deed'
}

# There is one Windows build, and it is x64. Saying which machine this is
# beats installing something that will not start.
$arch = $env:PROCESSOR_ARCHITECTURE
if ($arch -ne 'AMD64') {
    throw "no release is built for Windows on $arch: ``cargo install --path crates/deed-cli`` from a clone builds one"
}
$target = 'x86_64-pc-windows-msvc'

$version = if ($env:DEED_VERSION) { $env:DEED_VERSION } else { $null }
if (-not $version) {
    $latest = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest"
    $version = $latest.tag_name
}
if (-not $version) { throw "could not work out the latest release of $repo" }
if (-not $version.StartsWith('v')) { $version = "v$version" }

$name = "deed-$version-$target"
$asset = "$name.zip"
$base = if ($env:DEED_DOWNLOAD_BASE) {
    $env:DEED_DOWNLOAD_BASE
} else {
    "https://github.com/$repo/releases/download/$version"
}

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("deed-install-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    Write-Host "downloading $asset"
    $archive = Join-Path $tmp $asset
    Invoke-WebRequest "$base/$asset" -OutFile $archive
    $sums = Join-Path $tmp 'checksums.txt'
    Invoke-WebRequest "$base/deed-$version-checksums.txt" -OutFile $sums

    # The last field is the name and the first is the hash, read by field
    # rather than by exact spacing.
    $want = $null
    foreach ($line in Get-Content $sums) {
        $fields = $line.Trim() -split '\s+'
        if ($fields.Length -lt 2) { continue }
        $file = $fields[-1] -replace '^\*', '' -replace '^\./', ''
        if ($file -eq $asset) { $want = $fields[0]; break }
    }
    if (-not $want) { throw "the checksum list does not mention $asset" }
    $got = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLower()
    if ($want.ToLower() -ne $got) {
        throw "$asset hashes to $got and the release says $want"
    }
    Write-Host 'sha256 ok'

    Expand-Archive -Path $archive -DestinationPath $tmp -Force
    $binary = Join-Path $tmp "$name\deed.exe"
    if (-not (Test-Path $binary)) { throw "$asset does not contain $name\deed.exe" }

    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
    Move-Item -Path $binary -Destination (Join-Path $installDir 'deed.exe') -Force
    Write-Host "installed $installDir\deed.exe"
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}

# The user's own PATH, not the machine's: this wrote one file into one profile
# and changing the machine would be a larger claim than the install made.
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';') -notcontains $installDir) {
    [Environment]::SetEnvironmentVariable('Path', "$userPath;$installDir", 'User')
    Write-Host ''
    Write-Host "added $installDir to your PATH; open a new terminal to pick it up"
}

Write-Host ''
Write-Host 'next: deed new greeter; cd greeter; deed test .'
