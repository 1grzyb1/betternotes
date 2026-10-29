# Installs the latest BetterNotes release on Windows.
#   irm https://raw.githubusercontent.com/1grzyb1/betternotes/main/install.ps1 | iex
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$url = 'https://github.com/1grzyb1/betternotes/releases/latest/download/BetterNotes-windows-x86_64.zip'
$dest = Join-Path $env:LOCALAPPDATA 'Programs\BetterNotes'
$zip = Join-Path $env:TEMP 'BetterNotes.zip'

Write-Host 'Downloading BetterNotes...'
Invoke-WebRequest $url -OutFile $zip

if (Get-Process BetterNotes -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$dest*" }) {
    throw 'BetterNotes is running. Close it and run the installer again.'
}
New-Item -ItemType Directory -Force $dest | Out-Null
Expand-Archive $zip -DestinationPath $dest -Force
Remove-Item $zip

$exe = Join-Path $dest 'BetterNotes.exe'
$shell = New-Object -ComObject WScript.Shell
$startMenu = Join-Path ([Environment]::GetFolderPath('Programs')) 'BetterNotes.lnk'
$lnk = $shell.CreateShortcut($startMenu)
$lnk.TargetPath = $exe
$lnk.WorkingDirectory = $dest
$lnk.Save()

Write-Host "Installed to $exe"
Write-Host 'BetterNotes is now in the Start menu.'
