param([string]$Dir)
$ErrorActionPreference = "Stop"
$word = New-Object -ComObject Word.Application
$word.Visible = $false
try {
  foreach ($file in Get-ChildItem $Dir -Filter *.docx) {
    $document = $word.Documents.Open($file.FullName, $false, $true)
    $lines = @()
    foreach ($p in $document.Paragraphs) {
      $r = $p.Range.Duplicate
      $r.Collapse(1)
      $lines += "{0:0.000},{1},{2},{3},{4}" -f [double]$r.Information(6), `
        [int]$p.Range.Information(12), $p.Range.Font.Name, $p.Range.Font.Size, $r.Information(3)
    }
    $out = [System.IO.Path]::ChangeExtension($file.FullName, ".csv")
    $lines | Set-Content -Encoding ascii $out
    Write-Output "$($file.Name): $($lines.Count) paragraphs"
    $document.Close($false)
  }
} finally {
  $word.Quit()
  [void][System.Runtime.InteropServices.Marshal]::ReleaseComObject($word)
}
