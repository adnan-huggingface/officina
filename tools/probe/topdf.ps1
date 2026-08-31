# Word's own rendering of a document, as a PDF.
#
# The first half of the oracle behind `cargo xtask compare`; `pdfwords.py`
# beside this reads the result. Nothing here writes to the document: it is
# opened read-only and closed without saving.
#
# **Why not read the positions over COM.** `Range.Information(5|6)` answers to
# a twentieth of a point and is what `wordmap.ps1` uses, but each call costs
# Word a layout pass — measured at about 110ms on this machine, for every word.
# A sixteen-page document is some five thousand words, which is hours. One
# export is seconds and is exact, so the whole document goes through paper.
#
# Exporting needs a licensed Word, unlike the COM reads the other probes here
# do; an unlicensed install will refuse this.

param(
  [Parameter(Mandatory = $true)][string]$Path,
  [Parameter(Mandatory = $true)][string]$Out
)
$ErrorActionPreference = "Stop"

$word = New-Object -ComObject Word.Application
$word.Visible = $false
$document = $null
try {
  $full = (Resolve-Path -LiteralPath $Path).Path
  # ConfirmConversions off, ReadOnly on: a .doc must not be silently upgraded
  # and the file on disk must come back untouched.
  $document = $word.Documents.Open($full, $false, $true)
  # wdExportFormatPDF = 17, wdExportOptimizeForPrint = 0, wdExportAllDocument = 0.
  $document.ExportAsFixedFormat($Out, 17, $false, 0, 0)
} finally {
  if ($null -ne $document) { $document.Close($false) }
  $word.Quit()
  [void][System.Runtime.InteropServices.Marshal]::ReleaseComObject($word)
}
