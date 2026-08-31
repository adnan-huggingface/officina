# LibreOffice's own rendering of a document, as a PDF.
#
# The ODF half of the oracle behind `cargo xtask compare`; `pdfink.py` beside
# this reads the result and neither knows nor cares which application drew it.
# Nothing here writes to the document: LibreOffice converts a copy and the
# original is only read.
#
# **Why LibreOffice and not Word.** Word opens `.odt` through a converter it
# wrote for a format it does not own, and this machine's Word has already been
# caught rendering an embedded face wrongly. LibreOffice is the implementation
# ODF is defined against in practice, and it writes the current version of the
# standard. It is used here exactly as Word is: a black box that renders, asked
# a question and never read.
#
# **Why a private user profile.** Without `-env:UserInstallation` the run joins
# whatever LibreOffice the user already has open — which makes it fail outright
# if one is running, and inherit that install's accumulated settings if one is
# not. A profile of our own under `target/` makes the reading reproducible and
# leaves the user's alone.

param(
  [Parameter(Mandatory = $true)][string]$Path,
  [Parameter(Mandatory = $true)][string]$Out
)
$ErrorActionPreference = "Stop"

function Find-Soffice {
  $onPath = Get-Command soffice.exe -ErrorAction SilentlyContinue
  if ($onPath) { return $onPath.Source }
  foreach ($base in @($env:ProgramFiles, ${env:ProgramFiles(x86)})) {
    if (-not $base) { continue }
    $candidate = Join-Path $base "LibreOffice\program\soffice.exe"
    if (Test-Path -LiteralPath $candidate) { return $candidate }
  }
  return $null
}

$soffice = Find-Soffice
if (-not $soffice) {
  Write-Error "LibreOffice is not installed: soffice.exe is not on PATH and not under Program Files."
  exit 1
}

$full = (Resolve-Path -LiteralPath $Path).Path
$outDir = Split-Path -Parent ([System.IO.Path]::GetFullPath($Out))
if (-not (Test-Path -LiteralPath $outDir)) { New-Item -ItemType Directory -Path $outDir | Out-Null }

# A directory of our own for the conversion, because `--convert-to` names its
# own output after the input and would otherwise collide with a second document
# of the same stem.
$work = Join-Path $outDir ("convert-" + [System.IO.Path]::GetFileNameWithoutExtension($Out))
if (Test-Path -LiteralPath $work) { Remove-Item -LiteralPath $work -Recurse -Force }
New-Item -ItemType Directory -Path $work | Out-Null

$profileDir = Join-Path $outDir "soffice-profile"
$profileUrl = "file:///" + $profileDir.Replace('\', '/')

try {
  & $soffice --headless --norestore --nolockcheck --nodefault --nologo `
    "-env:UserInstallation=$profileUrl" `
    --convert-to "pdf:writer_pdf_Export" --outdir $work $full | Out-Null
  if ($LASTEXITCODE -ne 0) {
    Write-Error "LibreOffice exited $LASTEXITCODE converting $full"
    exit $LASTEXITCODE
  }

  $made = Get-ChildItem -LiteralPath $work -Filter *.pdf -File
  if ($made.Count -ne 1) {
    Write-Error "expected one PDF from the conversion, got $($made.Count)"
    exit 1
  }
  Move-Item -LiteralPath $made[0].FullName -Destination $Out -Force
} finally {
  if (Test-Path -LiteralPath $work) { Remove-Item -LiteralPath $work -Recurse -Force }
}
